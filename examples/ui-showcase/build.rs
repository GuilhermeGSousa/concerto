#![allow(missing_docs, reason = "example build script, not public API")]

use std::env;
use std::path::{Path, PathBuf};

use fs_extra::copy_items;
use fs_extra::dir::CopyOptions;

fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let content_path = manifest_dir.join("content");
    concerto_asset_build::track_assets("content")?;

    if !content_path.exists() {
        return Ok(());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let output_path = concerto_asset_build::profile_dir(&out_dir)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "OUT_DIR {} is not under a build directory",
                out_dir.display()
            )
        })?
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
