//! `import` turns a glTF into content assets whose cross-references address
//! content-tree paths, not `<source>#<sub>`.
use std::path::{Path, PathBuf};

use concerto_ecs::component::Component;
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, ImportProvenance,
};
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_foundation::assets::{Asset, AssetId};
use concerto_mesh::mesh::MeshComponent;
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
fn writes_content_assets_with_content_path_cross_references() {
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

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn import_upserts_the_registry_for_every_written_asset() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-registry-{}", std::process::id()));
    std::fs::create_dir_all(&project_root).unwrap();

    let written =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("import succeeds");

    let registry =
        AssetRegistry::from_content_tree(&project_root, "content", "gasset").expect("scan");
    for asset in &written {
        let id = read_content_asset_header(&project_root.join(&asset.address))
            .expect("header")
            .asset_id;
        assert_eq!(asset.asset_id, id);
        assert_eq!(
            registry
                .get(id)
                .map(|entry| (entry.address.as_str(), entry.kind.as_str())),
            Some((asset.address.as_str(), asset.kind.as_str())),
            "import must write every content asset it reports, with its kind"
        );
    }

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn re_importing_reuses_the_ids_already_on_disk() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-reuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    let first =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("first import");
    let ids_before: Vec<AssetId> = first.iter().map(|asset| asset.asset_id).collect();

    let second =
        concerto_import::import_source(&fixture(&project_root), &project_root, &Default::default())
            .expect("second import");
    let ids_after: Vec<AssetId> = second.iter().map(|asset| asset.asset_id).collect();

    assert_eq!(
        ids_before, ids_after,
        "a re-import must keep every id, or every baked cross-reference breaks"
    );

    std::fs::remove_dir_all(&project_root).ok();
}
