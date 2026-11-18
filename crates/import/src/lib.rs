//! The editor's import engine: source files into persistent UUID-addressed content assets.
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use concerto_asset_import::{
    DependencyEntry, EmittedEditorOnly, ErasedImporter, ImportContext, PreviousOutputResolver,
    SubAssetIdResolver,
};
use concerto_foundation::assets::content::{
    read_asset_record_without_editor_only, read_content_asset_header, write_content_asset,
    write_content_asset_with_editor_only, write_content_asset_with_embedded, ContentAssetHeader,
    EditorOnlySection, EmbeddedPayload, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry, EmbeddedIn};
use concerto_foundation::assets::AssetId;
use concerto_reflect::Reflect;

pub mod config;
pub mod metadata;
mod pack;

pub use pack::{pack_imports, PackReport, PackedSource};

use config::{content_address, ContentConfig};
use metadata::{sidecar_path, OutputMetadata, SourceMetadata};

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
    /// The importer's settings, written whole to the sidecar's `[settings]`; `None` imports with the sidecar's.
    pub settings: Option<Box<dyn Reflect>>,
    /// The settings of the source's main asset; `None` keeps the asset's own, or gives a new one the defaults.
    pub asset_settings: Option<Box<dyn Reflect>>,
}

/// The settings an import of a source would use if given none.
pub struct StagedSettings {
    /// The sidecar's importer settings, or the defaults.
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

    /// The settings importing `source` as `destination` would use if given none: the sidecar's, or the defaults, and the main asset's current ones; fails when the sidecar's settings do not parse.
    pub fn staged_settings(
        &self,
        source: &Path,
        destination: &str,
    ) -> anyhow::Result<StagedSettings> {
        let placement = placement(source, destination, &self.project_root)?;
        let importer = importer_of(&placement.source, &placement.placed)?;
        let metadata = source_metadata(
            importer,
            &placement.placed,
            &placement.project_root,
            &self.registry,
        )?;
        let settings = sidecar_settings(importer, &metadata, &placement.placed)?;
        let ctx = ImportContext::new(placement.placed.clone()).with_previous_outputs(
            previous_outputs(&self.registry, &metadata, &placement.project_root),
        );
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
    editor_only: Option<&'a [u8]>,
}

fn described(section: &EmittedEditorOnly) -> EditorOnlySection {
    EditorOnlySection {
        len: section.bytes.len() as u64,
        version: section.version,
    }
}

struct HostOutput<'a> {
    path: PathBuf,
    header: ContentAssetHeader,
    payload: &'a [u8],
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

/// Import a source and return an entry, with provenance, for every content asset
/// written; no registry file is written. The source's committed `.import.toml`
/// sidecar owns output UUIDs; scanning content headers finds their current
/// locations, including files moved since the previous import. An output that has
/// lost its import provenance belongs to the editor, is not rewritten, and is not
/// returned. A source that emits a scene stores its meshes, materials and
/// skeletons inside the scene's file unless the sidecar says `file = true`.
///
/// The complete plan is validated before writing. Files are staged and replaced
/// individually; this is not a multi-file transaction or a concurrent writer API.
pub fn import_source(
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<Vec<AssetEntry>> {
    config.validate()?;
    let source = source
        .canonicalize()
        .with_context(|| format!("resolving source '{}'", source.display()))?;
    std::fs::create_dir_all(project_root)?;
    let project_root = project_root.canonicalize()?;
    Ok(import_placed(
        &source,
        &source,
        &project_root,
        config,
        ImportSettings::default(),
    )?
    .assets)
}

/// Imports `source` into the project as `destination`, copying it and every file it refers to; nothing is copied unless the import succeeds. Given `settings` are written to the sidecar, and given asset settings to the source's main asset.
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
    import_with(importer, read, source, project_root, config, settings)
}

struct Existing {
    registry: AssetRegistry,
    headers: HashMap<AssetId, ContentAssetHeader>,
    metadata: SourceMetadata,
}

fn existing(
    importer: &dyn ErasedImporter,
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<Existing> {
    // Fail on malformed headers and duplicate IDs before touching any output.
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
    let metadata = source_metadata(importer, source, project_root, &registry)?;
    Ok(Existing {
        registry,
        headers,
        metadata,
    })
}

fn source_metadata(
    importer: &dyn ErasedImporter,
    source: &Path,
    project_root: &Path,
    registry: &AssetRegistry,
) -> anyhow::Result<SourceMetadata> {
    let importer_name = importer.name();
    let sidecar = sidecar_path(source);
    Ok(if sidecar.try_exists()? {
        let metadata = SourceMetadata::load(&sidecar)?;
        if metadata.importer != importer_name {
            bail!(
                "source metadata '{}' selects importer '{}', expected '{importer_name}'",
                sidecar.display(),
                metadata.importer
            );
        }
        metadata
    } else {
        // Adopt legacy outputs only through matching provenance, never merely
        // because a generated destination is already occupied.
        let mut outputs = BTreeMap::new();
        for entry in registry.iter() {
            if let Some(provenance) = &entry.provenance {
                if metadata::provenance_path(&provenance.source, project_root) == source
                    && outputs
                        .insert(
                            provenance.sub_asset.clone(),
                            OutputMetadata {
                                asset_id: entry.id,
                                file: false,
                            },
                        )
                        .is_some()
                {
                    bail!(
                        "ambiguous legacy ownership: source '{}' has multiple outputs named '{}'",
                        source.display(),
                        provenance.sub_asset
                    );
                }
            }
        }
        SourceMetadata {
            version: 1,
            source_id: AssetId::new(),
            importer: importer_name.to_string(),
            outputs,
            settings: toml::Table::new(),
        }
    })
}

fn previous_outputs(
    registry: &AssetRegistry,
    metadata: &SourceMetadata,
    project_root: &Path,
) -> PreviousOutputResolver {
    let located: BTreeMap<String, AssetEntry> = metadata
        .outputs
        .iter()
        .filter_map(|(key, output)| Some((key.clone(), registry.get(output.asset_id)?.clone())))
        .collect();
    let root = project_root.to_path_buf();
    Box::new(move |name| read_asset_record_without_editor_only(&root, located.get(name)?).ok())
}

fn sidecar_settings(
    importer: &dyn ErasedImporter,
    metadata: &SourceMetadata,
    source: &Path,
) -> anyhow::Result<Box<dyn Reflect>> {
    importer
        .settings_from_toml(&metadata.settings)
        .map_err(anyhow::Error::msg)
        .with_context(|| {
            format!(
                "reading the [settings] of '{}'",
                sidecar_path(source).display()
            )
        })
}

fn import_with(
    importer: &dyn ErasedImporter,
    read: &Path,
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
    settings: ImportSettings,
) -> anyhow::Result<ImportedSource> {
    let relative_source = source.strip_prefix(project_root).unwrap_or(source);
    let provenance_source = relative_source.to_string_lossy().replace('\\', "/");
    let sidecar = sidecar_path(source);
    let existing = existing(importer, source, project_root, config)?;
    let previous = previous_outputs(&existing.registry, &existing.metadata, project_root);
    let Existing {
        mut registry,
        headers,
        mut metadata,
    } = existing;
    validate_ownership(&metadata, &sidecar, source, project_root, config, &headers)?;
    let ImportSettings {
        settings: given,
        asset_settings,
    } = settings;
    let settings = match given {
        Some(settings) => {
            metadata.settings = importer
                .settings_to_toml(settings.as_ref())
                .map_err(anyhow::Error::msg)?;
            settings
        }
        None => sidecar_settings(importer, &metadata, source)?,
    };

    // The callback cannot return errors. All existing identities are validated
    // beforehand; only allocating new identities happens during the import.
    let ids: Arc<Mutex<BTreeMap<String, AssetId>>> = Arc::new(Mutex::new(
        metadata
            .outputs
            .iter()
            .map(|(key, output)| (key.clone(), output.asset_id))
            .collect(),
    ));
    let memo = Arc::clone(&ids);
    let resolver: SubAssetIdResolver = Box::new(move |sub_name, old_name| {
        let mut memo = memo.lock().expect("sub-asset identity memo poisoned");
        if let Some(id) = memo.get(sub_name) {
            return *id;
        }
        let id = old_name
            .and_then(|old_name| memo.get(old_name).copied())
            .unwrap_or_default();
        memo.insert(sub_name.to_string(), id);
        id
    });
    let mut ctx = ImportContext::with_sub_asset_id_resolver(source.to_path_buf(), resolver)
        .with_previous_outputs(previous);
    importer
        .import(read, settings.as_ref(), asset_settings.as_deref(), &mut ctx)
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .with_context(|| format!("importing '{}'", read.display()))?;
    let outputs = ctx.into_parts();
    let emitted: HashSet<AssetId> = outputs.sub_assets.iter().map(|s| s.asset_id).collect();
    let mut names = HashSet::new();
    let mut ids_seen = HashMap::new();
    let mut destinations = HashSet::new();
    let mut written = Vec::with_capacity(outputs.sub_assets.len());
    let mut plan = Vec::with_capacity(outputs.sub_assets.len());
    let host_file = match outputs.sub_assets.iter().find(|s| s.name == HOST) {
        Some(host) => {
            let address = loose_address(&registry, config, relative_source, host.asset_id, HOST);
            if headers
                .get(&host.asset_id)
                .is_some_and(|header| header.provenance.is_none())
            {
                bail!(
                    "'{address}' was saved from the editor and holds this import's scene id, so the import cannot store its meshes, materials and skeletons there; delete the [outputs.scene] table from '{}', move '{address}' if it sits at '{}', and import again",
                    sidecar.display(),
                    content_address(config, relative_source, HOST)
                );
            }
            Some(HostFile {
                id: host.asset_id,
                address,
            })
        }
        None => None,
    };
    let mut host = None;
    let mut embedded = Vec::new();
    let mut superseded = Vec::new();
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
        if headers
            .get(&sub_asset.asset_id)
            .is_some_and(|header| header.provenance.is_none())
        {
            log::warn!(
                "'{}' was saved from the editor; leaving it untouched",
                sub_asset.name
            );
            continue;
        }
        let keeps_file = metadata
            .outputs
            .get(&sub_asset.name)
            .is_some_and(|output| output.file);
        let embedded_in = host_file.as_ref().filter(|_| {
            sub_asset.name != HOST && embeds_by_default(sub_asset.type_name) && !keeps_file
        });
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
            editor_only: None,
            derived: None,
            embedded: Vec::new(),
        };
        if embedded_in.is_some() {
            if let Some(loose) = registry
                .get(sub_asset.asset_id)
                .filter(|entry| entry.embedded_in.is_none())
            {
                superseded.push(project_root.join(&loose.address));
            }
            embedded.push(EmbeddedOutput {
                header: ContentAssetHeader {
                    editor_only: sub_asset.editor_only.as_ref().map(described),
                    ..header.clone()
                },
                payload: &sub_asset.bytes,
                editor_only: sub_asset
                    .editor_only
                    .as_ref()
                    .map(|section| section.bytes.as_slice()),
            });
        } else if sub_asset.name == HOST {
            if sub_asset.editor_only.is_some() {
                bail!(
                    "the importer emitted '{HOST}' with editor-only data, which the file that holds an import's embedded assets cannot carry; emit it with `emit`"
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
                bytes: match &sub_asset.editor_only {
                    Some(editor_only) => write_content_asset_with_editor_only(
                        &header,
                        &sub_asset.bytes,
                        editor_only.version,
                        &editor_only.bytes,
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
        metadata.outputs.insert(
            sub_asset.name.clone(),
            OutputMetadata {
                asset_id: sub_asset.asset_id,
                file: keeps_file,
            },
        );
        written.push(entry);
    }
    drop(resolved);
    if let Some(host) = host {
        let payloads: Vec<EmbeddedPayload> = embedded
            .iter()
            .map(|output| EmbeddedPayload {
                header: output.header.clone(),
                payload: output.payload,
                editor_only: output.editor_only,
                derived_blob: None,
            })
            .collect();
        plan.push(PlannedFile {
            bytes: write_content_asset_with_embedded(&host.header, host.payload, &payloads)?,
            path: host.path,
        });
    }
    for (new_name, old_name) in &outputs.renames {
        let Some(sub_asset) = outputs.sub_assets.iter().find(|s| &s.name == new_name) else {
            continue;
        };
        if metadata.outputs.get(old_name).map(|o| o.asset_id) != Some(sub_asset.asset_id) {
            continue;
        }
        if metadata.outputs.contains_key(new_name) {
            metadata.outputs.remove(old_name);
        } else {
            log::warn!("'{old_name}' was saved from the editor; the rename to '{new_name}' was not applied");
        }
    }
    let mut removed = Vec::new();
    let mut deletions = Vec::new();
    let obsolete: Vec<(String, AssetId)> = metadata
        .outputs
        .iter()
        .filter(|(_, output)| !emitted.contains(&output.asset_id))
        .map(|(key, output)| (key.clone(), output.asset_id))
        .collect();
    for (key, id) in obsolete {
        let Some(header) = headers.get(&id) else {
            metadata.outputs.remove(&key);
            continue;
        };
        if header.provenance.is_none() {
            log::warn!(
                "'{key}' is no longer emitted but was saved from the editor; leaving it in place"
            );
            continue;
        }
        let Some(entry) = registry.remove(id) else {
            metadata.outputs.remove(&key);
            continue;
        };
        metadata.outputs.remove(&key);
        let file = entry
            .embedded_in
            .is_none()
            .then(|| project_root.join(&entry.address));
        deletions.push(Pruned { key, id, file });
        removed.push(id);
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
    let mut readable = HashMap::new();
    for dependency in &outputs.dependencies {
        readable
            .entry(dependency.path.as_path())
            .or_insert_with(|| match readable_file(&dependency.path) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!(
                        "'{}' refers to '{}', which cannot be read ({error}); importing without it",
                        source.display(),
                        dependency.path.display()
                    );
                    false
                }
            });
    }
    let readable_dependencies: Vec<DependencyEntry> = outputs
        .dependencies
        .iter()
        .filter(|dependency| readable[dependency.path.as_path()])
        .cloned()
        .collect();
    let copies = if read == source {
        Vec::new()
    } else {
        planned_copies(read, source, project_root, &readable_dependencies)?
    };

    // Staging all bytes first catches write errors before replacing old files.
    // Publish metadata first so a retry retains newly allocated identities even
    // if later replacements fail. Full crash recovery is a separate concern.
    let copied: Vec<_> = copies
        .iter()
        .map(|(from, to)| stage_copy(from, to).map(|file| (to, file)))
        .collect::<anyhow::Result<_>>()?;
    let metadata_text = toml::to_string_pretty(&metadata)?;
    let metadata_file = stage(&sidecar, metadata_text.as_bytes())?;
    let staged: Vec<_> = plan
        .iter()
        .map(|planned| stage(&planned.path, &planned.bytes).map(|file| (&planned.path, file)))
        .collect::<anyhow::Result<_>>()?;
    for (path, file) in copied {
        replace(file, path)?;
    }
    replace(metadata_file, &sidecar)?;
    for (path, file) in staged {
        replace(file, path)?;
    }
    for path in &superseded {
        if let Err(err) = std::fs::remove_file(path) {
            log::warn!(
                "could not delete '{}', whose asset is now in the scene file: {err}",
                path.display()
            );
        }
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
        .map(|(_, to)| {
            to.strip_prefix(project_root)
                .unwrap_or(to)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    Ok(ImportedSource {
        assets: written,
        siblings,
        removed,
    })
}

fn readable_file(path: &Path) -> std::io::Result<()> {
    if std::fs::File::open(path)?.metadata()?.is_file() {
        Ok(())
    } else {
        Err(std::io::Error::other("not a file"))
    }
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
        let from = metadata::normalize(&dependency.path);
        let Some(relative) = relative_to(&from, &metadata::normalize(read_dir)) else {
            bail!(
                "'{}' refers to '{}', which is not addressed relative to it",
                read.display(),
                dependency.path.display()
            );
        };
        let to = metadata::normalize(&placed_dir.join(&relative));
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

fn validate_ownership(
    metadata: &SourceMetadata,
    sidecar: &Path,
    source: &Path,
    root: &Path,
    config: &ContentConfig,
    headers: &HashMap<AssetId, ContentAssetHeader>,
) -> anyhow::Result<()> {
    let owned: HashSet<_> = metadata
        .outputs
        .values()
        .map(|output| output.asset_id)
        .collect();
    let mut candidates = metadata::project_sidecars(root, &root.join(&config.root))?;
    for (key, output) in &metadata.outputs {
        if let Some(header) = headers.get(&output.asset_id) {
            let Some(provenance) = &header.provenance else {
                continue;
            };
            if provenance.sub_asset != *key {
                bail!(
                    "metadata output '{key}' conflicts with existing sub-asset '{}'",
                    provenance.sub_asset
                );
            }
            let previous_source = metadata::provenance_path(&provenance.source, root);
            if previous_source != source && previous_source.try_exists()? {
                bail!("metadata output '{key}' is already owned by source '{}'; copying a sidecar does not create new asset identities", previous_source.display());
            }
            candidates.push(sidecar_path(&previous_source));
        }
    }
    candidates.sort();
    candidates.dedup();
    for candidate in candidates {
        if candidate == sidecar || !candidate.try_exists()? {
            continue;
        }
        let other = SourceMetadata::load(&candidate)?;
        if other.source_id == metadata.source_id {
            bail!("duplicate source identity in '{}' and '{}'; move the source and sidecar together, or remove copied metadata before importing a new source", sidecar.display(), candidate.display());
        }
        if other
            .outputs
            .values()
            .any(|output| owned.contains(&output.asset_id))
        {
            bail!(
                "output UUID ownership conflicts between '{}' and '{}'",
                sidecar.display(),
                candidate.display()
            );
        }
    }
    Ok(())
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

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Drawing {
        strokes: u32,
    }

    impl concerto_foundation::assets::EditorOnly for Drawing {
        type Asset = Blob;
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
        renamed_from: Option<&'static str>,
        refs: Vec<&'static str>,
        drawing: Option<u32>,
    }

    fn emit(name: &'static str) -> Emit {
        Emit {
            name,
            renamed_from: None,
            refs: Vec::new(),
            drawing: None,
        }
    }

    impl Emit {
        fn renamed_from(mut self, old: &'static str) -> Self {
            self.renamed_from = Some(old);
            self
        }

        fn with_drawing(mut self, drawing: u32) -> Self {
            self.drawing = Some(drawing);
            self
        }

        fn referring_to(mut self, name: &'static str) -> Self {
            self.refs.push(name);
            self
        }
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
                if let Some(old) = emit.renamed_from {
                    ctx.renamed_from(emit.name, old);
                }
            }
            for emit in &self.0 {
                let refs = emit.refs.iter().map(|r| ctx.sub_asset_id(r)).collect();
                match &emit.drawing {
                    Some(strokes) => {
                        ctx.emit_with(emit.name, &Blob { refs }, &Drawing { strokes: *strokes })?
                    }
                    None => ctx.emit(emit.name, &Blob { refs })?,
                }
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
                &self.root(),
                &self.config,
                ImportSettings::default(),
            )
        }

        fn outputs(&self) -> BTreeMap<String, AssetId> {
            SourceMetadata::load(&sidecar_path(&self.source))
                .expect("sidecar")
                .outputs
                .into_iter()
                .map(|(key, output)| (key, output.asset_id))
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
    fn a_renamed_sub_asset_keeps_its_id_file_and_references() {
        let fixture = Fixture::new();
        let first = fixture.import(vec![emit("animation/0")]).unwrap();
        let old = entry(&first, "animation/0").clone();

        let second = fixture
            .import(vec![
                emit("animation/clip")
                    .renamed_from("animation/0")
                    .with_drawing(7),
                emit("scene").referring_to("animation/clip"),
            ])
            .unwrap();

        let renamed = entry(&second, "animation/clip");
        assert_eq!(renamed.id, old.id);
        assert_eq!(renamed.address, old.address);
        let header = read_content_asset_header(&fixture.root().join(&old.address)).unwrap();
        assert_eq!(header.provenance.unwrap().sub_asset, "animation/clip");
        assert_eq!(
            concerto_foundation::assets::content::read_editor_only::<Drawing>(
                &fixture.root(),
                renamed
            )
            .unwrap(),
            Some(Drawing { strokes: 7 })
        );
        let outputs = fixture.outputs();
        assert_eq!(outputs.get("animation/clip"), Some(&old.id));
        assert!(!outputs.contains_key("animation/0"));
        let scene = entry(&second, "scene");
        let header = read_content_asset_header(&fixture.root().join(&scene.address)).unwrap();
        assert_eq!(header.references, vec![old.id]);
    }

    #[test]
    fn an_output_the_importer_stops_emitting_is_deleted_unless_the_editor_saved_it() {
        for saved in [false, true] {
            let fixture = Fixture::new();
            let first = fixture.import(vec![emit("keep"), emit("drop")]).unwrap();
            let old = entry(&first, "drop").clone();
            let path = fixture.root().join(&old.address);
            if saved {
                fixture.strip_provenance(&old);
            }

            let second = fixture.import(vec![emit("keep")]).unwrap();

            let removed: &[AssetId] = if saved { &[] } else { &[old.id] };
            assert_eq!(second.removed, removed, "saved: {saved}");
            assert_eq!(path.exists(), saved, "saved: {saved}");
            let outputs = fixture.outputs();
            assert_eq!(
                outputs.get("drop"),
                saved.then_some(&old.id),
                "saved: {saved}"
            );
            assert_eq!(outputs.get("keep"), Some(&entry(&first, "keep").id));
        }
    }
}
