//! The editor's import engine: source files into persistent UUID-addressed content assets.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use concerto_asset_import::{DependencyEntry, ImportContext, Importer, SubAssetIdResolver};
use concerto_foundation::assets::content::{
    read_content_asset_header, write_content_asset, ContentAssetHeader, ImportProvenance,
    CONTENT_FORMAT_VERSION,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::AssetId;

pub mod config;
pub mod metadata;

use config::{content_address, ContentConfig};
use metadata::{sidecar_path, OutputMetadata, SourceMetadata};

fn registered_importers() -> Vec<Box<dyn Importer>> {
    vec![
        Box::new(concerto_render::importers::image_importer::ImageImporter),
        Box::new(concerto_gltf::gltf_importer::GltfImporter),
        Box::new(concerto_obj::obj_importer::ObjImporter),
        Box::new(concerto_bvh::bvh_importer::BvhImporter),
    ]
}

/// Whether any registered importer handles this extension, given without a dot.
pub fn supported_extension(extension: &str) -> bool {
    let extension = extension.to_ascii_lowercase();
    registered_importers().iter().any(|importer| {
        importer
            .supported_extensions()
            .contains(&extension.as_str())
    })
}

/// What [`import_source_into`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSource {
    /// Every content asset written, with its import provenance.
    pub assets: Vec<AssetEntry>,
    /// Project-relative paths of the files copied along with the source.
    pub siblings: Vec<String>,
    /// Ids of the content assets deleted because the importer stopped emitting them.
    pub removed: Vec<AssetId>,
}

/// Import a source and return an entry, with provenance, for every content asset
/// written; no registry file is written. The source's committed `.import.toml`
/// sidecar owns output UUIDs; scanning content headers finds their current
/// locations, including files moved since the previous import. An output that has
/// lost its import provenance belongs to the editor, is not rewritten, and is not
/// returned.
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
    Ok(import_placed(&source, &source, &project_root, config)?.assets)
}

/// Imports `source` into the project as `destination`, copying it and every file it refers to; nothing is copied unless the import succeeds.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<ImportedSource> {
    config.validate()?;
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
    std::fs::create_dir_all(project_root)?;
    let project_root = project_root.canonicalize()?;
    let placed = placed_path(&project_root.join(relative));
    if !placed.starts_with(&project_root) {
        bail!("destination '{destination}' resolves outside the project");
    }
    import_placed(&source, &placed, &project_root, config)
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

fn import_placed(
    read: &Path,
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<ImportedSource> {
    let extension = extension_of(read);
    let importers = registered_importers();
    let Some(importer) = importers
        .iter()
        .find(|i| i.supported_extensions().contains(&extension.as_str()))
    else {
        bail!(
            "no importer handles '.{extension}' (source '{}')",
            source.display()
        );
    };
    let importer_name = match extension.as_str() {
        "gltf" | "glb" => "gltf",
        "obj" => "obj",
        "bvh" => "bvh",
        _ => "image",
    };
    import_with(
        importer.as_ref(),
        importer_name,
        read,
        source,
        project_root,
        config,
    )
}

fn import_with(
    importer: &dyn Importer,
    importer_name: &str,
    read: &Path,
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<ImportedSource> {
    let relative_source = source.strip_prefix(project_root).unwrap_or(source);
    let provenance_source = relative_source.to_string_lossy().replace('\\', "/");
    let sidecar = sidecar_path(source);

    // Fail on malformed headers and duplicate IDs before touching any output.
    let mut registry =
        AssetRegistry::from_content_tree(project_root, &config.root, &config.extension)?;
    let mut headers = HashMap::new();
    for entry in registry.iter() {
        headers.insert(
            entry.id,
            read_content_asset_header(&project_root.join(&entry.address))?,
        );
    }
    let mut metadata = if sidecar.try_exists()? {
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
        for (id, header) in &headers {
            if let Some(provenance) = &header.provenance {
                if metadata::provenance_path(&provenance.source, project_root) == source
                    && outputs
                        .insert(
                            provenance.sub_asset.clone(),
                            OutputMetadata { asset_id: *id },
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
    };
    validate_ownership(&metadata, &sidecar, source, project_root, config, &headers)?;

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
        .with_settings(metadata.settings.clone());
    importer
        .import(read, &mut ctx)
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .with_context(|| format!("importing '{}'", read.display()))?;
    let outputs = ctx.into_parts();
    let emitted: HashSet<AssetId> = outputs.sub_assets.iter().map(|s| s.asset_id).collect();
    let mut names = HashSet::new();
    let mut ids_seen = HashMap::new();
    let mut destinations = HashSet::new();
    let mut written = Vec::with_capacity(outputs.sub_assets.len());
    let mut plan = Vec::with_capacity(outputs.sub_assets.len());
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
        let address = registry
            .get(sub_asset.asset_id)
            .map(|entry| entry.address.clone())
            .unwrap_or_else(|| content_address(config, relative_source, &sub_asset.name));
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
        for reference in &sub_asset.references {
            if !emitted.contains(reference) {
                log::warn!("'{}' references {reference:?}, which this source does not emit; leaving it unresolved (cross-source imports land in a later phase)", sub_asset.name);
            }
        }
        let header = ContentAssetHeader {
            format_version: CONTENT_FORMAT_VERSION,
            asset_id: sub_asset.asset_id,
            references: sub_asset.references.clone(),
            kind: sub_asset.type_name.to_string(),
            provenance: Some(ImportProvenance {
                source: provenance_source.clone(),
                sub_asset: sub_asset.name.clone(),
            }),
        };
        plan.push((
            project_root.join(&address),
            write_content_asset(&header, &sub_asset.bytes)?,
        ));
        let entry = AssetEntry {
            id: sub_asset.asset_id,
            address,
            kind: header.kind,
            provenance: header.provenance,
        };
        registry.insert(entry.clone());
        metadata.outputs.insert(
            sub_asset.name.clone(),
            OutputMetadata {
                asset_id: sub_asset.asset_id,
            },
        );
        written.push(entry);
    }
    drop(resolved);
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
        deletions.push((key, id, project_root.join(&entry.address)));
        removed.push(id);
    }
    let rewritten: HashSet<AssetId> = written.iter().map(|entry| entry.id).collect();
    for (key, id, _) in &deletions {
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
    let copies = if read == source {
        Vec::new()
    } else {
        planned_copies(read, source, project_root, &outputs.dependencies)?
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
        .map(|(path, bytes)| stage(path, bytes).map(|file| (path, file)))
        .collect::<anyhow::Result<_>>()?;
    for (path, file) in copied {
        replace(file, path)?;
    }
    replace(metadata_file, &sidecar)?;
    for (path, file) in staged {
        replace(file, path)?;
    }
    for (key, _, path) in &deletions {
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

    #[test]
    fn supported_extensions_are_matched_without_regard_to_case() {
        assert!(supported_extension("obj"));
        assert!(supported_extension("GLB"), "matching is case-insensitive");
        assert!(supported_extension("gltf"));
        assert!(supported_extension("png"));
        assert!(!supported_extension("txt"));
        assert!(
            !supported_extension(".obj"),
            "the caller passes an extension, not a suffix with its dot"
        );
    }

    #[test]
    fn a_normalised_path_stays_under_the_canonical_directory_it_was_joined_to() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().canonicalize().expect("canonical tempdir");
        let path = metadata::normalize(&root.join("assets").join("..").join("square.mtl"));
        assert_eq!(path, root.join("square.mtl"));
        assert!(path.starts_with(&root));
    }

    #[test]
    fn a_dependency_whose_parent_steps_are_already_resolved_keeps_its_relative_position() {
        let source = Path::new("/source");
        let project = Path::new("/project");
        let copies = planned_copies(
            &source.join("models").join("tri.gltf"),
            &project.join("assets").join("models").join("tri.gltf"),
            project,
            &[DependencyEntry {
                path: source.join("triangle_ext.bin"),
                content_hash: 0,
            }],
        )
        .expect("the buffer lands inside the project");
        assert_eq!(
            copies[1],
            (
                source.join("triangle_ext.bin"),
                project.join("assets").join("triangle_ext.bin")
            )
        );
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

    type Entry = (&'static str, Option<&'static str>, Vec<&'static str>);

    struct Scripted(Vec<Entry>);

    impl Importer for Scripted {
        fn supported_extensions(&self) -> &'static [&'static str] {
            &["test"]
        }

        fn import(
            &self,
            _source_path: &Path,
            ctx: &mut ImportContext,
        ) -> Result<(), concerto_asset_import::ImportError> {
            for (name, old, _) in &self.0 {
                if let Some(old) = old {
                    ctx.renamed_from(name, old);
                }
            }
            for (name, _, refs) in &self.0 {
                let refs = refs.iter().map(|r| ctx.sub_asset_id(r)).collect();
                ctx.emit(name, &Blob { refs })?;
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

        fn import(&self, entries: Vec<Entry>) -> anyhow::Result<ImportedSource> {
            import_with(
                &Scripted(entries),
                "bvh",
                &self.source,
                &self.source,
                &self.root(),
                &self.config,
            )
        }

        fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            fn visit(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
                for entry in std::fs::read_dir(dir).expect("read dir") {
                    let path = entry.expect("entry").path();
                    if path.is_dir() {
                        visit(&path, files);
                    } else {
                        let bytes = std::fs::read(&path).expect("read file");
                        files.insert(path, bytes);
                    }
                }
            }
            let mut files = BTreeMap::new();
            visit(&self.root(), &mut files);
            files
        }

        fn outputs(&self) -> BTreeMap<String, AssetId> {
            SourceMetadata::load(&sidecar_path(&self.source))
                .expect("sidecar")
                .outputs
                .into_iter()
                .map(|(key, output)| (key, output.asset_id))
                .collect()
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
        let first = fixture.import(vec![("animation/0", None, vec![])]).unwrap();
        let old = entry(&first, "animation/0").clone();

        let second = fixture
            .import(vec![
                ("animation/clip", Some("animation/0"), vec![]),
                ("scene", None, vec!["animation/clip"]),
            ])
            .unwrap();

        let renamed = entry(&second, "animation/clip");
        assert_eq!(renamed.id, old.id);
        assert_eq!(renamed.address, old.address);
        let path = fixture.root().join(&old.address);
        let header = read_content_asset_header(&path).unwrap();
        assert_eq!(header.provenance.unwrap().sub_asset, "animation/clip");
        let outputs = fixture.outputs();
        assert_eq!(outputs.get("animation/clip"), Some(&old.id));
        assert!(!outputs.contains_key("animation/0"));
        let scene = entry(&second, "scene");
        let header = read_content_asset_header(&fixture.root().join(&scene.address)).unwrap();
        assert_eq!(header.references, vec![old.id]);
    }

    #[test]
    fn a_second_import_after_a_rename_keeps_the_same_id() {
        let fixture = Fixture::new();
        let first = fixture.import(vec![("animation/0", None, vec![])]).unwrap();
        let id = entry(&first, "animation/0").id;
        fixture
            .import(vec![("animation/clip", Some("animation/0"), vec![])])
            .unwrap();

        let third = fixture
            .import(vec![("animation/clip", None, vec![])])
            .unwrap();
        let fourth = fixture
            .import(vec![("animation/clip", Some("animation/0"), vec![])])
            .unwrap();

        assert_eq!(entry(&third, "animation/clip").id, id);
        assert_eq!(entry(&fourth, "animation/clip").id, id);
        assert_eq!(fixture.outputs().get("animation/clip"), Some(&id));
    }

    #[test]
    fn two_names_hinting_one_old_name_fail_without_writing() {
        let fixture = Fixture::new();
        fixture.import(vec![("animation/0", None, vec![])]).unwrap();
        let before = fixture.snapshot();

        let result = fixture.import(vec![
            ("animation/a", Some("animation/0"), vec![]),
            ("animation/b", Some("animation/0"), vec![]),
        ]);

        assert!(result.is_err());
        assert_eq!(fixture.snapshot(), before);
    }

    static LOGS: Mutex<Vec<(std::thread::ThreadId, String)>> = Mutex::new(Vec::new());

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            LOGS.lock()
                .unwrap()
                .push((std::thread::current().id(), record.args().to_string()));
        }

        fn flush(&self) {}
    }

    struct Logs;

    impl Logs {
        fn lines(&self) -> Vec<String> {
            let me = std::thread::current().id();
            LOGS.lock()
                .unwrap()
                .iter()
                .filter(|(thread, _)| *thread == me)
                .map(|(_, line)| line.clone())
                .collect()
        }
    }

    fn capture_logs() -> Logs {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            log::set_logger(&Capture).unwrap();
            log::set_max_level(log::LevelFilter::Warn);
        });
        let me = std::thread::current().id();
        LOGS.lock().unwrap().retain(|(thread, _)| *thread != me);
        Logs
    }

    fn strip_provenance(fixture: &Fixture, entry: &AssetEntry) {
        let path = fixture.root().join(&entry.address);
        let file = std::fs::read(&path).unwrap();
        let (mut header, bytes) =
            concerto_foundation::assets::content::read_content_asset(&file).unwrap();
        header.provenance = None;
        std::fs::write(&path, write_content_asset(&header, bytes).unwrap()).unwrap();
    }

    #[test]
    fn an_output_the_importer_stops_emitting_is_deleted() {
        let fixture = Fixture::new();
        let first = fixture
            .import(vec![("keep", None, vec![]), ("drop", None, vec![])])
            .unwrap();
        let dropped = entry(&first, "drop").clone();
        let kept = entry(&first, "keep").clone();

        let second = fixture.import(vec![("keep", None, vec![])]).unwrap();

        assert_eq!(second.removed, vec![dropped.id]);
        assert!(!fixture.root().join(&dropped.address).exists());
        assert!(fixture.root().join(&kept.address).exists());
        let outputs = fixture.outputs();
        assert!(!outputs.contains_key("drop"));
        assert_eq!(outputs.get("keep"), Some(&kept.id));
    }

    #[test]
    fn an_obsolete_output_saved_from_the_editor_is_kept() {
        let fixture = Fixture::new();
        let first = fixture
            .import(vec![("keep", None, vec![]), ("drop", None, vec![])])
            .unwrap();
        let dropped = entry(&first, "drop").clone();
        strip_provenance(&fixture, &dropped);

        let second = fixture.import(vec![("keep", None, vec![])]).unwrap();

        assert!(second.removed.is_empty());
        assert!(fixture.root().join(&dropped.address).exists());
        assert_eq!(fixture.outputs().get("drop"), Some(&dropped.id));
    }

    #[test]
    fn an_obsolete_output_whose_file_is_missing_only_loses_its_key() {
        let fixture = Fixture::new();
        let first = fixture
            .import(vec![("keep", None, vec![]), ("drop", None, vec![])])
            .unwrap();
        let dropped = entry(&first, "drop").clone();
        std::fs::remove_file(fixture.root().join(&dropped.address)).unwrap();

        let second = fixture.import(vec![("keep", None, vec![])]).unwrap();

        assert!(second.removed.is_empty());
        assert!(!fixture.outputs().contains_key("drop"));
    }

    #[test]
    fn an_editor_saved_file_under_a_hinted_rename_keeps_its_file_and_key() {
        let fixture = Fixture::new();
        let first = fixture.import(vec![("animation/0", None, vec![])]).unwrap();
        let old = entry(&first, "animation/0").clone();
        strip_provenance(&fixture, &old);

        let second = fixture
            .import(vec![("animation/clip", Some("animation/0"), vec![])])
            .unwrap();

        assert!(second.removed.is_empty());
        assert!(fixture.root().join(&old.address).exists());
        assert_eq!(fixture.outputs().get("animation/0"), Some(&old.id));
    }

    #[test]
    fn pruning_warns_only_for_referrers_the_import_does_not_rewrite() {
        let fixture = Fixture::new();
        let first = fixture
            .import(vec![
                ("drop", None, vec![]),
                ("rewritten", None, vec!["drop"]),
                ("untouched", None, vec!["drop"]),
            ])
            .unwrap();
        let untouched = entry(&first, "untouched").clone();
        strip_provenance(&fixture, &untouched);
        let logger = capture_logs();

        fixture
            .import(vec![
                ("rewritten", None, vec![]),
                ("untouched", None, vec![]),
            ])
            .unwrap();

        let lines = logger.lines();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("drop") && line.contains(&format!("{:?}", untouched.id))),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("'rewritten'")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_staging_failure_leaves_the_obsolete_file_in_place() {
        let fixture = Fixture::new();
        let first = fixture
            .import(vec![("keep", None, vec![]), ("drop", None, vec![])])
            .unwrap();
        let dropped = entry(&first, "drop").clone();
        let before = fixture.snapshot();
        let missing = fixture.root().join("elsewhere").join("a.test");

        let result = import_with(
            &Scripted(vec![("keep", None, vec![])]),
            "bvh",
            &missing,
            &fixture.source,
            &fixture.root(),
            &fixture.config,
        );

        assert!(result.is_err());
        assert!(fixture.root().join(&dropped.address).exists());
        assert_eq!(fixture.snapshot(), before);
    }
}
