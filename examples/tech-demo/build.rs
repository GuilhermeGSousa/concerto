use std::env;
use std::path::{Path, PathBuf};

use fs_extra::copy_items;
use fs_extra::dir::CopyOptions;

/// Copies the `content/` directory into `<exe-dir>/<CARGO_PKG_NAME>-content/content/`,
/// which `ContentAssetRoot::default_for_platform` discovers from the executable
/// name. Every example in this workspace builds into the same shared Cargo
/// `target/` directory, so each gets its own
/// `<pkg-name>-content/` directory there rather than sharing `<exe-dir>/content/`
/// with its siblings — no registry merging needed, since nothing else ever
/// writes into this example's own directory. The `-content` suffix keeps it
/// clear of the `<exe-dir>/<pkg-name>` path Cargo uses for the binary itself.
fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let content_path = manifest_dir.join("content");
    concerto_asset_build::track_assets("content")?;

    if !content_path.exists() {
        return Ok(());
    }

    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out, so the profile
    // directory — where the binary lands — is three levels up.
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output_path = out_dir
        .ancestors()
        .nth(3)
        .unwrap()
        .join(format!("{}-content", env::var("CARGO_PKG_NAME")?));
    std::fs::create_dir_all(&output_path)?;

    copy_items(
        &[content_path],
        Path::new(&output_path),
        &CopyOptions {
            overwrite: true,
            ..Default::default()
        },
    )?;

    Ok(())
}
