//! Covers spawn_scene_components end-to-end: one entity per node, components
//! deserialized and applied from their JSON payloads, hierarchy wired, and
//! the spawner component removed so the scene expands exactly once. The
//! `spawner_expands_nodes_and_upgrades_weak_handles` case additionally
//! drives the real system with a wired `AssetServer`, so the Weak->Strong
//! mesh-handle upgrade in `MeshComponent::apply` is exercised for real.
use concerto_ecs::component::Component;
use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::{IntoSystem, System, World};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::assets::AssetId;
use concerto_foundation::transform::Transform;
use concerto_mesh::mesh::{Mesh, MeshComponent};
use concerto_reflect::to_value;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};
use concerto_scene::spawner::{spawn_scene_components, SceneSpawnerComponent};
use glam::Vec3;

fn node(name: &str, children: Vec<usize>, components: Vec<SerializedComponent>) -> SceneNode {
    SceneNode {
        name: name.to_string(),
        children,
        components,
    }
}

fn transform_component(x: f32) -> SerializedComponent {
    let mut transform = Transform::IDENTITY;
    transform.translation = Vec3::new(x, 0.0, 0.0);
    SerializedComponent {
        type_name: "Transform".to_string(),
        value: to_value(&transform).unwrap(),
    }
}

fn mesh_component(id: AssetId) -> SerializedComponent {
    SerializedComponent {
        type_name: MeshComponent::name().to_string(),
        value: to_value(&MeshComponent {
            handle: AssetHandle::weak(id),
        })
        .unwrap(),
    }
}

#[test]
fn spawner_expands_nodes_and_upgrades_weak_handles() {
    let mut world = World::default();
    world.register_type::<Transform>();
    world.register_type::<MeshComponent>();
    world.register_type::<AssetHandle<Mesh>>();
    world.register_component::<SceneSpawnerComponent>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();

    // A wired AssetServer with the Mesh lifetime sender registered lets
    // the handle's scene type data upgrade a serialized Weak handle to Strong exactly
    // as it does at runtime, instead of degrading gracefully.
    let mesh_store = AssetStore::<Mesh>::new();
    let mut asset_server = AssetServer::new();
    asset_server.register_asset::<Mesh>(&mesh_store);
    world.insert_resource(mesh_store);
    world.insert_resource(asset_server);

    let scene_id = AssetId::from_path("fixture.scene");
    let mesh_id = AssetId::from_path("fixture.gltf#mesh/0");
    let scene = Scene {
        nodes: vec![
            node("root", vec![1], vec![transform_component(0.0)]),
            node(
                "child",
                vec![],
                vec![transform_component(5.0), mesh_component(mesh_id)],
            ),
        ],
        referenced_assets: vec![mesh_id],
    };
    let mut scene_store = AssetStore::<Scene>::new();
    scene_store.insert(scene_id, scene);
    world.insert_resource(scene_store);

    let spawner = world.spawn(SceneSpawnerComponent(AssetHandle::weak(scene_id)));

    let mut system = spawn_scene_components.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);

    assert!(
        world
            .get_component_for_entity::<SceneSpawnerComponent>(spawner)
            .is_none(),
        "the spawner component must be removed so the scene expands exactly once"
    );

    let child_count = world
        .get_component_for_entity::<Children>(spawner)
        .map(|children| children.into_iter().count())
        .unwrap_or(0);
    assert_eq!(
        child_count, 1,
        "only the single root node is parented to the spawner"
    );

    let mut transforms = world.query::<&Transform, ()>();
    assert_eq!(
        transforms.iter(&mut world).count(),
        3,
        "one Transform per node plus the spawner's own inserted identity Transform"
    );

    let mut mesh_query = world.query::<(&MeshComponent, &Transform), ()>();
    let (mesh, transform) = mesh_query
        .iter(&mut world)
        .next()
        .expect("the child node entity must carry the applied MeshComponent");

    assert!(
        matches!(mesh.handle, AssetHandle::Strong(..)),
        "the spawner must upgrade the serialized Weak mesh handle to Strong via the AssetServer"
    );
    assert_eq!(
        mesh.handle.id(),
        mesh_id,
        "the upgraded handle must still resolve to the serialized AssetId"
    );
    assert_eq!(
        transform.translation,
        Vec3::new(5.0, 0.0, 0.0),
        "the child node's Transform payload must be applied alongside its mesh"
    );
}
