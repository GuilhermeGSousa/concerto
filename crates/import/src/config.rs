use std::path::Path;

use concerto_asset_format::sub_asset_file_stem;
pub use concerto_asset_format::ContentConfig;

/// Default address for a source relative to the project. A conventional leading
/// `assets/` directory is stripped. Absolute external sources use their stem.
pub fn content_address(config: &ContentConfig, source: &Path, sub_name: &str) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "asset".to_string());
    let sanitized = sub_asset_file_stem(sub_name, &config.extension);
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn file_name(address: &str) -> &str {
        address.rsplit('/').next().unwrap()
    }

    fn address_for(name: &str) -> String {
        content_address(&ContentConfig::default(), Path::new("hero.obj"), name)
    }

    struct Case {
        name: String,
        file_name: String,
    }

    #[test]
    fn a_sub_asset_name_becomes_a_sanitised_file_name_capped_on_a_char_boundary() {
        let at_cap = "a".repeat(200 - ".gasset".len());
        let multi_byte = format!("aa{}", "あ".repeat(100));
        let cases = [
            Case {
                name: "material_steel".to_string(),
                file_name: "material_steel.gasset".to_string(),
            },
            Case {
                name: "material/steel#main".to_string(),
                file_name: "material_steel_main.gasset".to_string(),
            },
            Case {
                name: at_cap.clone(),
                file_name: format!("{at_cap}.gasset"),
            },
            Case {
                name: multi_byte.clone(),
                file_name: format!("aa{}_70f83377.gasset", "あ".repeat(60)),
            },
        ];
        for case in &cases {
            assert_eq!(
                address_for(&case.name),
                format!("content/hero/{}", case.file_name)
            );
        }
        let prefix = "a".repeat(299);
        assert_ne!(
            address_for(&format!("{prefix}x")),
            address_for(&format!("{prefix}y"))
        );
    }

    proptest! {
        #[test]
        fn a_file_name_fits_the_cap_keeps_its_extension_and_names_no_directory(
            name in "[a-z/#\\\\あ]{0,300}",
        ) {
            let address = address_for(&name);
            let file = file_name(&address);
            prop_assert!(file.len() <= 200, "{}", address);
            prop_assert!(file.ends_with(".gasset"), "{}", address);
            prop_assert!(address.starts_with("content/hero/"), "{}", address);
            prop_assert!(!file.contains('#') && !file.contains('\\'), "{}", address);
            prop_assert_eq!(&address, &address_for(&name));
            let sanitized = name.replace(['/', '\\', '#'], "_");
            if sanitized.len() + ".gasset".len() <= 200 {
                prop_assert_eq!(file, format!("{sanitized}.gasset"));
            }
        }
    }
}
