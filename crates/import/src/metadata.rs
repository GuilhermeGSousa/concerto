//! Committed source ownership, independent of generated output locations.
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};
use concerto_core::assets::AssetId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub version: u32,
    pub source_id: AssetId,
    pub importer: String,
    #[serde(default)]
    pub outputs: BTreeMap<String, OutputMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputMetadata {
    pub asset_id: AssetId,
}

pub fn sidecar_path(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_os_string();
    name.push(".import.toml");
    name.into()
}

impl SourceMetadata {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading source metadata '{}'", path.display()))?;
        let metadata: Self = toml::from_str(&text)
            .with_context(|| format!("parsing source metadata '{}'", path.display()))?;
        if metadata.version != 1 {
            bail!(
                "unsupported source metadata version {} in '{}'",
                metadata.version,
                path.display()
            );
        }
        if !matches!(metadata.importer.as_str(), "gltf" | "obj" | "image") {
            bail!(
                "unknown importer '{}' in '{}'",
                metadata.importer,
                path.display()
            );
        }
        let mut ids = HashMap::new();
        for (key, output) in &metadata.outputs {
            if let Some(previous) = ids.insert(output.asset_id, key) {
                bail!(
                    "metadata '{}' assigns the same UUID to '{previous}' and '{key}'",
                    path.display()
                );
            }
        }
        Ok(metadata)
    }
}

/// Normalize historic path spellings even if a moved source no longer exists.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let portable = path.to_string_lossy().replace('\\', "/");
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

pub(crate) fn provenance_path(value: &str, root: &Path) -> PathBuf {
    let path = normalize(Path::new(value));
    if path.is_absolute() {
        return path.canonicalize().unwrap_or(path);
    }
    // New provenance is root-relative; historic CLI provenance could include
    // the project directory relative to its working directory.
    let rooted = root.join(&path);
    if rooted.exists() {
        return rooted.canonicalize().unwrap_or(rooted);
    }
    if let Ok(absolute) = path.canonicalize() {
        return absolute;
    }
    normalize(&rooted)
}

/// Find ownership records without descending into generated trees or symlinks.
pub(crate) fn project_sidecars(root: &Path, content_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("scanning source metadata in '{}'", dir.display()))?
        {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                if path == content_root
                    || matches!(entry.file_name().to_str(), Some(".git" | "target"))
                {
                    continue;
                }
                pending.push(path);
            } else if kind.is_file() && path.to_string_lossy().ends_with(".import.toml") {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}
