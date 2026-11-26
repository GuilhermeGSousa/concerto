//! Converts a project's imports from one file per output to one scene file per import.
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use concerto_foundation::assets::content::{
    asset_record, read_content_asset_header, read_content_asset_parts,
    write_content_asset_with_embedded, ContentAssetHeader, EmbeddedPayload,
};
use concerto_foundation::assets::registry::AssetRegistry;

use crate::config::ContentConfig;
use crate::metadata::{self, SourceMetadata};
use crate::{embeds_by_default, replace, stage, HOST};

/// One source whose scene file [`pack_imports`] considered, and how many loose outputs it embedded there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedSource {
    pub source: String,
    pub embedded: usize,
}

/// What [`pack_imports`] did.
///
/// It holds an entry per source with a scene file, and a line per source it left
/// alone, with the reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackReport {
    pub sources: Vec<PackedSource>,
    pub skipped: Vec<String>,
}

struct Host {
    source: String,
    path: PathBuf,
    loose: Vec<PathBuf>,
}

enum Plan {
    Pack(Host),
    Skip(String),
}

/// Embeds each import's loose meshes, materials and skeletons into its scene file, keeping ids and payload bytes.
///
/// # Errors
///
/// Fails when the project does not resolve, when `content.toml`, the content
/// tree or a sidecar cannot be read, when two sources claim one output, and when
/// a file cannot be read or written.
pub fn pack_imports(project_root: &Path) -> anyhow::Result<PackReport> {
    let root = project_root
        .canonicalize()
        .with_context(|| format!("resolving project '{}'", project_root.display()))?;
    let config = ContentConfig::load_or_default(&root)?;
    let content_root = root.join(&config.root);
    let registry = AssetRegistry::from_content_tree(&root, &config.root, &config.extension)?;
    let mut sidecars = metadata::project_sidecars(&root, &content_root)?;
    sidecars.sort();

    let mut report = PackReport::default();
    let mut hosts = Vec::new();
    let mut claimed = HashSet::new();
    for sidecar in &sidecars {
        let metadata = SourceMetadata::load(sidecar)?;
        let source = source_of(sidecar);
        let label = label(&source, &root);
        match plan(&metadata, &source, &label, &root, &registry)? {
            Plan::Pack(host) => {
                for path in &host.loose {
                    if !claimed.insert(path.clone()) {
                        bail!(
                            "'{}' is an output of '{label}' and of another source; fix the sidecars and run again",
                            path.display()
                        );
                    }
                }
                hosts.push(host);
            }
            Plan::Skip(reason) => report.skipped.push(format!("{label}: skipped, {reason}")),
        }
    }
    let known: HashSet<&PathBuf> = sidecars.iter().collect();
    let orphans: BTreeSet<String> = registry
        .iter()
        .filter_map(|entry| entry.provenance.as_ref())
        .map(|provenance| metadata::provenance_path(&provenance.source, &root))
        .filter(|source| !known.contains(&metadata::sidecar_path(source)))
        .map(|source| label(&source, &root))
        .collect();
    for orphan in orphans {
        report.skipped.push(format!(
            "{orphan}: skipped, it has no sidecar in the project; importing it again converts it"
        ));
    }

    for host in &hosts {
        pack(host, &content_root)?;
        report.sources.push(PackedSource {
            source: host.source.clone(),
            embedded: host.loose.len(),
        });
    }
    Ok(report)
}

fn source_of(sidecar: &Path) -> PathBuf {
    let name = sidecar.to_string_lossy();
    PathBuf::from(name.strip_suffix(".import.toml").unwrap_or(&name))
}

fn label(source: &Path, root: &Path) -> String {
    source
        .strip_prefix(root)
        .unwrap_or(source)
        .to_string_lossy()
        .replace('\\', "/")
}

fn plan(
    metadata: &SourceMetadata,
    source: &Path,
    label: &str,
    root: &Path,
    registry: &AssetRegistry,
) -> anyhow::Result<Plan> {
    let Some(scene) = metadata.outputs.get(HOST) else {
        return Ok(Plan::Skip("it has no scene output".into()));
    };
    let Some(host) = registry.get(scene.asset_id) else {
        return Ok(Plan::Skip("its scene is not in the content tree".into()));
    };
    let Some(provenance) = &host.provenance else {
        return Ok(Plan::Skip(format!(
            "its scene '{}' was saved from the editor",
            host.address
        )));
    };
    if host.embedded_in.is_some() || metadata::provenance_path(&provenance.source, root) != source {
        return Ok(Plan::Skip(format!(
            "its scene id belongs to '{}', which another source owns",
            host.location()
        )));
    }
    let path = root.join(&host.address);
    if read_content_asset_header(&path)?.derived.is_some() {
        bail!(
            "'{}' carries cooked derived data, so it cannot hold embedded assets; import '{label}' again instead",
            host.address
        );
    }
    let mut loose = Vec::new();
    for (key, output) in &metadata.outputs {
        if key == HOST || output.file {
            continue;
        }
        let Some(entry) = registry.get(output.asset_id) else {
            continue;
        };
        let Some(provenance) = &entry.provenance else {
            continue;
        };
        if entry.embedded_in.is_some() || !embeds_by_default(&entry.kind) {
            continue;
        }
        if provenance.sub_asset != *key {
            bail!(
                "output '{key}' of '{label}' is '{}' in '{}'; import '{label}' again instead",
                provenance.sub_asset,
                entry.address
            );
        }
        if registry.in_file(host.id).any(|held| {
            held.embedded_in
                .as_ref()
                .is_some_and(|held| held.name == *key)
        }) {
            bail!(
                "'{}' already holds an asset named '{key}', and '{}' is another; delete one and run again",
                host.address,
                entry.address
            );
        }
        loose.push(root.join(&entry.address));
    }
    Ok(Plan::Pack(Host {
        source: label.to_string(),
        path,
        loose,
    }))
}

fn pack(host: &Host, content_root: &Path) -> anyhow::Result<()> {
    if host.loose.is_empty() {
        return Ok(());
    }
    let read = |path: &PathBuf| {
        std::fs::read(path).with_context(|| format!("reading '{}'", path.display()))
    };
    let file = read(&host.path)?;
    let parts = read_content_asset_parts(&file)
        .with_context(|| format!("malformed content asset '{}'", host.path.display()))?;
    let mut records = Vec::with_capacity(parts.header.embedded.len() + host.loose.len());
    for held in &parts.header.embedded {
        records.push(asset_record(&file, Some(held.name()))?);
    }
    for path in &host.loose {
        records.push(read(path)?);
    }
    let mut embedded = Vec::with_capacity(records.len());
    for record in &records {
        let parts = read_content_asset_parts(record)?;
        embedded.push(EmbeddedPayload {
            header: parts.header,
            payload: parts.payload,
            uncooked: parts.uncooked,
            derived_blob: parts.derived_blob,
        });
    }
    let header = ContentAssetHeader {
        embedded: Vec::new(),
        ..parts.header.clone()
    };
    let bytes = write_content_asset_with_embedded(&header, parts.payload, &embedded)
        .with_context(|| format!("packing '{}'", host.path.display()))?;
    let permissions = std::fs::metadata(&host.path)
        .with_context(|| format!("reading '{}'", host.path.display()))?
        .permissions();
    let staged = stage(&host.path, &bytes)?;
    staged
        .as_file()
        .set_permissions(permissions)
        .with_context(|| format!("staging '{}'", host.path.display()))?;
    replace(staged, &host.path)?;
    for path in &host.loose {
        std::fs::remove_file(path).with_context(|| {
            format!(
                "deleting '{}', whose asset is now in the scene file",
                path.display()
            )
        })?;
        let mut folder = path.parent();
        while let Some(dir) = folder {
            if dir == content_root
                || !dir.starts_with(content_root)
                || std::fs::remove_dir(dir).is_err()
            {
                break;
            }
            folder = dir.parent();
        }
    }
    Ok(())
}
