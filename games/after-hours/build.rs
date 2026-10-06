//! Generates `AssetId` constants from `content-manifest.txt` and stages
//! those files next to the binary.
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

const SOURCE_CONTENT: &str = "../../examples/tech-demo/content";
const PACK: &str = "UAL1";

fn main() -> anyhow::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let source_root = manifest_dir.join(SOURCE_CONTENT).canonicalize()?;
    concerto_asset_build::track_assets(format!("{SOURCE_CONTENT}/{PACK}"))?;
    println!("cargo:rerun-if-changed=content-manifest.txt");

    let manifest = fs::read_to_string(manifest_dir.join("content-manifest.txt"))?;
    let entries: Vec<(&str, &str)> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().expect("manifest line has a name");
            let file = parts.next().expect("manifest line has a file");
            (name, file)
        })
        .collect();

    let mut generated = String::new();
    for (name, file) in &entries {
        writeln!(
            generated,
            "pub const {name}: AssetId = asset_id!(\"{SOURCE_CONTENT}/{PACK}/{file}\");"
        )?;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("content.rs"), generated)?;

    let staged = out_dir
        .ancestors()
        .nth(3)
        .unwrap()
        .join(format!("{}-content", env::var("CARGO_PKG_NAME")?))
        .join("content");
    let _ = fs::remove_dir_all(&staged);
    fs::create_dir_all(staged.join(PACK))?;

    let addresses: Vec<String> = entries
        .iter()
        .map(|(_, file)| format!("content/{PACK}/{file}"))
        .collect();
    for (_, file) in &entries {
        fs::copy(
            source_root.join(PACK).join(file),
            staged.join(PACK).join(file),
        )?;
    }

    let registry = fs::read_to_string(source_root.join(".registry.toml"))?;
    let trimmed: String = registry
        .lines()
        .filter(|line| {
            !line.contains('=')
                || addresses
                    .iter()
                    .any(|address| line.ends_with(&format!("\"{address}\"")))
        })
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(staged.join(".registry.toml"), trimmed)?;

    Ok(())
}
