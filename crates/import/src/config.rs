use std::path::{Path, PathBuf};

pub use concerto_asset_format::ContentConfig;

/// Default address for a source relative to the project. A conventional leading
/// `assets/` directory is stripped. Absolute external sources use their stem.
pub fn content_address(config: &ContentConfig, source: &Path, sub_name: &str) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    // A '#' is fragile in an address that doubles as a filesystem path and a
    // cross-reference string (it reads as a fragment separator in URL-shaped
    // contexts, and needs escaping in others); '\\' would split the join on
    // Windows.
    let sanitized = sub_name.replace(['/', '\\', '#'], "_");
    let relative = source.strip_prefix("assets").unwrap_or(source);
    let parent = if relative.is_absolute() {
        Path::new("")
    } else {
        relative.parent().unwrap_or(Path::new(""))
    };
    Path::new(&config.root)
        .join(parent)
        .join(stem)
        .join(format!("{sanitized}.{}", config.extension))
        .to_string_lossy()
        .replace('\\', "/")
}

/// The project root a config path implies (its containing directory).
pub fn project_root_of(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_in_a_sub_asset_name_is_sanitized_out_of_the_address() {
        let address = content_address(
            &ContentConfig::default(),
            Path::new("hero.obj"),
            "material/steel#main",
        );
        assert!(!address.contains('#'), "got: {address}");
    }
}
