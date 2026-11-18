//! The editor's import engine: source files into persistent UUID-addressed content assets.
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use concerto_asset_import::{
    DependencyEntry, EmittedUncooked, ErasedImporter, ImportContext, PreviousOutputResolver,
    SubAssetIdResolver,
};
use concerto_foundation::assets::content::{
    asset_record, read_asset_record_without_uncooked, read_content_asset_header,
    write_content_asset, write_content_asset_with_embedded, write_content_asset_with_uncooked,
    ContentAssetHeader, ContentFile, EmbeddedPayload, ImportProvenance, ImportRecord, SourceFile,
    UncookedSection,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry, EmbeddedIn};
use concerto_foundation::assets::AssetId;
use concerto_foundation::hash::Fnv1a;
use concerto_reflect::Reflect;

pub mod config;

use config::{content_address, ContentConfig};

static IMPORTERS: [&dyn ErasedImporter; 5] = [
    &concerto_render::importers::image_importer::ImageImporter,
    &concerto_gltf::gltf_importer::GltfImporter,
    &concerto_obj::obj_importer::ObjImporter,
    &concerto_bvh::bvh_importer::BvhImporter,
    &concerto_audio::AudioImporter,
];

/// Every importer the import library runs.
pub fn importers() -> &'static [&'static dyn ErasedImporter] {
    &IMPORTERS
}

/// The importer for files with `extension`, given without a dot and in any case.
pub fn importer_for(extension: &str) -> Option<&'static dyn ErasedImporter> {
    let extension = extension.to_ascii_lowercase();
    importers().iter().copied().find(|importer| {
        importer
            .supported_extensions()
            .contains(&extension.as_str())
    })
}

/// The settings given to one import; each one left `None` keeps what the project holds.
#[derive(Default)]
pub struct ImportSettings {
    /// The importer's settings, written whole into the anchor's import record; `None` imports with the record's.
    pub settings: Option<Box<dyn Reflect>>,
    /// The settings of the source's main asset; `None` keeps the asset's own, or gives a new one the defaults.
    pub asset_settings: Option<Box<dyn Reflect>>,
}

/// The settings an import of a source would use if given none.
pub struct StagedSettings {
    /// The importer settings of the anchor's import record, or the defaults.
    pub settings: Box<dyn Reflect>,
    /// The main asset's current settings, or the defaults when it will be new; `None` when the importer has none.
    pub asset_settings: Option<Box<dyn Reflect>>,
}

/// A project's content as one scan found it, to stage any number of sources against.
pub struct ContentScan {
    project_root: PathBuf,
    registry: AssetRegistry,
}

impl ContentScan {
    /// Scans the content tree of the existing project at `project_root`; fails on a malformed header or a duplicate id.
    pub fn new(project_root: &Path, config: &ContentConfig) -> anyhow::Result<Self> {
        config.validate()?;
        let project_root = project_root
            .canonicalize()
            .with_context(|| format!("resolving project '{}'", project_root.display()))?;
        let registry =
            AssetRegistry::from_content_tree(&project_root, &config.root, &config.extension)?;
        Ok(Self {
            project_root,
            registry,
        })
    }

    /// The settings importing `source` as `destination` would use if given none: the anchor's import record's, or the defaults, and the main asset's current ones; fails when the record cannot be used.
    pub fn staged_settings(
        &self,
        source: &Path,
        destination: &str,
    ) -> anyhow::Result<StagedSettings> {
        let placement = placement(source, destination, &self.project_root)?;
        let importer = importer_of(&placement.source, &placement.placed)?;
        let provenance_source = provenance_source(&placement.placed, &placement.project_root);
        let found = found(&self.registry, &provenance_source, &placement.project_root)?;
        let settings = settings_for(importer, None, &found, &provenance_source)?;
        let ctx = ImportContext::new(placement.placed.clone())
            .with_previous_outputs(previous_outputs(&found, &placement.project_root));
        let asset_settings = importer
            .current_asset_settings(&ctx)
            .or_else(|| importer.default_asset_settings());
        Ok(StagedSettings {
            settings,
            asset_settings,
        })
    }
}

const HOST: &str = "scene";
const MAIN: &str = "main";

/// Whether an output of `kind` is stored inside its import's scene file by default.
pub fn embeds_by_default(kind: &str) -> bool {
    matches!(kind, "Mesh" | "StandardMaterial" | "Skeleton")
}

/// What [`import_source_into`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSource {
    /// Every content asset written, with its import provenance; one stored in the scene file has `embedded_in`.
    pub assets: Vec<AssetEntry>,
    /// Project-relative paths of the files copied along with the source.
    pub siblings: Vec<String>,
    /// Ids of the content assets deleted because the importer stopped emitting them.
    pub removed: Vec<AssetId>,
}

struct PlannedFile {
    path: PathBuf,
    bytes: Vec<u8>,
}

struct EmbeddedOutput<'a> {
    header: ContentAssetHeader,
    payload: &'a [u8],
    uncooked: Option<&'a [u8]>,
}

struct HostOutput<'a> {
    path: PathBuf,
    header: ContentAssetHeader,
    payload: &'a [u8],
}

fn described(section: &EmittedUncooked) -> UncookedSection {
    UncookedSection {
        len: section.bytes.len() as u64,
        version: section.version,
    }
}

struct HostFile {
    id: AssetId,
    address: String,
}

struct Pruned {
    key: String,
    id: AssetId,
    file: Option<PathBuf>,
}

fn loose_address(
    registry: &AssetRegistry,
    config: &ContentConfig,
    relative_source: &Path,
    id: AssetId,
    name: &str,
) -> String {
    registry
        .get(id)
        .filter(|entry| entry.embedded_in.is_none())
        .map(|entry| entry.address.clone())
        .unwrap_or_else(|| content_address(config, relative_source, name))
}

/// Imports `source` into the project as `destination` with its referenced files, writing given `settings` into the anchor's import record and keeping each output where the content holds it.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
    config: &ContentConfig,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    config.validate()?;
    std::fs::create_dir_all(project_root)?;
    let placement = placement(source, destination, project_root)?;
    import_placed(
        &placement.source,
        &placement.placed,
        &placement.project_root,
        config,
        settings,
    )
}

struct Placement {
    source: PathBuf,
    placed: PathBuf,
    project_root: PathBuf,
}

fn placement(source: &Path, destination: &str, project_root: &Path) -> anyhow::Result<Placement> {
    let source = source
        .canonicalize()
        .with_context(|| format!("resolving source '{}'", source.display()))?;
    let relative = Path::new(destination);
    if destination.is_empty()
        || destination.ends_with(['/', '\\'])
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("destination '{destination}' must be a file inside the project, without '.' or '..'");
    }
    let extension = extension_of(&source);
    if extension_of(relative) != extension {
        bail!("destination '{destination}' must keep the source's extension '.{extension}'");
    }
    let project_root = project_root.canonicalize()?;
    let placed = placed_path(&project_root.join(relative));
    if !placed.starts_with(&project_root) {
        bail!("destination '{destination}' resolves outside the project");
    }
    Ok(Placement {
        source,
        placed,
        project_root,
    })
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn placed_path(target: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut existing = target;
    loop {
        if let Ok(resolved) = existing.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(resolved, |path, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => return target.to_path_buf(),
        }
    }
}

fn importer_of(read: &Path, source: &Path) -> anyhow::Result<&'static dyn ErasedImporter> {
    let extension = extension_of(read);
    importer_for(&extension).with_context(|| {
        format!(
            "no importer handles '.{extension}' (source '{}')",
            source.display()
        )
    })
}

fn import_placed(
    read: &Path,
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    let importer = importer_of(read, source)?;
    let owner = provenance_source(source, project_root);
    import_with(
        importer,
        read,
        source,
        &owner,
        project_root,
        config,
        settings,
    )
}

/// Imports again the source of the import that `asset` belongs to: from the anchor's recorded path, or from `source` when given, which becomes the import's source.
pub fn reimport(
    asset: AssetId,
    source: Option<&Path>,
    project_root: &Path,
    config: &ContentConfig,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    config.validate()?;
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("resolving project '{}'", project_root.display()))?;
    let registry =
        AssetRegistry::from_content_tree(&project_root, &config.root, &config.extension)?;
    let entry = registry
        .get(asset)
        .with_context(|| format!("no asset {} in the project", asset.simple_hex()))?;
    let Some(provenance) = &entry.provenance else {
        bail!("'{}' was not imported; it has no source", entry.location());
    };
    let recorded = provenance.source.clone();
    let recorded_path = project_root.join(&recorded);
    let (read, placed) = match source {
        None => {
            if !recorded_path.is_file() {
                let outputs = found(&registry, &recorded, &project_root)?.outputs;
                let anchor = anchor_of(&outputs).unwrap_or(entry).location();
                bail!("'{recorded}', the source of '{anchor}', is missing; re-import it with a source");
            }
            (recorded_path.clone(), recorded_path)
        }
        Some(source) => {
            let read = source
                .canonicalize()
                .with_context(|| format!("resolving source '{}'", source.display()))?;
            let extension = extension_of(&recorded_path);
            if extension_of(&read) != extension {
                bail!(
                    "'{}' must have the extension '.{extension}' of '{recorded}'",
                    read.display()
                );
            }
            let placed = if read.starts_with(&project_root) {
                read.clone()
            } else {
                let name = read.file_name().context("source has no file name")?;
                placed_path(
                    &recorded_path
                        .parent()
                        .context("the recorded source has no parent directory")?
                        .join(name),
                )
            };
            (read, placed)
        }
    };
    let placed_source = provenance_source(&placed, &project_root);
    if placed_source != recorded {
        let other = found(&registry, &placed_source, &project_root)?.outputs;
        if let Some(other) = anchor_of(&other).or_else(|| other.values().next()) {
            bail!(
                "'{placed_source}' is already the source of '{}'; re-import that one, or pick another file",
                other.location()
            );
        }
    }
    let importer = importer_of(&read, &placed)?;
    import_with(
        importer,
        &read,
        &placed,
        &recorded,
        &project_root,
        config,
        settings,
    )
}

/// Moves the embedded asset `id` out of its import's scene file into a loose file at its usual address, keeping its id, provenance and uncooked section.
pub fn extract_embedded(
    id: AssetId,
    project_root: &Path,
    config: &ContentConfig,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetEntry> {
    config.validate()?;
    let entry = registry
        .get(id)
        .with_context(|| format!("no asset {} in the project", id.simple_hex()))?
        .clone();
    let (Some(held), Some(provenance)) = (&entry.embedded_in, &entry.provenance) else {
        bail!("'{}' is a file of its own already", entry.location());
    };
    let address = content_address(config, Path::new(&provenance.source), &provenance.sub_asset);
    let loose_path = project_root.join(&address);
    if registry.id_for_address(&address).is_some() || loose_path.try_exists()? {
        bail!(
            "'{address}' is taken; move or rename it, then extract '{}' again",
            entry.location()
        );
    }
    let host_path = project_root.join(&entry.address);
    let bytes =
        std::fs::read(&host_path).with_context(|| format!("reading '{}'", entry.address))?;
    let loose = asset_record(&bytes, Some(&held.name))
        .with_context(|| format!("malformed content asset '{}'", entry.location()))?;
    let file = ContentFile::parse(&bytes)
        .with_context(|| format!("malformed content asset '{}'", entry.address))?;
    let mut kept = Vec::with_capacity(file.header().embedded.len());
    for asset in file
        .header()
        .embedded
        .iter()
        .filter(|asset| asset.name() != held.name)
    {
        let record = file.embedded(asset)?;
        kept.push(EmbeddedPayload {
            header: file
                .header()
                .record_header(Some(asset.name()))
                .context("an embedded asset is missing from its file's directory")?,
            payload: record.payload,
            uncooked: record.uncooked.map(|section| section.bytes),
            derived_blob: record.cooked.map(|cooked| cooked.blob),
        });
    }
    let host_header = ContentAssetHeader {
        embedded: Vec::new(),
        ..file.header().clone()
    };
    let host = write_content_asset_with_embedded(&host_header, file.host()?.payload, &kept)?;
    let staged_loose = stage(&loose_path, &loose)?;
    let staged_host = stage(&host_path, &host)?;
    replace(staged_loose, &loose_path)?;
    if let Err(error) = replace(staged_host, &host_path) {
        if let Err(removal) = std::fs::remove_file(&loose_path) {
            bail!(
                "rewriting '{}' without '{}' failed ({error:#}), and deleting '{address}' again failed too ({removal}); delete '{address}', since two files with one id stop the project from opening",
                entry.address,
                held.name
            );
        }
        return Err(error);
    }
    let extracted = AssetEntry {
        embedded_in: None,
        address,
        ..entry
    };
    registry.insert(extracted.clone());
    Ok(extracted)
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn provenance_source(source: &Path, project_root: &Path) -> String {
    portable(source.strip_prefix(project_root).unwrap_or(source))
}

struct Existing {
    registry: AssetRegistry,
    headers: HashMap<AssetId, ContentAssetHeader>,
}

fn existing(project_root: &Path, config: &ContentConfig) -> anyhow::Result<Existing> {
    let registry = AssetRegistry::from_content_tree(project_root, &config.root, &config.extension)?;
    let mut files: HashMap<AssetId, ContentAssetHeader> = HashMap::new();
    let mut headers = HashMap::new();
    for entry in registry.iter() {
        let file = match files.entry(entry.file()) {
            Entry::Occupied(file) => file.into_mut(),
            Entry::Vacant(file) => file.insert(read_content_asset_header(
                &project_root.join(&entry.address),
            )?),
        };
        let header = file
            .record_header(entry.embedded_in.as_ref().map(|held| held.name.as_str()))
            .with_context(|| format!("'{}' is not in its file's directory", entry.location()))?;
        headers.insert(entry.id, header);
    }
    Ok(Existing { registry, headers })
}

struct Recorded {
    anchor: String,
    record: ImportRecord,
}

struct Found {
    outputs: BTreeMap<String, AssetEntry>,
    recorded: Option<Recorded>,
}

fn found(registry: &AssetRegistry, source: &str, project_root: &Path) -> anyhow::Result<Found> {
    let mut outputs: BTreeMap<String, AssetEntry> = BTreeMap::new();
    for entry in registry.iter() {
        let Some(provenance) = entry
            .provenance
            .as_ref()
            .filter(|provenance| provenance.source == source)
        else {
            continue;
        };
        if let Some(other) = outputs.insert(provenance.sub_asset.clone(), entry.clone()) {
            bail!(
                "'{}' and '{}' are both '{}' of '{source}'; delete one and import again",
                other.location(),
                entry.location(),
                provenance.sub_asset
            );
        }
    }
    let recorded = match anchor_of(&outputs) {
        Some(anchor) => read_content_asset_header(&project_root.join(&anchor.address))?
            .import
            .map(|record| Recorded {
                anchor: anchor.location(),
                record,
            }),
        None => None,
    };
    Ok(Found { outputs, recorded })
}

fn anchor_of(outputs: &BTreeMap<String, AssetEntry>) -> Option<&AssetEntry> {
    outputs
        .get(HOST)
        .or_else(|| outputs.get(MAIN))
        .filter(|entry| entry.embedded_in.is_none())
}

fn settings_for(
    importer: &dyn ErasedImporter,
    given: Option<Box<dyn Reflect>>,
    found: &Found,
    source: &str,
) -> anyhow::Result<Box<dyn Reflect>> {
    if let Some(Recorded { anchor, record }) = &found.recorded {
        if record.importer != importer.name() {
            bail!(
                "'{anchor}' was imported by '{}', not '{}'",
                record.importer,
                importer.name()
            );
        }
    }
    if let Some(given) = given {
        return Ok(given);
    }
    let Some(Recorded { anchor, record }) = &found.recorded else {
        if !found.outputs.is_empty() {
            log::warn!(
                "'{source}' has outputs but no import record; importing with default settings"
            );
        }
        return Ok(importer.default_settings());
    };
    match importer.settings_from_bytes(record.settings_version, &record.settings) {
        Ok(Some(settings)) => Ok(settings),
        Ok(None) => {
            log::warn!(
                "'{source}' was imported with settings version {}, and '{}' reads version {}; importing with default settings",
                record.settings_version,
                importer.name(),
                importer.settings_version()
            );
            Ok(importer.default_settings())
        }
        Err(error) => bail!(
            "the import record of '{anchor}' does not decode ({error}); import it again with settings"
        ),
    }
}

fn previous_outputs(found: &Found, project_root: &Path) -> PreviousOutputResolver {
    let located = found.outputs.clone();
    let root = project_root.to_path_buf();
    Box::new(move |name| read_asset_record_without_uncooked(&root, located.get(name)?).ok())
}

fn import_with(
    importer: &dyn ErasedImporter,
    read: &Path,
    source: &Path,
    owner: &str,
    project_root: &Path,
    config: &ContentConfig,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    let relative_source = source.strip_prefix(project_root).unwrap_or(source);
    let provenance_source = provenance_source(source, project_root);
    let Existing {
        mut registry,
        headers,
    } = existing(project_root, config)?;
    let found = found(&registry, owner, project_root)?;
    let ImportSettings {
        settings: given,
        asset_settings,
    } = settings;
    let settings = settings_for(importer, given, &found, owner)?;
    let encoded_settings = importer
        .settings_to_bytes(settings.as_ref())
        .map_err(anyhow::Error::msg)?;

    let ids: Arc<Mutex<BTreeMap<String, AssetId>>> = Arc::new(Mutex::new(
        found
            .outputs
            .iter()
            .map(|(key, entry)| (key.clone(), entry.id))
            .collect(),
    ));
    let memo = Arc::clone(&ids);
    let resolver: SubAssetIdResolver = Box::new(move |sub_name| {
        *memo
            .lock()
            .expect("sub-asset identity memo poisoned")
            .entry(sub_name.to_string())
            .or_default()
    });
    let mut ctx = ImportContext::with_sub_asset_id_resolver(source.to_path_buf(), resolver)
        .with_previous_outputs(previous_outputs(&found, project_root));
    importer
        .import(read, settings.as_ref(), asset_settings.as_deref(), &mut ctx)
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .with_context(|| format!("importing '{}'", read.display()))?;
    let outputs = ctx.into_parts();
    let anchor = [HOST, MAIN]
        .into_iter()
        .find(|name| outputs.sub_assets.iter().any(|s| s.name == *name))
        .with_context(|| {
            format!(
                "importing '{}': the importer emitted neither '{HOST}' nor '{MAIN}'",
                read.display()
            )
        })?;

    let mut readable = HashMap::new();
    for dependency in &outputs.dependencies {
        readable
            .entry(dependency.path.as_path())
            .or_insert_with(|| match file_hash(&dependency.path) {
                Ok(hash) => Some(hash),
                Err(error) => {
                    log::warn!(
                        "'{}' refers to '{}', which cannot be read ({error}); importing without it",
                        source.display(),
                        dependency.path.display()
                    );
                    None
                }
            });
    }
    let readable_dependencies: Vec<DependencyEntry> = outputs
        .dependencies
        .iter()
        .filter(|dependency| readable[dependency.path.as_path()].is_some())
        .cloned()
        .collect();
    let copies = if read == source {
        Vec::new()
    } else {
        planned_copies(read, source, project_root, &readable_dependencies)?
    };
    let mut dependencies: Vec<SourceFile> = Vec::new();
    for dependency in &readable_dependencies {
        let path = recorded_dependency(&dependency.path, read, source, project_root);
        if dependencies.iter().all(|file| file.path != path) {
            dependencies.push(SourceFile {
                path,
                hash: readable[dependency.path.as_path()].unwrap_or_default(),
            });
        }
    }
    let record = ImportRecord {
        importer: importer.name().to_string(),
        settings_version: importer.settings_version(),
        settings: encoded_settings,
        source: SourceFile {
            path: provenance_source.clone(),
            hash: file_hash(read).with_context(|| format!("reading '{}'", read.display()))?,
        },
        dependencies,
    };

    let emitted: HashSet<AssetId> = outputs.sub_assets.iter().map(|s| s.asset_id).collect();
    let mut names = HashSet::new();
    let mut ids_seen = HashMap::new();
    let mut destinations = HashSet::new();
    let mut written = Vec::with_capacity(outputs.sub_assets.len());
    let mut plan = Vec::with_capacity(outputs.sub_assets.len());
    let host_file = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == HOST)
        .map(|host| HostFile {
            id: host.asset_id,
            address: loose_address(&registry, config, relative_source, host.asset_id, HOST),
        });
    let mut host = None;
    let mut embedded = Vec::new();
    let resolved = ids.lock().expect("sub-asset identity memo poisoned");
    for sub_asset in &outputs.sub_assets {
        if !names.insert(&sub_asset.name)
            || resolved.get(&sub_asset.name) != Some(&sub_asset.asset_id)
        {
            bail!(
                "importer emitted duplicate or inconsistent identity for '{}'",
                sub_asset.name
            );
        }
        if let Some(other) = ids_seen.insert(sub_asset.asset_id, &sub_asset.name) {
            bail!(
                "importer emitted '{other}' and '{}' with the same identity",
                sub_asset.name
            );
        }
        let embeds = match registry.get(sub_asset.asset_id) {
            Some(entry) => entry.embedded_in.is_some(),
            None => embeds_by_default(sub_asset.type_name),
        };
        let embedded_in = host_file
            .as_ref()
            .filter(|_| sub_asset.name != HOST && embeds);
        let address = match embedded_in {
            Some(host_file) => host_file.address.clone(),
            None => {
                let address = loose_address(
                    &registry,
                    config,
                    relative_source,
                    sub_asset.asset_id,
                    &sub_asset.name,
                );
                if !destinations.insert(address.clone()) {
                    bail!("multiple sub-assets would write '{address}'");
                }
                if let Some(existing) = registry.id_for_address(&address) {
                    if existing != sub_asset.asset_id {
                        if registry
                            .get(existing)
                            .is_some_and(|entry| entry.provenance.is_none())
                        {
                            bail!(
                                "'{address}' was saved from the editor and is not this import's '{}'; move or rename it, then import again",
                                sub_asset.name
                            );
                        }
                        bail!("destination '{address}' is owned by another asset; choose a different source destination or move that asset");
                    }
                } else if project_root.join(&address).try_exists()? {
                    bail!("destination '{address}' is occupied by a file or directory outside the validated content index");
                }
                address
            }
        };
        for reference in &sub_asset.references {
            if !emitted.contains(reference) {
                log::warn!("'{}' references {reference:?}, which this source does not emit; leaving it unresolved (cross-source imports land in a later phase)", sub_asset.name);
            }
        }
        let header = ContentAssetHeader {
            asset_id: sub_asset.asset_id,
            references: sub_asset.references.clone(),
            kind: sub_asset.type_name.to_string(),
            kind_version: sub_asset.kind_version,
            provenance: Some(ImportProvenance {
                source: provenance_source.clone(),
                sub_asset: sub_asset.name.clone(),
            }),
            import: (sub_asset.name == anchor).then(|| record.clone()),
            uncooked: None,
            derived: None,
            embedded: Vec::new(),
        };
        let uncooked = sub_asset.uncooked.as_ref();
        if embedded_in.is_some() {
            embedded.push(EmbeddedOutput {
                header: ContentAssetHeader {
                    uncooked: uncooked.map(described),
                    ..header.clone()
                },
                payload: &sub_asset.bytes,
                uncooked: uncooked.map(|section| section.bytes.as_slice()),
            });
        } else if sub_asset.name == HOST {
            if uncooked.is_some() {
                bail!(
                    "the importer emitted '{HOST}' with uncooked data, which the file that holds an import's embedded assets cannot carry; emit it with `emit`"
                );
            }
            host = Some(HostOutput {
                path: project_root.join(&address),
                header: header.clone(),
                payload: &sub_asset.bytes,
            });
        } else {
            plan.push(PlannedFile {
                path: project_root.join(&address),
                bytes: match uncooked {
                    Some(uncooked) => write_content_asset_with_uncooked(
                        &header,
                        &sub_asset.bytes,
                        uncooked.version,
                        &uncooked.bytes,
                    )?,
                    None => write_content_asset(&header, &sub_asset.bytes)?,
                },
            });
        }
        let entry = AssetEntry {
            embedded_in: embedded_in.map(|host_file| EmbeddedIn {
                host: host_file.id,
                name: sub_asset.name.clone(),
            }),
            id: sub_asset.asset_id,
            address,
            kind: header.kind,
            kind_version: Some(header.kind_version),
            provenance: header.provenance,
        };
        registry.insert(entry.clone());
        written.push(entry);
    }
    drop(resolved);
    if let Some(host) = host {
        let payloads: Vec<EmbeddedPayload> = embedded
            .iter()
            .map(|output| EmbeddedPayload {
                header: output.header.clone(),
                payload: output.payload,
                uncooked: output.uncooked,
                derived_blob: None,
            })
            .collect();
        plan.push(PlannedFile {
            bytes: write_content_asset_with_embedded(&host.header, host.payload, &payloads)?,
            path: host.path,
        });
    }
    let mut removed = Vec::new();
    let mut deletions = Vec::new();
    for (key, output) in &found.outputs {
        if emitted.contains(&output.id) {
            continue;
        }
        let Some(entry) = registry.remove(output.id) else {
            continue;
        };
        let file = entry
            .embedded_in
            .is_none()
            .then(|| project_root.join(&entry.address));
        deletions.push(Pruned {
            key: key.clone(),
            id: output.id,
            file,
        });
        removed.push(output.id);
    }
    if let Some(host_file) = &host_file {
        let kept: HashSet<AssetId> = embedded.iter().map(|o| o.header.asset_id).collect();
        let strays: Vec<AssetId> = registry
            .in_file(host_file.id)
            .filter(|entry| entry.embedded_in.is_some() && !kept.contains(&entry.id))
            .map(|entry| entry.id)
            .collect();
        for id in strays {
            registry.remove(id);
            removed.push(id);
        }
    }
    let rewritten: HashSet<AssetId> = written.iter().map(|entry| entry.id).collect();
    for Pruned { key, id, .. } in &deletions {
        let id = *id;
        for sub_asset in &outputs.sub_assets {
            if rewritten.contains(&sub_asset.asset_id) && sub_asset.references.contains(&id) {
                log::warn!(
                    "'{key}' ({id:?}) is deleted but '{}' still references it",
                    sub_asset.name
                );
            }
        }
        for (other, header) in &headers {
            if !rewritten.contains(other)
                && !removed.contains(other)
                && header.references.contains(&id)
            {
                log::warn!("'{key}' ({id:?}) is deleted but {other:?} still references it");
            }
        }
    }

    let copied: Vec<_> = copies
        .iter()
        .map(|(from, to)| stage_copy(from, to).map(|file| (to, file)))
        .collect::<anyhow::Result<_>>()?;
    let staged: Vec<_> = plan
        .iter()
        .map(|planned| stage(&planned.path, &planned.bytes).map(|file| (&planned.path, file)))
        .collect::<anyhow::Result<_>>()?;
    for (path, file) in copied {
        replace(file, path)?;
    }
    for (path, file) in staged {
        replace(file, path)?;
    }
    for Pruned { key, file, .. } in &deletions {
        let Some(path) = file else {
            continue;
        };
        if let Err(err) = std::fs::remove_file(path) {
            log::warn!(
                "could not delete obsolete output '{key}' at '{}': {err}",
                path.display()
            );
        }
    }
    let siblings = copies
        .iter()
        .skip(1)
        .map(|(_, to)| portable(to.strip_prefix(project_root).unwrap_or(to)))
        .collect();
    Ok(ImportedSource {
        assets: written,
        siblings,
        removed,
    })
}

fn file_hash(path: &Path) -> std::io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut hash = Fnv1a::new();
    hash.raw(&bytes);
    Ok(hash.finish())
}

fn recorded_dependency(
    dependency: &Path,
    read: &Path,
    source: &Path,
    project_root: &Path,
) -> String {
    let from = normalize(dependency);
    let placed = match (read.parent(), source.parent()) {
        (Some(read_dir), Some(placed_dir)) if read != source => {
            match relative_to(&from, &normalize(read_dir)) {
                Some(relative) => normalize(&placed_dir.join(relative)),
                None => from,
            }
        }
        _ => from,
    };
    match relative_to(&placed, project_root) {
        Some(relative) => portable(&relative),
        None => portable(&placed),
    }
}

fn normalize(path: &Path) -> PathBuf {
    let portable = path
        .to_string_lossy()
        .replace(['\\', '/'], std::path::MAIN_SEPARATOR_STR);
    let mut normalized = PathBuf::new();
    for component in Path::new(&portable).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if normalized.file_name().is_some() => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn planned_copies(
    read: &Path,
    placed: &Path,
    project_root: &Path,
    dependencies: &[DependencyEntry],
) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let read_dir = read.parent().context("source has no parent directory")?;
    let placed_dir = placed
        .parent()
        .context("destination has no parent directory")?;
    let mut copies = vec![(read.to_path_buf(), placed.to_path_buf())];
    for dependency in dependencies {
        let from = normalize(&dependency.path);
        let Some(relative) = relative_to(&from, &normalize(read_dir)) else {
            bail!(
                "'{}' refers to '{}', which is not addressed relative to it",
                read.display(),
                dependency.path.display()
            );
        };
        let to = normalize(&placed_dir.join(&relative));
        if !to.starts_with(project_root) {
            bail!(
                "'{}' would land outside the project; choose a destination deeper inside it",
                relative.display()
            );
        }
        if from == to || copies.iter().any(|copy| copy.0 == from && copy.1 == to) {
            continue;
        }
        if copies.iter().any(|copy| copy.1 == to) {
            bail!("two files would be copied to '{}'", to.display());
        }
        copies.push((from, to));
    }
    Ok(copies)
}

fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    let path: Vec<_> = path.components().collect();
    let base: Vec<_> = base.components().collect();
    let shared = path
        .iter()
        .zip(&base)
        .take_while(|(part, other)| part == other)
        .count();
    if shared == 0 {
        return None;
    }
    let mut relative: PathBuf = base[shared..]
        .iter()
        .map(|_| Component::ParentDir)
        .collect();
    relative.extend(&path[shared..]);
    Some(relative)
}

fn stage(path: &Path, bytes: &[u8]) -> anyhow::Result<tempfile::NamedTempFile> {
    let parent = path.parent().context("output path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating '{}'", parent.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("staging '{}'", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing staged '{}'", path.display()))?;
    file.flush()?;
    Ok(file)
}

fn stage_copy(from: &Path, to: &Path) -> anyhow::Result<tempfile::NamedTempFile> {
    let parent = to.parent().context("output path has no parent")?;
    let reading = || format!("reading '{}'", from.display());
    let staging = || format!("staging '{}'", to.display());
    let mut reader = std::fs::File::open(from).with_context(reading)?;
    let permissions = reader.metadata().with_context(reading)?.permissions();
    std::fs::create_dir_all(parent).with_context(|| format!("creating '{}'", parent.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent).with_context(staging)?;
    std::io::copy(&mut reader, &mut file).with_context(staging)?;
    file.flush().with_context(staging)?;
    file.as_file()
        .set_permissions(permissions)
        .with_context(staging)?;
    Ok(file)
}

fn replace(file: tempfile::NamedTempFile, path: &Path) -> anyhow::Result<()> {
    file.persist(path)
        .map_err(|err| err.error)
        .with_context(|| format!("replacing '{}'", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_asset_import::{Importer, NoSettings};

    struct Extension {
        extension: &'static str,
        importer: Option<&'static str>,
    }

    #[test]
    fn each_extension_finds_its_importer_without_regard_to_case() {
        let handled = |extension, importer| Extension {
            extension,
            importer: Some(importer),
        };
        let cases = [
            handled("gltf", "gltf"),
            handled("GLB", "gltf"),
            handled("obj", "obj"),
            handled("bvh", "bvh"),
            handled("png", "image"),
            handled("jpg", "image"),
            handled("JPEG", "image"),
            handled("ogg", "audio"),
            handled("wav", "audio"),
            Extension {
                extension: "mtl",
                importer: None,
            },
            Extension {
                extension: "txt",
                importer: None,
            },
        ];
        for case in &cases {
            assert_eq!(
                importer_for(case.extension).map(|importer| importer.name()),
                case.importer,
                "{}",
                case.extension
            );
        }
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Blob {
        refs: Vec<AssetId>,
    }

    impl concerto_foundation::assets::Asset for Blob {
        fn name() -> &'static str {
            "Blob"
        }

        fn referenced_sub_assets(&self) -> Vec<AssetId> {
            self.refs.clone()
        }
    }

    #[derive(Clone)]
    struct Emit {
        name: &'static str,
    }

    fn emit(name: &'static str) -> Emit {
        Emit { name }
    }

    struct Scripted(Vec<Emit>);

    impl Importer for Scripted {
        type Settings = NoSettings;
        type AssetSettings = NoSettings;

        fn name(&self) -> &'static str {
            "bvh"
        }

        fn supported_extensions(&self) -> &'static [&'static str] {
            &["test"]
        }

        fn import(
            &self,
            _source_path: &Path,
            _settings: &NoSettings,
            _asset_settings: Option<&NoSettings>,
            ctx: &mut ImportContext,
        ) -> Result<(), concerto_asset_import::ImportError> {
            for emit in &self.0 {
                ctx.emit(emit.name, &Blob { refs: Vec::new() })?;
            }
            Ok(())
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        source: PathBuf,
        config: ContentConfig,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().canonicalize().expect("canonical tempdir");
            let source = root.join("assets").join("a.test");
            std::fs::create_dir_all(source.parent().unwrap()).expect("assets");
            std::fs::write(&source, b"").expect("source");
            Self {
                dir,
                source,
                config: ContentConfig::default(),
            }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().canonicalize().expect("canonical tempdir")
        }

        fn import(&self, emits: Vec<Emit>) -> anyhow::Result<ImportedSource> {
            import_with(
                &Scripted(emits),
                &self.source,
                &self.source,
                "assets/a.test",
                &self.root(),
                &self.config,
                ImportSettings::default(),
            )
        }

        fn outputs(&self) -> BTreeMap<String, AssetId> {
            AssetRegistry::from_content_tree(&self.root(), "content", "gasset")
                .expect("scan")
                .iter()
                .filter_map(|entry| {
                    let provenance = entry.provenance.as_ref()?;
                    (provenance.source == "assets/a.test")
                        .then(|| (provenance.sub_asset.clone(), entry.id))
                })
                .collect()
        }

        fn strip_provenance(&self, entry: &AssetEntry) {
            let path = self.root().join(&entry.address);
            let file = std::fs::read(&path).unwrap();
            let (mut header, bytes) =
                concerto_foundation::assets::content::read_content_asset(&file).unwrap();
            header.provenance = None;
            std::fs::write(&path, write_content_asset(&header, bytes).unwrap()).unwrap();
        }
    }

    fn entry<'a>(imported: &'a ImportedSource, name: &str) -> &'a AssetEntry {
        imported
            .assets
            .iter()
            .find(|asset| {
                asset
                    .provenance
                    .as_ref()
                    .is_some_and(|p| p.sub_asset == name)
            })
            .expect("entry written")
    }

    #[test]
    fn an_output_the_importer_stops_emitting_is_deleted_unless_the_editor_saved_it() {
        for saved in [false, true] {
            let fixture = Fixture::new();
            let first = fixture
                .import(vec![emit("main"), emit("keep"), emit("drop")])
                .unwrap();
            let old = entry(&first, "drop").clone();
            let path = fixture.root().join(&old.address);
            if saved {
                fixture.strip_provenance(&old);
            }

            let second = fixture.import(vec![emit("main"), emit("keep")]).unwrap();

            let removed: &[AssetId] = if saved { &[] } else { &[old.id] };
            assert_eq!(second.removed, removed, "saved: {saved}");
            assert_eq!(path.exists(), saved, "saved: {saved}");
            assert_eq!(
                fixture.outputs().get("keep"),
                Some(&entry(&first, "keep").id)
            );
        }
    }
}
