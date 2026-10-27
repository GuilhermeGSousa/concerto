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
    std::fs::write(base.join("content.toml"), "root = \"data\"\n").unwrap();
    let config = ContentConfig::load_or_default(&base).expect("parses");
    assert_eq!(config.root, "data");
    assert_eq!(config.extension, "gasset");
    std::fs::remove_dir_all(&base).ok();
}
