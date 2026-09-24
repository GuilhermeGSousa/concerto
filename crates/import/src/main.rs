use std::path::PathBuf;

use anyhow::bail;

use concerto_import::config::{project_root_of, ContentConfig};
use concerto_import::import_source;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let mut args = std::env::args().skip(1);
    let mut source: Option<PathBuf> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut extension: Option<String> = None;
    let mut content_root: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = args.next().map(PathBuf::from),
            "--ext" => extension = args.next(),
            "--content-root" => content_root = args.next(),
            other if other.starts_with("--") => bail!("unknown flag '{other}'"),
            other => source = Some(PathBuf::from(other)),
        }
    }

    let Some(source) = source else {
        eprintln!(
            "usage: concerto-import <source> [--config <content.toml>] [--ext <ext>] [--content-root <dir>]"
        );
        std::process::exit(2);
    };

    // An explicit `--config <file>` loads that exact file (erroring if it is
    // missing) and its parent is the project root; the implicit case reads
    // `content.toml` from the current directory, or the built-in defaults.
    let (project_root, mut config) = match config_path {
        Some(path) => {
            let config = ContentConfig::load(&path)?;
            (project_root_of(&path), config)
        }
        None => {
            let root = PathBuf::from(".");
            let config = ContentConfig::load_or_default(&root)?;
            (root, config)
        }
    };
    if let Some(extension) = extension {
        config.extension = extension;
    }
    if let Some(root) = content_root {
        config.root = root;
    }

    let written = import_source(&source, &project_root, &config)?;
    println!("imported {} -> {} assets", source.display(), written.len());
    for asset in &written {
        println!("  {} -> {}", asset.sub_asset_name, asset.address);
    }
    Ok(())
}
