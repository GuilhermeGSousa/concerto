//! Offline import of source files into persistent UUID-addressed content assets.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context};
use concerto_asset_import::{DependencyEntry, ImportContext, Importer, SubAssetIdResolver};
use concerto_foundation::assets::content::{
    read_content_asset_header, write_content_asset, AssetRegistry, ContentAssetHeader,
    ImportProvenance, CONTENT_FORMAT_VERSION, REGISTRY_FILE_NAME,
};
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

/// One content asset written by [`import_source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAsset {
    /// The persistent UUID written into the content asset header.
    pub asset_id: AssetId,
    /// The sub-asset name within the source, e.g. `"mesh/0"`.
    pub sub_asset_name: String,
    /// The project-relative content-tree address it was written to.
    pub address: String,
    /// The asset type tag (`Asset::name()`), e.g. `"Mesh"`.
    pub kind: String,
}

/// What [`import_source_into`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSource {
    pub assets: Vec<ImportedAsset>,
    /// Project-relative paths of the files copied along with the source.
    pub siblings: Vec<String>,
}

/// Import a source and rebuild its project's registry. The source's committed
/// `.import.toml` sidecar owns output UUIDs; scanning content headers finds their
/// current locations, including files moved since the previous import. An output
/// that has lost its import provenance belongs to the editor and is not rewritten.
///
/// The complete plan is validated before writing. Files are staged and replaced
/// individually; this is not a multi-file transaction or a concurrent writer API.
pub fn import_source(
    source: &Path,
    project_root: &Path,
    config: &ContentConfig,
) -> anyhow::Result<Vec<ImportedAsset>> {
    validate_config(config)?;
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
    validate_config(config)?;
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
        _ => "image",
    };
    let relative_source = source.strip_prefix(project_root).unwrap_or(source);
    let provenance_source = relative_source.to_string_lossy().replace('\\', "/");
    let sidecar = sidecar_path(source);

    // Fail on malformed headers and duplicate IDs before touching any output.
    let mut registry =
        AssetRegistry::from_content_tree(project_root, &config.root, &config.extension)?;
    let mut headers = HashMap::new();
    for (id, address) in registry.iter() {
        headers.insert(id, read_content_asset_header(&project_root.join(address))?);
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
    let resolver: SubAssetIdResolver = Box::new(move |sub_name| {
        *memo
            .lock()
            .expect("sub-asset identity memo poisoned")
            .entry(sub_name.to_string())
            .or_default()
    });
    let mut ctx = ImportContext::with_sub_asset_id_resolver(source.to_path_buf(), resolver);
    importer
        .import(read, &mut ctx)
        .map_err(|err| anyhow::anyhow!("{err:?}"))
        .with_context(|| format!("importing '{}'", read.display()))?;
    let outputs = ctx.into_parts();
    let emitted: HashSet<AssetId> = outputs.sub_assets.iter().map(|s| s.asset_id).collect();
    let mut names = HashSet::new();
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
            .map(str::to_owned)
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
        registry.insert(sub_asset.asset_id, &address);
        metadata.outputs.insert(
            sub_asset.name.clone(),
            OutputMetadata {
                asset_id: sub_asset.asset_id,
            },
        );
        written.push(ImportedAsset {
            asset_id: sub_asset.asset_id,
            sub_asset_name: sub_asset.name.clone(),
            address,
            kind: header.kind,
        });
    }
    drop(resolved);
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
    #[derive(serde::Serialize)]
    struct RegistryFile {
        assets: BTreeMap<String, String>,
    }
    let registry_text = toml::to_string_pretty(&RegistryFile {
        assets: registry
            .iter()
            .map(|(id, address)| (id.simple_hex(), address.to_owned()))
            .collect(),
    })?;
    let registry_path = project_root.join(REGISTRY_FILE_NAME);
    let registry_file = stage(&registry_path, registry_text.as_bytes())?;
    for (path, file) in copied {
        replace(file, path)?;
    }
    replace(metadata_file, &sidecar)?;
    for (path, file) in staged {
        replace(file, path)?;
    }
    replace(registry_file, &registry_path)?;
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
        let Ok(relative) = dependency.path.strip_prefix(read_dir) else {
            bail!(
                "'{}' refers to '{}', which is not addressed relative to it",
                read.display(),
                dependency.path.display()
            );
        };
        let to = metadata::normalize(&placed_dir.join(relative));
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

fn validate_config(config: &ContentConfig) -> anyhow::Result<()> {
    let root = Path::new(&config.root);
    if root.as_os_str().is_empty()
        || root.is_absolute()
        || root
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("content root must be a nonempty project-relative directory without '.' or '..'");
    }
    if config.extension.is_empty() || config.extension.contains(['/', '\\', '.']) {
        bail!("content extension must be a nonempty extension without dots or path separators");
    }
    Ok(())
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
}
