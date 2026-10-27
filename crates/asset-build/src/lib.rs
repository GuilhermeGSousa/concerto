//! Cargo dependency tracking for compile-time `asset_id!` references, and the registry a game embeds.
use std::io;
use std::path::Path;

use anyhow::Context;
use concerto_asset_format::{scan_content, write_registry, ContentConfig};

const REGISTRY_FILE: &str = "concerto_registry.toml";

/// Register the package's content directory from its `build.rs`:
///
/// ```no_run
/// fn main() -> std::io::Result<()> {
///     concerto_asset_build::track_assets("content")
/// }
/// ```
///
/// Cargo watches the directory recursively, including additions and removals.
/// Re-running this build script recompiles the package and re-expands its asset
/// macros. This reads no asset payloads and works on stable Rust. Register one
/// common directory containing all assets referenced by the package's macros.
/// It also writes the registry of the content tree `content.toml` names through
/// [`write_registry_into`], which `concerto::content_registry!()` embeds; a
/// duplicate id or an unreadable header fails the build.
pub fn track_assets(directory: impl AsRef<Path>) -> io::Result<()> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| io::Error::other("track_assets must be called from a Cargo build script"))?;
    let directory = Path::new(&manifest).join(directory).canonicalize()?;
    if !directory.is_dir() {
        return Err(io::Error::other("asset tracking root must be a directory"));
    }
    let directory = directory
        .to_str()
        .ok_or_else(|| io::Error::other("asset tracking root must be a valid UTF-8 path"))?;
    if directory.contains(['\n', '\r']) {
        return Err(io::Error::other(
            "asset tracking root cannot contain newlines",
        ));
    }
    println!("cargo:rerun-if-changed={directory}");
    println!("cargo:rustc-env=CONCERTO_ASSET_ROOT={directory}");

    let config = Path::new(&manifest).join("content.toml");
    if let Some(config) = config.is_file().then(|| config.to_str()).flatten() {
        println!("cargo:rerun-if-changed={config}");
    }

    let out_dir = std::env::var_os("OUT_DIR")
        .ok_or_else(|| io::Error::other("track_assets must be called from a Cargo build script"))?;
    if let Err(error) = write_registry_into(Path::new(&manifest), Path::new(&out_dir)) {
        let message = format!("{error:#}");
        println!("cargo:warning={}", message.replace(['\n', '\r'], " "));
        return Err(io::Error::other(message));
    }
    Ok(())
}

/// Writes the registry of the content tree `<manifest>/content.toml` names to `<out_dir>/concerto_registry.toml`.
pub fn write_registry_into(manifest: &Path, out_dir: &Path) -> anyhow::Result<()> {
    let config = ContentConfig::load_or_default(manifest)?;
    let entries = scan_content(manifest, &manifest.join(&config.root), &config.extension)?;
    let path = out_dir.join(REGISTRY_FILE);
    std::fs::write(&path, write_registry(&entries))
        .with_context(|| format!("failed to write '{}'", path.display()))
}
