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
    content_toml: Option<&'static str>,
    files: Vec<File>,
    listed: Vec<Listed>,
    refusal: &'static [&'static str],
}

#[test]
fn the_registry_lists_every_content_asset_of_the_configured_extension_or_names_the_problem() {
    let cases = [
        Tree {
            case: "the default tree, with a file that is not content",
            content_toml: None,
            files: vec![
                file("content/hero/body.gasset", "Mesh"),
                file("content/level.gasset", "Scene"),
                file("content/notes.txt", "Text"),
            ],
            listed: vec![
                listed("content/hero/body.gasset", "Mesh"),
                listed("content/level.gasset", "Scene"),
            ],
            refusal: &[],
        },
        Tree {
            case: "an extension set in content.toml",
            content_toml: Some("extension = \"blob\"\n"),
            files: vec![
                file("content/hero.blob", "Mesh"),
                file("content/other.gasset", "Mesh"),
            ],
            listed: vec![listed("content/hero.blob", "Mesh")],
            refusal: &[],
        },
        Tree {
            case: "a root outside the project",
            content_toml: Some("root = \"../x\"\n"),
            files: vec![],
            listed: vec![],
            refusal: &["content root must be 'content'"],
        },
        Tree {
            case: "one id at two addresses",
            content_toml: None,
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
            refusal: &["content/a.gasset", "content/b.gasset"],
        },
    ];
    for case in &cases {
        let project = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        if let Some(text) = case.content_toml {
            std::fs::write(project.path().join("content.toml"), text).unwrap();
        }
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

        if case.refusal.is_empty() {
            result.unwrap_or_else(|error| panic!("{}: {error:#}", case.case));
            let mut expected = case.listed.clone();
            expected.sort();
            assert_eq!(written(out.path()), expected, "{}", case.case);
        } else {
            let error = format!("{:#}", result.expect_err(case.case));
            for part in case.refusal {
                assert!(error.contains(part), "{}: {error}", case.case);
            }
            assert!(
                !out.path().join("concerto_registry.toml").exists(),
                "{}",
                case.case
            );
        }
    }
}

#[test]
fn a_moved_file_is_listed_at_its_new_address_on_the_next_run() {
    let project = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let id = AssetId::from_path("content/new/hero.gasset");
    write_asset(&project.path().join("content/old/hero.gasset"), id, "Mesh");
    write_registry_into(project.path(), out.path()).unwrap();

    std::fs::create_dir_all(project.path().join("content/new")).unwrap();
    std::fs::rename(
        project.path().join("content/old/hero.gasset"),
        project.path().join("content/new/hero.gasset"),
    )
    .unwrap();
    write_registry_into(project.path(), out.path()).unwrap();

    assert_eq!(
        written(out.path()),
        vec![listed("content/new/hero.gasset", "Mesh")]
    );
}
