#![cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use concerto_asset_build::write_registry_into;
use concerto_asset_format::{parse_registry, write_content_asset, AssetId, ContentAssetHeader};

fn write_asset(path: &Path, id: AssetId, kind: &str) {
    let header = ContentAssetHeader {
        asset_id: id,
        references: vec![],
        kind: kind.into(),
        kind_version: 1,
        provenance: None,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, b"payload").unwrap()).unwrap();
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Listed {
    id: AssetId,
    address: String,
    kind: String,
}

fn listed(address: &str, kind: &str) -> Listed {
    Listed {
        id: AssetId::from_path(address),
        address: address.to_string(),
        kind: kind.to_string(),
    }
}

fn written(out: &Path) -> Vec<Listed> {
    let text = std::fs::read_to_string(out.join("concerto_registry.toml")).unwrap();
    let mut entries: Vec<_> = parse_registry(&text)
        .unwrap()
        .into_iter()
        .map(|entry| Listed {
            id: entry.id,
            address: entry.address,
            kind: entry.kind,
        })
        .collect();
    entries.sort();
    entries
}

struct File {
    address: &'static str,
    id_of: &'static str,
    kind: &'static str,
}

fn file(address: &'static str, kind: &'static str) -> File {
    File {
        address,
        id_of: address,
        kind,
    }
}

struct Tree {
    case: &'static str,
    files: Vec<File>,
    listed: Vec<Listed>,
    refused: bool,
}

#[test]
fn the_registry_lists_every_content_asset_or_refuses_an_id_at_two_addresses() {
    let cases = [
        Tree {
            case: "the default tree, with a file that is not content",
            files: vec![
                file("content/hero/body.gasset", "Mesh"),
                file("content/level.gasset", "Scene"),
                file("content/notes.txt", "Text"),
            ],
            listed: vec![
                listed("content/hero/body.gasset", "Mesh"),
                listed("content/level.gasset", "Scene"),
            ],
            refused: false,
        },
        Tree {
            case: "one id at two addresses",
            files: vec![
                File {
                    address: "content/a.gasset",
                    id_of: "shared",
                    kind: "Mesh",
                },
                File {
                    address: "content/b.gasset",
                    id_of: "shared",
                    kind: "Mesh",
                },
            ],
            listed: vec![],
            refused: true,
        },
    ];
    for case in &cases {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        for file in &case.files {
            let path = project.path().join(file.address);
            if file.address.ends_with(".txt") {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "ignored").unwrap();
            } else {
                write_asset(&path, AssetId::from_path(file.id_of), file.kind);
            }
        }

        let result = write_registry_into(project.path(), out.path());

        if !case.refused {
            result.unwrap_or_else(|error| panic!("{}: {error:#}", case.case));
            let mut expected = case.listed.clone();
            expected.sort();
            assert_eq!(written(out.path()), expected, "{}", case.case);
        } else {
            result.expect_err(case.case);
            assert!(
                !out.path().join("concerto_registry.toml").exists(),
                "{}",
                case.case
            );
        }
    }
}
