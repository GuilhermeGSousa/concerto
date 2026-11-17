use std::path::{Path, PathBuf};

use concerto_asset_format::{
    parse_registry, scan_content, write_content_asset, write_content_asset_with_embedded,
    write_registry, AssetId, ContentAssetHeader, ContentConfig, EmbeddedIn, EmbeddedPayload,
    ImportProvenance, ScannedEntry,
};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("asset-format-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_asset(path: &Path, id: AssetId, kind: &str, provenance: Option<ImportProvenance>) {
    let header = ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: kind.to_string(),
        kind_version: 1,
        provenance,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
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
                kind_version: Some(1),
                provenance: None,
                embedded_in: None,
            },
            ScannedEntry {
                id: body,
                address: "content/hero/mesh_Body.gasset".into(),
                kind: "Mesh".into(),
                kind_version: Some(1),
                provenance,
                embedded_in: None,
            },
        ]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_registry_text_round_trips_without_the_kind_version() {
    let host = AssetId::new();
    let entries = vec![
        ScannedEntry {
            id: AssetId::new(),
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
            embedded_in: None,
        },
        ScannedEntry {
            id: host,
            address: "content/quoted \"b\".gasset".into(),
            kind: "Scene".into(),
            kind_version: Some(3),
            provenance: None,
            embedded_in: None,
        },
        ScannedEntry {
            id: AssetId::new(),
            address: "content/quoted \"b\".gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
            embedded_in: Some(EmbeddedIn {
                host,
                name: "mesh/\"0\"".into(),
            }),
        },
        ScannedEntry {
            id: AssetId::new(),
            address: "content/quoted \"b\".gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
            embedded_in: Some(EmbeddedIn {
                host,
                name: "mesh/1".into(),
            }),
        },
    ];

    let text = write_registry(&entries);
    let mut parsed = parse_registry(&text).expect("parses");
    parsed.sort_by_key(|entry| entry.id);
    let mut expected = entries;
    expected[1].kind_version = None;
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
            kind_version: None,
            provenance: Some(ImportProvenance {
                source: "assets/b.glb".into(),
                sub_asset: "b".into(),
            }),
            embedded_in: None,
        },
        ScannedEntry {
            id: ids[0],
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
            embedded_in: None,
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
fn parsing_refuses_the_legacy_address_only_form_and_two_ids_at_one_address() {
    for text in [
        format!(
            "[assets]\n{} = \"content/a.gasset\"\n",
            AssetId::new().simple_hex()
        ),
        format!(
            "[assets]\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n",
            AssetId::new().simple_hex(),
            AssetId::new().simple_hex(),
        ),
    ] {
        assert!(parse_registry(&text).is_err(), "{text}");
    }
}

enum Config {
    Loaded {
        extension: &'static str,
        default_scene: Option<&'static str>,
    },
    Refused,
}

struct ConfigCase {
    name: &'static str,
    toml: Option<&'static str>,
    expected: Config,
}

#[test]
fn content_toml_fills_its_defaults_and_refuses_a_root_or_extension_it_cannot_use() {
    let cases = [
        ConfigCase {
            name: "absent",
            toml: None,
            expected: Config::Loaded {
                extension: "gasset",
                default_scene: None,
            },
        },
        ConfigCase {
            name: "a default scene",
            toml: Some("default_scene = \"content/levels/forest.gasset\"\n"),
            expected: Config::Loaded {
                extension: "gasset",
                default_scene: Some("content/levels/forest.gasset"),
            },
        },
        ConfigCase {
            name: "another root",
            toml: Some("root = \"data\"\n"),
            expected: Config::Refused,
        },
        ConfigCase {
            name: "a dotted extension",
            toml: Some("extension = \".gasset\"\n"),
            expected: Config::Refused,
        },
    ];
    for case in cases {
        let base = temp_root("config");
        if let Some(toml) = case.toml {
            std::fs::write(base.join("content.toml"), toml).unwrap();
        }
        let loaded = ContentConfig::load_or_default(&base);
        std::fs::remove_dir_all(&base).ok();
        match case.expected {
            Config::Loaded {
                extension,
                default_scene,
            } => {
                let config = loaded.unwrap_or_else(|err| panic!("{}: {err:#}", case.name));
                assert_eq!(config.root, "content", "{}", case.name);
                assert_eq!(config.extension, extension, "{}", case.name);
                assert_eq!(
                    config.default_scene.as_deref(),
                    default_scene,
                    "{}",
                    case.name
                );
            }
            Config::Refused => assert!(loaded.is_err(), "{}", case.name),
        }
    }
}

struct Scan {
    base: PathBuf,
    dir: PathBuf,
    extension: &'static str,
}

enum Scanned {
    Listed(&'static [&'static str]),
    Refused,
}

struct Layout {
    name: &'static str,
    build: fn(&Path) -> Scan,
    expected: Scanned,
}

fn content(root: &Path) -> Scan {
    Scan {
        base: root.to_path_buf(),
        dir: root.join("content"),
        extension: "gasset",
    }
}

fn host(path: &Path, id: AssetId, embedded: AssetId) -> Vec<u8> {
    let header = |id, kind: &str, sub_asset: &str| ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: kind.to_string(),
        kind_version: 1,
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".into(),
            sub_asset: sub_asset.into(),
        }),
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let bytes = write_content_asset_with_embedded(
        &header(id, "Scene", "scene"),
        b"scene",
        &[EmbeddedPayload {
            header: header(embedded, "Mesh", "mesh/0"),
            payload: b"mesh payload",
            editor_only: None,
            derived_blob: None,
        }],
    )
    .unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

fn mesh(path: &Path) {
    write_asset(path, AssetId::new(), "Mesh", None);
}

#[cfg(unix)]
fn link(target: &Path, link: &Path) {
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn a_scan_lists_each_file_of_the_extension_once_under_the_first_path_it_walks() {
    let cases = [
        Layout {
            name: "only the given extension, under a non-canonical base",
            build: |root| {
                mesh(&root.join("data/a.asset"));
                mesh(&root.join("data/b.gasset"));
                Scan {
                    base: root.join("data/.."),
                    dir: root.join("data"),
                    extension: "asset",
                }
            },
            expected: Scanned::Listed(&["data/a.asset"]),
        },
        Layout {
            name: "a duplicate id",
            build: |root| {
                let id = AssetId::new();
                write_asset(&root.join("content/a.gasset"), id, "Mesh", None);
                write_asset(&root.join("content/b.gasset"), id, "Mesh", None);
                content(root)
            },
            expected: Scanned::Refused,
        },
        Layout {
            name: "a host, then its embedded assets",
            build: |root| {
                host(
                    &root.join("content/scene.gasset"),
                    AssetId::new(),
                    AssetId::new(),
                );
                content(root)
            },
            expected: Scanned::Listed(&["content/scene.gasset", "content/scene.gasset#mesh/0"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a symlinked content directory keeps content addresses",
            build: |root| {
                mesh(&root.join("outside/a.gasset"));
                link(&root.join("outside"), &root.join("base/content"));
                content(&root.join("base"))
            },
            expected: Scanned::Listed(&["content/a.gasset"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a symlink cycle",
            build: |root| {
                mesh(&root.join("content/a.gasset"));
                link(&root.join("content"), &root.join("content/sub/loop"));
                content(root)
            },
            expected: Scanned::Listed(&["content/a.gasset"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a directory reached through two paths",
            build: |root| {
                mesh(&root.join("content/real/a.gasset"));
                link(&root.join("content/real"), &root.join("content/alias"));
                content(root)
            },
            expected: Scanned::Listed(&["content/alias/a.gasset"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a dangling directory symlink is skipped",
            build: |root| {
                mesh(&root.join("content/a.gasset"));
                link(&root.join("missing"), &root.join("content/dangling"));
                content(root)
            },
            expected: Scanned::Listed(&["content/a.gasset"]),
        },
    ];
    for case in cases {
        let root = temp_root("layout");
        let scan = (case.build)(&root);
        let result = scan_content(&scan.base, &scan.dir, scan.extension);
        let root_text = root.to_string_lossy().replace('\\', "/");
        std::fs::remove_dir_all(&root).ok();
        match case.expected {
            Scanned::Listed(addresses) => {
                let entries = result.unwrap_or_else(|err| panic!("{}: {err:#}", case.name));
                assert_eq!(
                    entries
                        .iter()
                        .map(|entry| entry.location().replace(&root_text, "<root>"))
                        .collect::<Vec<_>>(),
                    addresses,
                    "{}",
                    case.name
                );
            }
            Scanned::Refused => assert!(result.is_err(), "{}", case.name),
        }
    }
}
