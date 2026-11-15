//! `AssetRegistry` get/insert/remove/iter and scanning.
use concerto_foundation::assets::content::{
    write_content_asset, write_content_asset_with_embedded, ContentAssetHeader, EmbeddedPayload,
    ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry, EmbeddedIn};
use concerto_foundation::assets::AssetId;
use proptest::prelude::*;

fn entry(id: AssetId, address: &str, kind: &str) -> AssetEntry {
    AssetEntry {
        id,
        address: address.into(),
        kind: kind.into(),
        kind_version: None,
        provenance: None,
        embedded_in: None,
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
            embedded: Vec::new(),
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

fn header(id: AssetId, kind: &str, sub_asset: &str) -> ContentAssetHeader {
    ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: kind.to_string(),
        kind_version: 1,
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".into(),
            sub_asset: sub_asset.into(),
        }),
        derived: None,
        embedded: Vec::new(),
    }
}

fn as_registry_text_holds(entry: &AssetEntry) -> AssetEntry {
    AssetEntry {
        kind_version: None,
        provenance: None,
        ..entry.clone()
    }
}

#[test]
fn a_host_and_its_embedded_assets_are_listed_and_found() {
    let dir = temp_root("embedded");
    let [loose, host, mesh, material] = [(); 4].map(|_| AssetId::new());
    let scene = "content/hero/scene.gasset";
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    std::fs::write(
        dir.join("content/hero/crate.gasset"),
        write_content_asset(&header(loose, "Mesh", "mesh/crate"), b"loose").unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join(scene),
        write_content_asset_with_embedded(
            &header(host, "Scene", "scene"),
            b"scene",
            &[
                EmbeddedPayload {
                    header: header(mesh, "Mesh", "mesh/0"),
                    payload: b"mesh",
                    derived_blob: None,
                },
                EmbeddedPayload {
                    header: header(material, "StandardMaterial", "material/steel"),
                    payload: b"material",
                    derived_blob: None,
                },
            ],
        )
        .unwrap(),
    )
    .unwrap();

    let mut registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").expect("scan");
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(registry.iter().count(), 4);
    assert_eq!(registry.id_for_address(scene), Some(host));
    assert_eq!(
        registry.id_for_location("content/hero/scene.gasset#mesh/0"),
        Some(mesh)
    );
    assert_eq!(registry.id_for_location(scene), Some(host));
    let held = registry.get(mesh).expect("the mesh is listed");
    assert_eq!(held.address, scene);
    assert_eq!(
        held.embedded_in,
        Some(EmbeddedIn {
            host,
            name: "mesh/0".into()
        })
    );
    assert_eq!(held.file(), host, "an embedded asset's file is its host's");
    assert_eq!(registry.get(host).expect("host").file(), host);
    assert_eq!(held.location(), "content/hero/scene.gasset#mesh/0");
    assert_eq!(
        held.name(),
        "mesh_0",
        "an embedded asset is named as its loose file was"
    );
    assert_eq!(registry.get(host).expect("host").location(), scene);
    let in_file: Vec<_> = registry.in_file(host).map(|entry| entry.id).collect();
    assert_eq!(in_file, [host, material, mesh], "the host comes first");

    let text = registry.to_toml();
    let parsed = AssetRegistry::parse(&text).expect("the registry text parses");
    assert_eq!(
        parsed.iter().cloned().collect::<Vec<_>>(),
        registry
            .iter()
            .map(as_registry_text_holds)
            .collect::<Vec<_>>(),
        "got: {text}"
    );

    let replacement = AssetId::new();
    registry.insert(AssetEntry {
        embedded_in: Some(EmbeddedIn {
            host,
            name: "mesh/0".into(),
        }),
        ..entry(replacement, scene, "Mesh")
    });
    assert_eq!(registry.get(mesh), None, "one asset per location");
    assert_eq!(
        registry.id_for_location("content/hero/scene.gasset#mesh/0"),
        Some(replacement)
    );
    assert_eq!(registry.id_for_address(scene), Some(host));
    assert_eq!(registry.iter().count(), 4);

    assert_eq!(registry.remove(host).map(|entry| entry.id), Some(host));
    assert_eq!(
        registry.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [loose],
        "removing a host removes what its file embeds"
    );

    let hostless = AssetRegistry::parse(&format!(
        "[assets]\n{} = {{ address = \"content/a.gasset\", kind = \"Mesh\", embedded_as = \"mesh/0\" }}\n",
        mesh.simple_hex()
    ))
    .expect_err("an embedded line needs the line of its host");
    assert!(
        format!("{hostless:#}").contains("content/a.gasset"),
        "got: {hostless:#}"
    );

    let before = AssetRegistry::parse(&format!(
        "[assets]\n{} = {{ address = \"content/a.gasset\", kind = \"Mesh\" }}\n",
        loose.simple_hex()
    ))
    .expect("text written before embedded assets parses");
    assert_eq!(
        before.get(loose),
        Some(&entry(loose, "content/a.gasset", "Mesh"))
    );
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
                    .then(a.name().cmp(&b.name()))
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
