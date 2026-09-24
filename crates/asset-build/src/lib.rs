//! Cargo dependency tracking for compile-time `asset_id!` references.
use std::io;
use std::path::Path;

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
    Ok(())
}
