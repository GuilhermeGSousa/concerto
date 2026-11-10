use std::path::{Path, PathBuf};

use concerto_asset_format::{
    parse_registry, scan_content, write_content_asset, write_registry, AssetId, ContentAssetHeader,
    ContentConfig, ImportProvenance, ScannedEntry,
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
        derived: None,
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
            },
            ScannedEntry {
                id: body,
                address: "content/hero/mesh_Body.gasset".into(),
                kind: "Mesh".into(),
                kind_version: Some(1),
                provenance,
            },
        ]
    );

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_registry_text_round_trips_without_the_kind_version() {
    let entries = vec![
        ScannedEntry {
            id: AssetId::new(),
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
            provenance: None,
        },
        ScannedEntry {
            id: AssetId::new(),
            address: "content/quoted \"b\".gasset".into(),
            kind: "Scene".into(),
            kind_version: Some(3),
            provenance: None,
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
        },
        ScannedEntry {
            id: ids[0],
            address: "content/a.gasset".into(),
            kind: "Mesh".into(),
            kind_version: None,
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

struct RegistryRefusal {
    text: String,
    names: &'static str,
}

#[test]
fn parsing_refuses_the_legacy_address_only_form_and_two_ids_at_one_address() {
    for case in [
        RegistryRefusal {
            text: format!(
                "[assets]\n{} = \"content/a.gasset\"\n",
                AssetId::new().simple_hex()
            ),
            names: "rebuild",
        },
        RegistryRefusal {
            text: format!(
                "[assets]\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n",
                AssetId::new().simple_hex(),
                AssetId::new().simple_hex(),
            ),
            names: "content/a.gasset",
        },
    ] {
        let err = parse_registry(&case.text).expect_err("refused");
        assert!(format!("{err:#}").contains(case.names), "got: {err:#}");
    }
}

enum Config {
    Loaded {
        extension: &'static str,
        default_scene: Option<&'static str>,
    },
    Refused(&'static str),
}

struct ConfigCase {
    name: &'static str,
    toml: Option<&'static str>,
    expected: Config,
}

#[test]
fn content_toml_fills_its_defaults_and_refuses_a_root_or_extension_it_cannot_use() {
    let root = "content root must be 'content'";
    let extension = "content extension must be a nonempty extension";
    let refused = |name, toml, message| ConfigCase {
        name,
        toml: Some(toml),
        expected: Config::Refused(message),
    };
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
            name: "a missing field takes the default",
            toml: Some("extension = \"bin\"\n"),
            expected: Config::Loaded {
                extension: "bin",
                default_scene: None,
            },
        },
        ConfigCase {
            name: "the content root with a custom extension",
            toml: Some("root = \"content\"\nextension = \"bin\"\n"),
            expected: Config::Loaded {
                extension: "bin",
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
        refused("another root", "root = \"data\"\n", root),
        refused("a parent root", "root = \"../x\"\n", root),
        refused("an empty root", "root = \"\"\n", root),
        refused("an absolute root", "root = \"/abs\"\n", root),
        refused("an empty extension", "extension = \"\"\n", extension),
        refused("a dotted extension", "extension = \".gasset\"\n", extension),
        refused(
            "an extension with a slash",
            "extension = \"a/b\"\n",
            extension,
        ),
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
            Config::Refused(message) => {
                let err = loaded
                    .err()
                    .unwrap_or_else(|| panic!("{} loaded", case.name));
                assert!(
                    format!("{err:#}").contains(message),
                    "{}: {err:#}",
                    case.name
                );
            }
        }
    }

    let base = temp_root("config-unreadable");
    std::fs::create_dir(base.join("content.toml")).unwrap();
    assert!(ContentConfig::load_or_default(&base).is_err());
    std::fs::remove_dir_all(&base).ok();
}

struct Scan {
    base: PathBuf,
    dir: PathBuf,
    extension: &'static str,
}

enum Scanned {
    Listed(&'static [&'static str]),
    Refused(&'static [&'static str]),
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
            name: "an absent directory",
            build: content,
            expected: Scanned::Listed(&[]),
        },
        Layout {
            name: "a duplicate id names both addresses",
            build: |root| {
                let id = AssetId::new();
                write_asset(&root.join("content/a.gasset"), id, "Mesh", None);
                write_asset(&root.join("content/b.gasset"), id, "Mesh", None);
                content(root)
            },
            expected: Scanned::Refused(&["content/a.gasset", "content/b.gasset"]),
        },
        Layout {
            name: "an unreadable header names its path",
            build: |root| {
                std::fs::create_dir_all(root.join("content")).unwrap();
                std::fs::write(root.join("content/broken.gasset"), b"nope").unwrap();
                content(root)
            },
            expected: Scanned::Refused(&["broken.gasset"]),
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
            name: "two symlinks to one directory",
            build: |root| {
                mesh(&root.join("target/a.gasset"));
                link(&root.join("target"), &root.join("base/content/alias_a"));
                link(&root.join("target"), &root.join("base/content/alias_b"));
                content(&root.join("base"))
            },
            expected: Scanned::Listed(&["content/alias_a/a.gasset"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a directory outside the base keeps the walked path",
            build: |root| {
                std::fs::create_dir_all(root.join("base")).unwrap();
                mesh(&root.join("other/a.gasset"));
                Scan {
                    base: root.join("base"),
                    dir: root.join("other"),
                    extension: "gasset",
                }
            },
            expected: Scanned::Listed(&["<root>/other/a.gasset"]),
        },
        #[cfg(unix)]
        Layout {
            name: "a base with a parent after a symlink is normalised lexically",
            build: |root| {
                mesh(&root.join("elsewhere/a.gasset"));
                link(&root.join("elsewhere"), &root.join("base/link"));
                Scan {
                    base: root.join("base/link/.."),
                    dir: root.join("base/link"),
                    extension: "gasset",
                }
            },
            expected: Scanned::Listed(&["link/a.gasset"]),
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
                        .map(|entry| entry.address.replace(&root_text, "<root>"))
                        .collect::<Vec<_>>(),
                    addresses,
                    "{}",
                    case.name
                );
            }
            Scanned::Refused(named) => {
                let message = format!("{:#}", result.err().expect(case.name));
                for name in named {
                    assert!(message.contains(name), "{}: {message}", case.name);
                }
            }
        }
    }
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
