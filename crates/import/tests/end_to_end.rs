//! The full import loop: import writes content assets, the runtime byte
//! path reads them back as the right type, and an editor-saved Scene
//! round-trips.
//!
//! These drive `load_content_asset_bytes` — the exact helper every
//! `AssetServer` calls — rather than asynchronous UUID loading, because completing
//! an AssetServer load needs the LoadTaskPool plus a World to pump
//! `handle_asset_load_events`, and no such harness exists in the test
//! suite today. This covers the same code path minus the task-pool wrapper.
use std::path::{Path, PathBuf};

use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, save_content_asset, AssetRegistry,
};
use concerto_foundation::assets::utils::load_content_asset_bytes;
use concerto_foundation::assets::{Asset, ContentAssetRoot};
use concerto_mesh::mesh::Mesh;
use concerto_scene::scene::{Scene, SceneNode};

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
fn imported_content_assets_load_back_as_their_type() {
    let root = std::env::temp_dir().join(format!("content-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    concerto_import::import_source(&fixture(&root), &root, &Default::default()).expect("import");

    let address = "content/triangle/mesh_0.gasset";
    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(root.clone()),
        address,
        Mesh::name(),
    ))
    .expect("the imported content asset loads");

    let mesh: Mesh = bincode::deserialize(&bytes).expect("payload is a Mesh");
    assert_eq!(
        mesh.primitives[0].vertices.len(),
        3,
        "the triangle fixture's mesh survives import -> load"
    );

    let err = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(root.clone()),
        address,
        Scene::name(),
    ))
    .expect_err("kind tag must reject a Scene load of a Mesh file");
    assert!(format!("{err:#}").contains("Mesh"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn an_editor_saved_scene_round_trips() {
    let root = std::env::temp_dir().join(format!("content-save-e2e-{}", std::process::id()));
    let address = "content/levels/intro.gasset";

    let scene = Scene {
        nodes: vec![SceneNode {
            name: "root".to_string(),
            children: Vec::new(),
            components: Vec::new(),
        }],
        referenced_assets: Vec::new(),
    };
    save_content_asset(&scene, &root, address).expect("save");

    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(root.clone()),
        address,
        Scene::name(),
    ))
    .expect("an editor-saved asset loads through the runtime path");
    let restored: Scene = bincode::deserialize(&bytes).expect("payload is a Scene");
    assert_eq!(restored.nodes.len(), scene.nodes.len());

    let raw = std::fs::read(root.join(address)).unwrap();
    let (header, _) = read_content_asset(&raw).expect("readable");
    assert_eq!(header.kind, Scene::name());
    assert_eq!(
        header.provenance, None,
        "an editor save is not import-derived"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_baked_reference_resolves_to_the_minted_id_of_its_target() {
    let root = std::env::temp_dir().join(format!("content-minted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let written = concerto_import::import_source(&fixture(&root), &root, &Default::default())
        .expect("import");

    let scene_address = &written
        .iter()
        .find(|a| a.sub_asset_name == "scene")
        .expect("the fixture emits a scene")
        .address;
    let mesh_address = &written
        .iter()
        .find(|a| a.sub_asset_name.starts_with("mesh/"))
        .expect("the fixture emits a mesh")
        .address;

    // Load the scene by address, exactly as game code would.
    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(root.clone()),
        scene_address,
        Scene::name(),
    ))
    .expect("the scene loads by address");
    let scene: Scene = bincode::deserialize(&bytes).expect("payload is a Scene");

    // The id it references is the mesh's *minted* id — not derivable from
    // either address — and the registry is what connects the two.
    let referenced = scene
        .referenced_assets
        .first()
        .copied()
        .expect("the scene references its mesh");
    let registry = AssetRegistry::load(&root).expect("registry loads");
    assert_eq!(
        registry.get(referenced),
        Some(mesh_address.as_str()),
        "a baked reference must resolve through the registry to the file it names"
    );
    assert_eq!(
        read_content_asset_header(&root.join(mesh_address))
            .unwrap()
            .asset_id,
        referenced,
        "and that file's own header must carry the same id"
    );

    std::fs::remove_dir_all(&root).ok();
}
