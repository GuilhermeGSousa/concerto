//! `AssetRegistry` load/save/get/insert/remove/iter, and `save_content_asset`
//! upserting it.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, save_content_asset, write_content_asset,
    AssetRegistry, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("asset-registry-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn insert_get_remove_round_trip_in_memory() {
    let mut registry = AssetRegistry::new();
    let id = AssetId::from_path("content/hero/scene.gasset");
    assert_eq!(registry.get(id), None);

    registry.insert(id, "content/hero/scene.gasset");
    assert_eq!(registry.get(id), Some("content/hero/scene.gasset"));

    assert_eq!(
        registry.remove(id),
        Some("content/hero/scene.gasset".to_string())
    );
    assert_eq!(registry.get(id), None);
}

#[test]
fn save_then_load_round_trips_every_entry() {
    let dir = temp_root("save-load");
    let mut registry = AssetRegistry::new();
    let a = AssetId::from_path("content/a.gasset");
    let b = AssetId::from_path("content/b.gasset");
    registry.insert(a, "content/a.gasset");
    registry.insert(b, "content/b.gasset");
    registry.save(&dir).expect("save");

    let loaded = AssetRegistry::load(&dir).expect("load");
    assert_eq!(loaded.get(a), Some("content/a.gasset"));
    assert_eq!(loaded.get(b), Some("content/b.gasset"));
    assert_eq!(loaded.iter().count(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn loading_a_registry_that_was_never_saved_is_empty_not_an_error() {
    let dir = temp_root("never-saved");
    let registry = AssetRegistry::load(&dir).expect("an absent registry file is not an error");
    assert_eq!(registry.iter().count(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn registry_file_lands_under_content_and_is_keyed_by_simple_hex() {
    let dir = temp_root("on-disk-shape");
    let mut registry = AssetRegistry::new();
    let id = AssetId::from_path("content/hero/scene.gasset");
    registry.insert(id, "content/hero/scene.gasset");
    registry.save(&dir).expect("save");

    let text = std::fs::read_to_string(dir.join("content/.registry.toml")).expect("file exists");
    assert!(
        text.contains(&id.simple_hex()),
        "the registry must key entries by simple_hex(), got: {text}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[derive(Debug, Serialize, Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

#[test]
fn save_content_asset_upserts_the_registry() {
    let dir = temp_root("save-upserts");
    let address = "content/things/one.gasset";

    save_content_asset(&Thing, &dir, address).expect("save");

    let raw = std::fs::read(dir.join(address)).unwrap();
    let (header, _) = read_content_asset(&raw).expect("readable");

    let registry = AssetRegistry::load(&dir).expect("load");
    assert_eq!(
        registry.get(header.asset_id),
        Some(address),
        "save_content_asset must register the asset it just wrote"
    );

    assert_eq!(
        header.provenance, None,
        "an editor save is not import-derived"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_content_asset_header_does_not_read_the_payload() {
    let dir = temp_root("prefix-read");
    let address = "content/big/thing.gasset";
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Thing".to_string(),
        provenance: None,
    };
    // A payload far larger than the header, so a whole-file read would be
    // obviously wasteful and a prefix read is provably enough.
    let payload = vec![7u8; 4 * 1024 * 1024];
    std::fs::create_dir_all(dir.join("content/big")).unwrap();
    std::fs::write(
        dir.join(address),
        write_content_asset(&header, &payload).unwrap(),
    )
    .unwrap();

    let read = read_content_asset_header(&dir.join(address)).expect("header reads");
    assert_eq!(read, header, "the prefix read returns the whole header");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn id_for_address_is_the_inverse_of_get() {
    let mut registry = AssetRegistry::new();
    let id = AssetId::new();
    registry.insert(id, "content/hero/scene.gasset");

    assert_eq!(registry.get(id), Some("content/hero/scene.gasset"));
    assert_eq!(
        registry.id_for_address("content/hero/scene.gasset"),
        Some(id)
    );
    assert_eq!(registry.id_for_address("content/nope.gasset"), None);
}

#[test]
fn parse_rejects_two_ids_sharing_one_address() {
    let text = format!(
        "[assets]\n\"{}\" = \"content/a.gasset\"\n\"{}\" = \"content/a.gasset\"\n",
        AssetId::new().simple_hex(),
        AssetId::new().simple_hex(),
    );
    let err = AssetRegistry::parse(&text).expect_err("a duplicate address is malformed");
    assert!(
        format!("{err:#}").contains("content/a.gasset"),
        "the error must name the duplicated address, got: {err:#}"
    );
}

#[test]
fn from_content_tree_indexes_every_gasset_by_its_header_id() {
    let dir = temp_root("scan");
    let mut expected = Vec::new();
    for (sub, kind) in [("mesh/Body", "Mesh"), ("animation/Idle", "AnimationClip")] {
        let address = format!("content/hero/{}.gasset", sub.replace('/', "_"));
        let id = AssetId::new();
        let header = ContentAssetHeader {
            format_version: CONTENT_FORMAT_VERSION,
            asset_id: id,
            references: Vec::new(),
            kind: kind.to_string(),
            provenance: None,
        };
        std::fs::create_dir_all(dir.join("content/hero")).unwrap();
        std::fs::write(
            dir.join(&address),
            write_content_asset(&header, b"payload").unwrap(),
        )
        .unwrap();
        expected.push((id, address));
    }

    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").expect("scan");

    assert_eq!(registry.iter().count(), 2);
    for (id, address) in &expected {
        assert_eq!(
            registry.get(*id),
            Some(address.as_str()),
            "the scan keys each file by the id in its own header"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_repoints_a_moved_file_at_its_new_address() {
    let dir = temp_root("scan-moved");
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    let bytes = write_content_asset(&header, b"payload").unwrap();
    std::fs::write(dir.join("content/hero/old.gasset"), &bytes).unwrap();

    let before = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(before.get(id), Some("content/hero/old.gasset"));

    // The rename an artist (or a future editor) would do by hand.
    std::fs::rename(
        dir.join("content/hero/old.gasset"),
        dir.join("content/hero/new.gasset"),
    )
    .unwrap();

    let after = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(
        after.get(id),
        Some("content/hero/new.gasset"),
        "the id follows the file, which is what keeps baked references resolving"
    );
    assert_eq!(
        after.iter().count(),
        1,
        "the old address is gone, not merged"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_rejects_two_files_sharing_an_id() {
    let dir = temp_root("scan-dup-id");
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    let bytes = write_content_asset(&header, b"payload").unwrap();
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    std::fs::write(dir.join("content/hero/a.gasset"), &bytes).unwrap();
    std::fs::write(dir.join("content/hero/b.gasset"), &bytes).unwrap();

    let err = AssetRegistry::from_content_tree(&dir, "content", "gasset")
        .expect_err("a copy-pasted .gasset is a malformed tree");
    let message = format!("{err:#}");
    assert!(
        message.contains("a.gasset") && message.contains("b.gasset"),
        "the error must name both paths, got: {message}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_of_an_absent_tree_is_empty() {
    let dir = temp_root("scan-absent");
    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset")
        .expect("a project with no content tree yet is not an error");
    assert_eq!(registry.iter().count(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_content_asset_mints_an_id_not_derived_from_the_address() {
    let dir = temp_root("save-mints");
    let address = "content/things/one.gasset";

    save_content_asset(&Thing, &dir, address).expect("save");

    let raw = std::fs::read(dir.join(address)).unwrap();
    let (header, _) = read_content_asset(&raw).expect("readable");
    assert_ne!(
        header.asset_id,
        AssetId::from_path(address),
        "identity must be minted, not derived from where the file happens to sit"
    );
    assert_eq!(
        AssetRegistry::load(&dir).unwrap().get(header.asset_id),
        Some(address),
        "the registry points at the minted id"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_over_an_existing_asset_reuses_its_id() {
    let dir = temp_root("save-reuses");
    let address = "content/things/two.gasset";

    save_content_asset(&Thing, &dir, address).expect("first save");
    let first = read_content_asset_header(&dir.join(address))
        .unwrap()
        .asset_id;

    save_content_asset(&Thing, &dir, address).expect("second save");
    let second = read_content_asset_header(&dir.join(address))
        .unwrap()
        .asset_id;

    assert_eq!(
        first, second,
        "re-saving must keep the identity every existing reference names"
    );

    std::fs::remove_dir_all(&dir).ok();
}
