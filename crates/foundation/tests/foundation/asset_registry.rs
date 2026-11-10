//! `AssetRegistry` get/insert/remove/iter and scanning.
use concerto_foundation::assets::content::{
    write_content_asset, ContentAssetHeader, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{Asset, AssetId};
use proptest::prelude::*;
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

#[derive(Debug, Serialize, Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

#[test]
fn from_content_tree_indexes_every_gasset_by_its_header_id_and_follows_a_moved_file() {
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
            derived: None,
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

    let moved = expected[0].id;
    std::fs::rename(
        dir.join("content/hero/mesh_Body.gasset"),
        dir.join("content/hero/renamed.gasset"),
    )
    .unwrap();
    let after = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(
        address_of(&after, moved),
        Some("content/hero/renamed.gasset"),
        "the id follows the file, which is what keeps baked references resolving"
    );
    assert_eq!(
        after.iter().count(),
        2,
        "the old address is gone, not merged"
    );

    std::fs::remove_dir_all(&dir).ok();
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

#[derive(Debug, Clone)]
enum Edit {
    Insert {
        id: usize,
        address: usize,
        kind: usize,
    },
    Remove {
        id: usize,
    },
}

const ADDRESSES: [&str; 5] = [
    "content/a.gasset",
    "content/b/x.gasset",
    "content/b/a.gasset",
    "content/a/z.mesh.gasset",
    "loose.gasset",
];

const KINDS: [&str; 2] = ["Mesh", "Thing"];

prop_compose! {
    fn insert()(id in 0..4usize, address in 0..ADDRESSES.len(), kind in 0..KINDS.len()) -> Edit {
        Edit::Insert { id, address, kind }
    }
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![insert(), (0..4usize).prop_map(|id| Edit::Remove { id })]
}

proptest! {
    #[test]
    fn each_id_and_each_address_name_one_entry_listed_by_folder_then_name(
        edits in proptest::collection::vec(edit(), 0..24)
    ) {
        let ids = [(); 4].map(|_| AssetId::new());
        let mut registry = AssetRegistry::new();
        let mut model: Vec<AssetEntry> = Vec::new();
        for edit in edits {
            match edit {
                Edit::Insert { id, address, kind } => {
                    let inserted = entry(ids[id], ADDRESSES[address], KINDS[kind]);
                    model.retain(|held| held.id != inserted.id && held.address != inserted.address);
                    model.push(inserted.clone());
                    registry.insert(inserted);
                }
                Edit::Remove { id } => {
                    let held = model.iter().position(|held| held.id == ids[id]);
                    prop_assert_eq!(registry.remove(ids[id]), held.map(|at| model.remove(at)));
                }
            }
            model.sort_by(|a, b| {
                a.folder()
                    .cmp(b.folder())
                    .then(a.name().cmp(b.name()))
                    .then(a.kind.cmp(&b.kind))
                    .then(a.address.cmp(&b.address))
            });
            prop_assert_eq!(registry.iter().cloned().collect::<Vec<_>>(), model.clone());
            for id in ids {
                let held = model.iter().find(|held| held.id == id);
                prop_assert_eq!(registry.get(id), held);
            }
            for address in ADDRESSES {
                let held = model.iter().find(|held| held.address == address);
                prop_assert_eq!(registry.id_for_address(address), held.map(|held| held.id));
            }
        }
    }
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
