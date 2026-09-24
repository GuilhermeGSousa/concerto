//! Imported UUIDs survive UUID loading, handle serialization, and content moves.
use std::{path::PathBuf, sync::Arc, time::Duration};

use concerto_ecs::World;
use concerto_foundation::assets::{
    asset_server::{handle_asset_load_events, AssetServer},
    asset_store::AssetStore,
    content::read_content_asset_header,
    handle::AssetHandle,
    AssetId, ContentAssetRoot,
};
use concerto_mesh::mesh::Mesh;

struct Project(PathBuf);

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn runtime(root: &Project) -> (AssetServer, World) {
    let mut server = AssetServer::with_content_root(ContentAssetRoot::Directory(root.0.clone()));
    let store = AssetStore::<Mesh>::new();
    server.register_asset(&store);
    pollster::block_on(server.initialize()).expect("initialize registry");
    let mut world = World::new();
    world.insert_resource(store);
    world.insert_resource(server.clone());
    (server, world)
}

fn wait_for_mesh(world: &mut World, handle: &AssetHandle<Mesh>) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        handle_asset_load_events(world);
        if let Some(mesh) = world
            .get_resource::<AssetStore<Mesh>>()
            .unwrap()
            .get(handle)
        {
            assert_eq!(mesh.vertices.len(), 3);
            return;
        }
        assert!(std::time::Instant::now() < deadline, "mesh load timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn imported_uuid_handle_resolves_after_serialization_and_content_move() {
    let project = Project(
        std::env::temp_dir().join(format!("import-runtime-{}", AssetId::new().simple_hex())),
    );
    std::fs::create_dir_all(project.0.join("assets")).unwrap();
    let source = project.0.join("assets/triangle.gltf");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../gltf-loader/tests/fixtures/triangle.gltf"),
        &source,
    )
    .unwrap();
    let imported =
        concerto_import::import_source(&source, &project.0, &Default::default()).unwrap();
    let address = &imported
        .iter()
        .find(|a| a.sub_asset_name == "mesh/0")
        .unwrap()
        .address;
    let id = read_content_asset_header(&project.0.join(address))
        .unwrap()
        .asset_id;
    assert_ne!(id, AssetId::from_path(address));

    let serialized = {
        let (server, mut world) = runtime(&project);
        let handle = server.load::<Mesh>(id);
        assert_eq!(handle.id(), id);
        wait_for_mesh(&mut world, &handle);
        bincode::serialize(&handle).unwrap()
    };

    let moved_address = "content/relocated_mesh.gasset";
    std::fs::rename(project.0.join(address), project.0.join(moved_address)).unwrap();
    concerto_import::import_source(&source, &project.0, &Default::default()).unwrap();
    assert!(!project.0.join(address).exists());

    let restored: AssetHandle<Mesh> = bincode::deserialize(&serialized).unwrap();
    let (server, mut world) = runtime(&project);
    let by_id = server.load::<Mesh>(restored.id());
    let repeated = server.load::<Mesh>(id);
    assert_eq!(repeated.id(), id);
    match (&by_id, &repeated) {
        (AssetHandle::Strong(a, _), AssetHandle::Strong(b, _)) => assert!(Arc::ptr_eq(a, b)),
        _ => panic!("both loads must return the same strong handle"),
    }
    wait_for_mesh(&mut world, &by_id);
    assert_eq!(
        world
            .get_resource::<AssetStore<Mesh>>()
            .unwrap()
            .into_iter()
            .count(),
        1
    );
}
