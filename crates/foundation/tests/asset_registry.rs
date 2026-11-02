//! `AssetRegistry` get/insert/remove/iter and scanning, and `save_content_asset_into`
//! minting and reusing ids.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, save_content_asset_into, write_content_asset,
    ContentAssetHeader, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

fn entry(id: AssetId, address: &str, kind: &str) -> AssetEntry {
    AssetEntry {
        id,
        address: address.into(),
        kind: kind.into(),
        kind_version: None,
        provenance: None,
    }
}

fn address_of(registry: &AssetRegistry, id: AssetId) -> Option<&str> {
    registry.get(id).map(|entry| entry.address.as_str())
}

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

    registry.insert(entry(id, "content/hero/scene.gasset", "Scene"));
    assert_eq!(
        registry.get(id),
        Some(&entry(id, "content/hero/scene.gasset", "Scene"))
    );

    assert_eq!(
        registry.remove(id),
        Some(entry(id, "content/hero/scene.gasset", "Scene"))
    );
    assert_eq!(registry.get(id), None);
    assert_eq!(registry.id_for_address("content/hero/scene.gasset"), None);
}

#[derive(Debug, Serialize, Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

#[test]
fn read_content_asset_header_does_not_read_the_payload() {
    let dir = temp_root("prefix-read");
    let address = "content/big/thing.gasset";
    let id = AssetId::new();
    let header = ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: "Thing".to_string(),
        kind_version: 1,
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
    registry.insert(entry(id, "content/hero/scene.gasset", "Scene"));

    assert_eq!(address_of(&registry, id), Some("content/hero/scene.gasset"));
    assert_eq!(
        registry.id_for_address("content/hero/scene.gasset"),
        Some(id)
    );
    assert_eq!(registry.id_for_address("content/nope.gasset"), None);
}

#[test]
fn parse_rejects_two_ids_sharing_one_address() {
    let text = format!(
        "[assets]\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n\"{}\" = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n",
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
        let provenance = (kind == "Mesh").then(|| ImportProvenance {
            source: "assets/hero.glb".into(),
            sub_asset: sub.into(),
        });
        let header = ContentAssetHeader {
            asset_id: id,
            references: Vec::new(),
            kind: kind.to_string(),
            kind_version: 1,
            provenance: provenance.clone(),
        };
        std::fs::create_dir_all(dir.join("content/hero")).unwrap();
        std::fs::write(
            dir.join(&address),
            write_content_asset(&header, b"payload").unwrap(),
        )
        .unwrap();
        expected.push(AssetEntry {
            kind_version: Some(1),
            provenance,
            ..entry(id, &address, kind)
        });
    }

    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").expect("scan");

    assert_eq!(registry.iter().count(), 2);
    for expected in &expected {
        assert_eq!(
            registry.get(expected.id),
            Some(expected),
            "the scan keys each file by the id in its own header and keeps its kind and provenance"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_repoints_a_moved_file_at_its_new_address() {
    let dir = temp_root("scan-moved");
    let id = AssetId::new();
    let header = ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        kind_version: 1,
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    let bytes = write_content_asset(&header, b"payload").unwrap();
    std::fs::write(dir.join("content/hero/old.gasset"), &bytes).unwrap();

    let before = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(address_of(&before, id), Some("content/hero/old.gasset"));

    // The rename an artist (or a future editor) would do by hand.
    std::fs::rename(
        dir.join("content/hero/old.gasset"),
        dir.join("content/hero/new.gasset"),
    )
    .unwrap();

    let after = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(
        address_of(&after, id),
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
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        kind_version: 1,
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
fn save_content_asset_into_mints_an_id_not_derived_from_the_address() {
    let dir = temp_root("save-mints");
    let address = "content/things/one.gasset";
    let mut registry = AssetRegistry::new();

    save_content_asset_into(&Thing, &dir, address, &mut registry).expect("save");

    let raw = std::fs::read(dir.join(address)).unwrap();
    let (header, _) = read_content_asset(&raw).expect("readable");
    assert_ne!(
        header.asset_id,
        AssetId::from_path(address),
        "identity must be minted, not derived from where the file happens to sit"
    );
    assert_eq!(
        address_of(&registry, header.asset_id),
        Some(address),
        "the registry points at the minted id"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_over_an_existing_asset_reuses_its_id() {
    let dir = temp_root("save-reuses");
    let address = "content/things/two.gasset";

    let mut registry = AssetRegistry::new();
    save_content_asset_into(&Thing, &dir, address, &mut registry).expect("first save");
    let first = read_content_asset_header(&dir.join(address))
        .unwrap()
        .asset_id;

    save_content_asset_into(&Thing, &dir, address, &mut registry).expect("second save");
    let second = read_content_asset_header(&dir.join(address))
        .unwrap()
        .asset_id;

    assert_eq!(
        first, second,
        "re-saving must keep the identity every existing reference names"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn parse_rejects_the_old_format_and_says_how_to_rebuild() {
    let text = format!(
        "[assets]\n{} = \"content/a.gasset\"\n",
        AssetId::new().simple_hex()
    );
    let err = AssetRegistry::parse(&text).expect_err("the address-only format is gone");
    assert!(
        format!("{err:#}").contains("rebuild"),
        "the error must say how to rebuild the registry, got: {err:#}"
    );
}

#[derive(Debug, Serialize, Deserialize)]
struct Mesh;

impl Asset for Mesh {
    fn name() -> &'static str {
        "Mesh"
    }
}

#[test]
fn of_type_lists_only_that_kind_ordered_by_folder_then_name() {
    let mut registry = AssetRegistry::new();
    let [a, b, c, d] = [(); 4].map(|_| AssetId::new());
    registry.insert(entry(a, "content/z/rock.gasset", "Mesh"));
    registry.insert(entry(b, "content/a/tree.gasset", "Mesh"));
    registry.insert(entry(c, "content/a/level.gasset", "Thing"));
    registry.insert(entry(d, "content/a/bush.gasset", "Mesh"));

    let meshes: Vec<_> = registry.of_type::<Mesh>().map(|entry| entry.id).collect();
    assert_eq!(meshes, [d, b, a]);
    let things: Vec<_> = registry.of_type::<Thing>().map(|entry| entry.id).collect();
    assert_eq!(things, [c]);
    let all: Vec<_> = registry.iter().map(|entry| entry.id).collect();
    assert_eq!(all, [d, c, b, a]);
}

#[test]
fn inserting_an_existing_id_moves_it_and_frees_its_old_address() {
    let mut registry = AssetRegistry::new();
    let id = AssetId::new();
    let other = AssetId::new();
    registry.insert(entry(id, "content/c.gasset", "Mesh"));
    registry.insert(entry(other, "content/b.gasset", "Mesh"));
    registry.insert(entry(id, "content/a.gasset", "Mesh"));

    assert_eq!(registry.id_for_address("content/c.gasset"), None);
    assert_eq!(registry.id_for_address("content/a.gasset"), Some(id));
    let order: Vec<_> = registry.iter().map(|entry| entry.id).collect();
    assert_eq!(order, [id, other]);
}

#[test]
fn inserting_a_new_id_at_a_held_address_replaces_the_entry_there() {
    let mut registry = AssetRegistry::new();
    let old = AssetId::new();
    let new = AssetId::new();
    let other = AssetId::new();
    registry.insert(entry(old, "content/a.gasset", "Mesh"));
    registry.insert(entry(other, "content/b.gasset", "Mesh"));
    registry.insert(entry(new, "content/a.gasset", "Mesh"));

    assert_eq!(registry.get(old), None);
    assert_eq!(registry.id_for_address("content/a.gasset"), Some(new));
    let order: Vec<_> = registry.iter().map(|entry| entry.id).collect();
    assert_eq!(order, [new, other]);

    registry.remove(new);
    assert_eq!(registry.id_for_address("content/a.gasset"), None);
    let order: Vec<_> = registry.iter().map(|entry| entry.id).collect();
    assert_eq!(order, [other]);
}

#[test]
fn an_entry_names_itself_and_its_folder_from_its_address() {
    let nested = entry(AssetId::new(), "content/hero/body.mesh.gasset", "Mesh");
    assert_eq!(nested.name(), "body.mesh");
    assert_eq!(nested.folder(), "content/hero");
    assert!(nested.is::<Mesh>());
    assert!(!nested.is::<Thing>());

    let top = entry(AssetId::new(), "loose.gasset", "Mesh");
    assert_eq!(top.name(), "loose");
    assert_eq!(top.folder(), "");
}
