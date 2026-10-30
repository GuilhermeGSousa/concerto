use std::path::Path;

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
    let sanitized = cap_file_name(&sanitized, sub_name, &config.extension);
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

const MAX_FILE_NAME_BYTES: usize = 200;

fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

fn cap_file_name(sanitized: &str, original: &str, extension: &str) -> String {
    if sanitized.len() + 1 + extension.len() <= MAX_FILE_NAME_BYTES {
        return sanitized.to_string();
    }
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(extension.len() + 1 + 9);
    let mut end = budget.min(sanitized.len());
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}_{:08x}", &sanitized[..end], fnv1a32(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_name(address: &str) -> &str {
        address.rsplit('/').next().unwrap()
    }

    fn address_for(name: &str) -> String {
        content_address(&ContentConfig::default(), Path::new("hero.obj"), name)
    }

    #[test]
    fn a_short_name_keeps_its_address() {
        assert_eq!(
            address_for("material_steel"),
            "content/hero/material_steel.gasset"
        );
    }

    #[test]
    fn a_file_name_of_exactly_the_cap_is_unchanged() {
        let name = "a".repeat(200 - ".gasset".len());
        let address = address_for(&name);
        assert_eq!(file_name(&address), format!("{name}.gasset"));
        assert_eq!(file_name(&address).len(), 200);
    }

    #[test]
    fn a_file_name_one_byte_over_the_cap_is_capped() {
        let name = "a".repeat(200 - ".gasset".len() + 1);
        let address = address_for(&name);
        assert!(file_name(&address).len() <= 200, "got: {address}");
        assert_ne!(file_name(&address), format!("{name}.gasset"));
    }

    #[test]
    fn a_long_name_is_capped_and_keeps_its_extension() {
        let address = address_for(&"a".repeat(300));
        assert!(file_name(&address).len() <= 200, "got: {address}");
        assert!(file_name(&address).ends_with(".gasset"), "got: {address}");
    }

    #[test]
    fn long_names_differing_in_the_last_character_stay_distinct() {
        let prefix = "a".repeat(299);
        assert_ne!(
            address_for(&format!("{prefix}x")),
            address_for(&format!("{prefix}y"))
        );
    }

    #[test]
    fn the_same_long_name_gives_the_same_address() {
        let name = "b".repeat(300);
        assert_eq!(address_for(&name), address_for(&name));
    }

    #[test]
    fn a_long_multi_byte_name_is_cut_on_a_char_boundary() {
        let address = address_for(&format!("aa{}", "あ".repeat(100)));
        let name = file_name(&address);
        assert_eq!(name.len(), 198, "got: {address}");
        assert!(name.ends_with(".gasset"), "got: {address}");
    }

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
