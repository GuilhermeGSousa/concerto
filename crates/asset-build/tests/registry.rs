#![cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use concerto_asset_build::write_registry_into;
use concerto_asset_format::{
    parse_registry, write_content_asset, AssetId, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};

fn write_asset(path: &Path, id: AssetId, kind: &str) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: vec![],
        kind: kind.into(),
        provenance: None,
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, b"payload").unwrap()).unwrap();
}

fn written(out: &Path) -> Vec<(AssetId, String, String)> {
    let text = std::fs::read_to_string(out.join("concerto_registry.toml")).unwrap();
    let mut entries: Vec<_> = parse_registry(&text)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.id, entry.address, entry.kind))
        .collect();
    entries.sort();
    entries
}

#[test]
fn writes_the_registry_of_the_content_tree() {
    let project = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mesh = AssetId::new();
    let scene = AssetId::new();
    write_asset(
        &project.path().join("content/hero/body.gasset"),
        mesh,
        "Mesh",
    );
    write_asset(&project.path().join("content/level.gasset"), scene, "Scene");
    std::fs::write(project.path().join("content/notes.txt"), "ignored").unwrap();

    write_registry_into(project.path(), out.path()).unwrap();

    let mut expected = vec![
        (
            mesh,
            "content/hero/body.gasset".to_string(),
            "Mesh".to_string(),
        ),
        (
            scene,
            "content/level.gasset".to_string(),
            "Scene".to_string(),
        ),
    ];
    expected.sort();
    assert_eq!(written(out.path()), expected);
}

#[test]
fn honours_the_root_and_extension_in_content_toml() {
    let project = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("content.toml"),
        "root = \"data\"\nextension = \"blob\"\n",
    )
    .unwrap();
    let kept = AssetId::new();
    write_asset(&project.path().join("data/hero.blob"), kept, "Mesh");
    write_asset(
        &project.path().join("data/other.gasset"),
        AssetId::new(),
        "Mesh",
    );
    write_asset(
        &project.path().join("content/hero.gasset"),
        AssetId::new(),
        "Mesh",
    );

    write_registry_into(project.path(), out.path()).unwrap();

    assert_eq!(
        written(out.path()),
        vec![(kept, "data/hero.blob".to_string(), "Mesh".to_string())]
    );
}

#[test]
fn fails_on_a_duplicate_id() {
    let project = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let id = AssetId::new();
    write_asset(&project.path().join("content/a.gasset"), id, "Mesh");
    write_asset(&project.path().join("content/b.gasset"), id, "Mesh");

    let error = format!(
        "{:#}",
        write_registry_into(project.path(), out.path()).unwrap_err()
    );

    assert!(error.contains("content/a.gasset"), "{error}");
    assert!(error.contains("content/b.gasset"), "{error}");
}

#[test]
fn a_moved_file_is_listed_at_its_new_address_on_the_next_run() {
    let project = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let id = AssetId::new();
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
        vec![(
            id,
            "content/new/hero.gasset".to_string(),
            "Mesh".to_string()
        )]
    );
}
