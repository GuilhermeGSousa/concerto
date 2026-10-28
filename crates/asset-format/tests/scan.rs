use std::path::{Path, PathBuf};

use concerto_asset_format::{
    parse_registry, scan_content, write_content_asset, write_registry, AssetId, ContentAssetHeader,
    ContentConfig, ImportProvenance, ScannedEntry, CONTENT_FORMAT_VERSION,
};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("asset-format-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_asset(path: &Path, id: AssetId, kind: &str, provenance: Option<ImportProvenance>) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: kind.to_string(),
        provenance,
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, b"payload").unwrap()).unwrap();
}

fn sorted(mut entries: Vec<ScannedEntry>) -> Vec<ScannedEntry> {
    entries.sort_by(|a, b| a.address.cmp(&b.address));
    entries
}

#[test]
fn scanning_a_tree_lists_every_file_of_the_extension_relative_to_the_base() {
    let base = temp_root("scan");
    let body = AssetId::new();
    let idle = AssetId::new();
    let provenance = Some(ImportProvenance {
        source: "assets/hero.glb".into(),
        sub_asset: "mesh/Body".into(),
    });
    write_asset(
        &base.join("content/hero/mesh_Body.gasset"),
        body,
        "Mesh",
        provenance.clone(),
    );
    write_asset(
        &base.join("content/hero/anim/idle.gasset"),
        idle,
        "AnimationClip",
        None,
    );
    std::fs::write(base.join("content/hero/notes.txt"), "not an asset").unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        sorted(entries),
        vec![
            ScannedEntry {
                id: idle,
                address: "content/hero/anim/idle.gasset".into(),
                kind: "AnimationClip".into(),
                provenance: None,
            },
            ScannedEntry {
                id: body,
                address: "content/hero/mesh_Body.gasset".into(),
                kind: "Mesh".into(),
                provenance,
            },
        ]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn scanning_honours_the_extension_and_a_non_canonical_base() {
    let base = temp_root("scan-extension");
    let id = AssetId::new();
    write_asset(&base.join("data/a.asset"), id, "Mesh", None);
    write_asset(&base.join("data/b.gasset"), AssetId::new(), "Mesh", None);

    let entries = scan_content(&base.join("data/.."), &base.join("data"), "asset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "data/a.asset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn scanning_an_absent_directory_lists_nothing() {
    let base = temp_root("scan-absent");
    let entries = scan_content(&base, &base.join("content"), "gasset").expect("not an error");
    assert!(entries.is_empty());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_duplicate_id_is_an_error_naming_both_addresses() {
    let base = temp_root("scan-dup");
    let id = AssetId::new();
    write_asset(&base.join("content/a.gasset"), id, "Mesh", None);
    write_asset(&base.join("content/b.gasset"), id, "Mesh", None);

    let err = scan_content(&base, &base.join("content"), "gasset").expect_err("duplicate id");
    let message = format!("{err:#}");
    assert!(
        message.contains("content/a.gasset") && message.contains("content/b.gasset"),
        "got: {message}"
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn an_unreadable_header_is_an_error_naming_its_path() {
    let base = temp_root("scan-unreadable");
    std::fs::create_dir_all(base.join("content")).unwrap();
    std::fs::write(base.join("content/broken.gasset"), b"nope").unwrap();

    let err = scan_content(&base, &base.join("content"), "gasset").expect_err("bad header");
    let message = format!("{err:#}");
    assert!(message.contains("broken.gasset"), "got: {message}");

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_registry_text_round_trips() {
    let entries = vec![
        ScannedEntry {
            id: AssetId::new(),
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        },
        ScannedEntry {
            id: AssetId::new(),
            address: "content/quoted \"b\".gasset".into(),
            kind: "Scene".into(),
            provenance: None,
        },
    ];

    let text = write_registry(&entries);
    let mut parsed = parse_registry(&text).expect("parses");
    parsed.sort_by_key(|entry| entry.id);
    let mut expected = entries;
    expected.sort_by_key(|entry| entry.id);
    assert_eq!(parsed, expected);
}

#[test]
fn writing_the_registry_drops_provenance_and_orders_by_id() {
    let mut ids = [AssetId::new(), AssetId::new()];
    ids.sort();
    let entries = vec![
        ScannedEntry {
            id: ids[1],
            address: "content/b.gasset".into(),
            kind: "Mesh".into(),
            provenance: Some(ImportProvenance {
                source: "assets/b.glb".into(),
                sub_asset: "b".into(),
            }),
        },
        ScannedEntry {
            id: ids[0],
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        },
    ];

    assert_eq!(
        write_registry(&entries),
        format!(
            "[assets]\n{} = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n{} = {{ address = \"content/b.gasset\", kind = \"Mesh\" }}\n",
            ids[0].simple_hex(),
            ids[1].simple_hex(),
        )
    );
}

#[test]
fn parsing_rejects_the_legacy_address_only_form() {
    let text = format!(
        "[assets]\n{} = \"content/a.gasset\"\n",
        AssetId::new().simple_hex()
    );
    let err = parse_registry(&text).expect_err("legacy form");
    assert!(format!("{err:#}").contains("rebuild"), "got: {err:#}");
}

#[test]
fn parsing_rejects_two_ids_at_one_address() {
    let text = format!(
        "[assets]\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n",
        AssetId::new().simple_hex(),
        AssetId::new().simple_hex(),
    );
    let err = parse_registry(&text).expect_err("duplicate address");
    assert!(
        format!("{err:#}").contains("content/a.gasset"),
        "got: {err:#}"
    );
}

#[test]
fn content_config_defaults_when_content_toml_is_absent() {
    let base = temp_root("config-absent");
    let config = ContentConfig::load_or_default(&base).expect("defaults");
    assert_eq!(config.root, "content");
    assert_eq!(config.extension, "gasset");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn content_config_fills_a_missing_field_from_the_default() {
    let base = temp_root("config-partial");
    std::fs::write(base.join("content.toml"), "extension = \"bin\"\n").unwrap();
    let config = ContentConfig::load_or_default(&base).expect("parses");
    assert_eq!(config.root, "content");
    assert_eq!(config.extension, "bin");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn content_config_rejects_a_root_other_than_content() {
    for (tag, root) in [
        ("data", "data"),
        ("parent", "../x"),
        ("empty", ""),
        ("absolute", "/abs"),
    ] {
        let base = temp_root(&format!("config-root-{tag}"));
        std::fs::write(base.join("content.toml"), format!("root = \"{root}\"\n")).unwrap();
        let err = ContentConfig::load_or_default(&base).expect_err("rejected");
        assert!(
            format!("{err:#}").contains("content root must be 'content'"),
            "root {root:?}: {err:#}"
        );
        std::fs::remove_dir_all(&base).ok();
    }
}

#[test]
fn content_config_rejects_a_bad_extension() {
    for (tag, extension) in [("empty", ""), ("dot", ".gasset"), ("slash", "a/b")] {
        let base = temp_root(&format!("config-ext-{tag}"));
        std::fs::write(
            base.join("content.toml"),
            format!("extension = \"{extension}\"\n"),
        )
        .unwrap();
        let err = ContentConfig::load_or_default(&base).expect_err("rejected");
        assert!(
            format!("{err:#}").contains("content extension must be a nonempty extension"),
            "extension {extension:?}: {err:#}"
        );
        std::fs::remove_dir_all(&base).ok();
    }
}

#[test]
fn content_config_accepts_a_custom_extension_with_the_default_root() {
    let base = temp_root("config-bin");
    std::fs::write(
        base.join("content.toml"),
        "root = \"content\"\nextension = \"bin\"\n",
    )
    .unwrap();
    let config = ContentConfig::load_or_default(&base).expect("accepted");
    assert_eq!(config.extension, "bin");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn validate_rejects_a_hand_built_bad_config() {
    let bad_root = ContentConfig {
        root: "data".to_string(),
        ..ContentConfig::default()
    };
    assert!(bad_root.validate().is_err());
    let bad_extension = ContentConfig {
        extension: "a.b".to_string(),
        ..ContentConfig::default()
    };
    assert!(bad_extension.validate().is_err());
    assert!(ContentConfig::default().validate().is_ok());
}

#[test]
fn an_unreadable_content_toml_is_an_error() {
    let base = temp_root("config-unreadable");
    std::fs::create_dir(base.join("content.toml")).unwrap();
    assert!(ContentConfig::load_or_default(&base).is_err());
    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn a_symlinked_content_directory_keeps_content_addresses() {
    let base = temp_root("scan-symlink-root");
    let outside = temp_root("scan-symlink-outside");
    let id = AssetId::new();
    write_asset(&outside.join("a.gasset"), id, "Mesh", None);
    std::os::unix::fs::symlink(&outside, base.join("content")).unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
    std::fs::remove_dir_all(&outside).ok();
}

#[cfg(unix)]
#[test]
fn a_symlink_cycle_lists_each_entry_once_and_terminates() {
    let base = temp_root("scan-cycle");
    let id = AssetId::new();
    write_asset(&base.join("content/a.gasset"), id, "Mesh", None);
    std::fs::create_dir_all(base.join("content/sub")).unwrap();
    std::os::unix::fs::symlink(base.join("content"), base.join("content/sub/loop")).unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn a_directory_reached_through_two_paths_is_listed_once_under_the_first_name() {
    let base = temp_root("scan-twice");
    let id = AssetId::new();
    write_asset(&base.join("content/real/a.gasset"), id, "Mesh", None);
    std::os::unix::fs::symlink(base.join("content/real"), base.join("content/alias")).unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "content/alias/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn a_directory_outside_the_base_keeps_the_walked_path_as_its_address() {
    let base = temp_root("scan-outside-base");
    let other = temp_root("scan-outside-dir");
    let id = AssetId::new();
    write_asset(&other.join("a.gasset"), id, "Mesh", None);

    let entries = scan_content(&base, &other, "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: other.join("a.gasset").to_string_lossy().replace('\\', "/"),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
    std::fs::remove_dir_all(&other).ok();
}

#[cfg(unix)]
#[test]
fn a_base_with_parent_after_a_symlink_is_normalised_lexically() {
    let base = temp_root("scan-lexical");
    let elsewhere = temp_root("scan-lexical-target");
    let id = AssetId::new();
    write_asset(&elsewhere.join("a.gasset"), id, "Mesh", None);
    std::os::unix::fs::symlink(&elsewhere, base.join("link")).unwrap();

    let entries = scan_content(&base.join("link/.."), &base.join("link"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "link/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
    std::fs::remove_dir_all(&elsewhere).ok();
}

#[cfg(unix)]
#[test]
fn a_dangling_directory_symlink_is_skipped() {
    let base = temp_root("scan-dangling");
    let id = AssetId::new();
    write_asset(&base.join("content/a.gasset"), id, "Mesh", None);
    std::os::unix::fs::symlink(base.join("missing"), base.join("content/dangling")).unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_an_error_naming_its_path() {
    use std::os::unix::fs::PermissionsExt;

    let base = temp_root("scan-denied");
    let denied = base.join("content/denied");
    std::fs::create_dir_all(&denied).unwrap();
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000)).unwrap();

    if std::fs::read_dir(&denied).is_ok() {
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&base).ok();
        return;
    }

    let result = scan_content(&base, &base.join("content"), "gasset");
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o755)).unwrap();

    let message = format!("{:#}", result.expect_err("unreadable directory"));
    assert!(message.contains("content/denied"), "got: {message}");

    std::fs::remove_dir_all(&base).ok();
}

#[cfg(unix)]
#[test]
fn two_symlinks_to_one_directory_list_it_once_under_the_first_name() {
    let base = temp_root("scan-two-links");
    let target = temp_root("scan-two-links-target");
    let id = AssetId::new();
    write_asset(&target.join("a.gasset"), id, "Mesh", None);
    std::fs::create_dir_all(base.join("content")).unwrap();
    std::os::unix::fs::symlink(&target, base.join("content/alias_a")).unwrap();
    std::os::unix::fs::symlink(&target, base.join("content/alias_b")).unwrap();

    let entries = scan_content(&base, &base.join("content"), "gasset").expect("scan");

    assert_eq!(
        entries,
        vec![ScannedEntry {
            id,
            address: "content/alias_a/a.gasset".into(),
            kind: "Mesh".into(),
            provenance: None,
        }]
    );

    std::fs::remove_dir_all(&base).ok();
    std::fs::remove_dir_all(&target).ok();
}
