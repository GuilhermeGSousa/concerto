//! `import` turns a glTF into content assets whose cross-references address
//! content-tree paths, not `<source>#<sub>`.
use std::path::{Path, PathBuf};

use concerto_ecs::component::Component;
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, save_content_asset_into, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::utils::load_content_asset;
use concerto_foundation::assets::{Asset, AssetId, ContentAssetRoot};
use concerto_mesh::mesh::{Mesh, MeshComponent};
use concerto_scene::scene::Scene;

fn fixture(project_root: &Path) -> PathBuf {
    let destination = project_root.join("assets/triangle.gltf");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../gltf/tests/fixtures/triangle.gltf")
        .canonicalize()
        .expect("fixture exists");
    std::fs::copy(source, &destination).unwrap();
    destination
}

#[test]
fn an_imported_gltf_writes_content_assets_that_load_back_and_reference_each_other_by_minted_id() {
    let project_root = std::env::temp_dir().join(format!("import-gltf-{}", std::process::id()));
    std::fs::create_dir_all(&project_root).unwrap();

    let written =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("import succeeds");

    for expected in [
        "content/triangle/mesh_0.gasset",
        "content/triangle/scene.gasset",
    ] {
        assert!(
            written.iter().any(|a| a.address == expected),
            "expected {expected} among {written:?}"
        );
        assert!(project_root.join(expected).exists(), "{expected} on disk");
    }

    let raw = std::fs::read(project_root.join("content/triangle/scene.gasset")).unwrap();
    let (header, payload) = read_content_asset(&raw).expect("readable");
    assert_eq!(header.kind, Scene::name());
    assert_ne!(
        header.asset_id,
        AssetId::from_path("content/triangle/scene.gasset"),
        "identity is minted, not derived from the address"
    );
    assert_eq!(
        header.provenance,
        Some(ImportProvenance {
            source: "assets/triangle.gltf".to_string(),
            sub_asset: "scene".to_string(),
        }),
        "import must record where the content asset came from"
    );

    let scene: Scene = bincode::deserialize(payload).expect("scene payload");
    let mesh_id = read_content_asset_header(&project_root.join("content/triangle/mesh_0.gasset"))
        .expect("mesh header")
        .asset_id;
    let component = scene.nodes[0]
        .components
        .iter()
        .find(|c| c.type_name == MeshComponent::name())
        .expect("the node carries a MeshComponent");
    let mut mesh = MeshComponent::default();
    assert!(concerto_reflect::apply_value(&mut mesh, &component.value).is_empty());
    let referenced = mesh.handle.id();
    assert_eq!(
        referenced, mesh_id,
        "the MeshComponent must address the mesh's minted id, not triangle.gltf#mesh/0"
    );
    assert!(
        header.references.contains(&mesh_id),
        "header references list carries the mesh's minted id"
    );

    let content = ContentAssetRoot::Directory(project_root.clone());
    let mesh = pollster::block_on(load_content_asset::<Mesh>(
        &content,
        "content/triangle/mesh_0.gasset",
    ))
    .expect("the imported mesh loads through the runtime path");
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(
        read_content_asset_header(&project_root.join("content/triangle/mesh_0.gasset"))
            .unwrap()
            .kind_version,
        Mesh::FORMAT_VERSION
    );
    let err = pollster::block_on(load_content_asset::<Scene>(
        &content,
        "content/triangle/mesh_0.gasset",
    ))
    .expect_err("the kind tag rejects a Scene load of a Mesh file");
    assert!(format!("{err:#}").contains("Mesh"));
    let loaded = pollster::block_on(load_content_asset::<Scene>(
        &content,
        "content/triangle/scene.gasset",
    ))
    .expect("the scene loads by address");
    let registry =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("scan");
    assert_eq!(
        registry
            .get(loaded.referenced_assets[0])
            .map(|entry| entry.address.as_str()),
        Some("content/triangle/mesh_0.gasset"),
        "a baked reference resolves through the registry to the file it names"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn an_imports_returned_entries_bring_a_scanned_registry_up_to_date() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-registry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();
    save_content_asset_into(
        &Scene {
            nodes: Vec::new(),
            referenced_assets: Vec::new(),
        },
        &project_root,
        "content/authored.gasset",
        &mut AssetRegistry::new(),
    )
    .unwrap();
    let mut registry =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("scan");

    let written =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("import succeeds");
    for entry in &written {
        registry.insert(entry.clone());
    }

    assert!(!written.is_empty());
    for entry in &written {
        let provenance = entry
            .provenance
            .as_ref()
            .expect("an import records provenance");
        assert_eq!(provenance.source, "assets/triangle.gltf");
        assert_eq!(
            read_content_asset_header(&project_root.join(&entry.address))
                .unwrap()
                .provenance
                .as_ref(),
            Some(provenance)
        );
    }
    assert!(!project_root.join("content/.registry.toml").exists());
    let rescanned =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("rescan");
    assert_eq!(sorted(&registry), sorted(&rescanned));

    std::fs::remove_dir_all(&project_root).ok();
}

fn sorted(registry: &AssetRegistry) -> Vec<AssetEntry> {
    let mut entries: Vec<_> = registry.iter().cloned().collect();
    entries.sort_by(|a, b| a.address.cmp(&b.address));
    entries
}
