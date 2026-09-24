//! `utils::load_registry` — the async, ContentAssetRoot-aware counterpart to
//! `AssetRegistry::load` that `AssetServer::resolve_by_id` uses at runtime.
use concerto_core::assets::content::AssetRegistry;
use concerto_core::assets::utils::load_registry;
use concerto_core::assets::{AssetId, ContentAssetRoot};

#[test]
fn loads_a_registry_written_by_asset_registry_save() {
    let dir = std::env::temp_dir().join(format!("registry-loading-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let id = AssetId::from_path("content/hero/scene.gasset");
    let mut registry = AssetRegistry::new();
    registry.insert(id, "content/hero/scene.gasset");
    registry.save(&dir).expect("save");

    let loaded =
        pollster::block_on(load_registry(&ContentAssetRoot::Directory(dir.clone()))).expect("load");
    assert_eq!(loaded.get(id), Some("content/hero/scene.gasset"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_absent_registry_file_is_an_empty_registry() {
    let dir = std::env::temp_dir().join(format!("registry-loading-absent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let loaded = pollster::block_on(load_registry(&ContentAssetRoot::Directory(dir.clone())))
        .expect("an absent registry file is not an error");
    assert_eq!(loaded.iter().count(), 0);

    std::fs::remove_dir_all(&dir).ok();
}
