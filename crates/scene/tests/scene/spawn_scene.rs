use concerto_ecs::component::Component;
use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::World;
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::assets::AssetId;
use concerto_foundation::transform::Transform;
use concerto_mesh::mesh::{Mesh, MeshComponent};
use concerto_reflect::to_value;
use concerto_scene::node_id::NodeId;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};
use concerto_scene::spawner::{spawn_scene_in_world, SpawnFilter};
use glam::Vec3;

fn node(name: &str, children: Vec<usize>, components: Vec<SerializedComponent>) -> SceneNode {
    SceneNode {
        id: NodeId::from_name_path(name),
        name: name.to_string(),
        children,
        components,
        link: None,
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
fn spawning_a_scene_applies_its_nodes_and_upgrades_weak_handles() {
    let mut world = World::default();
    world.register_type::<Transform>();
    world.register_type::<MeshComponent>();
    world.register_type::<AssetHandle<Mesh>>();
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
    let parent = world.spawn(Transform::IDENTITY);
    spawn_scene_in_world(&mut world, &scene, parent, &SpawnFilter::default());

    let child_count = world
        .get_component_for_entity::<Children>(parent)
        .map(|children| children.into_iter().count())
        .unwrap_or(0);
    assert_eq!(
        child_count, 1,
        "only the single root node is parented to the parent"
    );

    let mut transforms = world.query::<&Transform, ()>();
    assert_eq!(
        transforms.iter(&mut world).count(),
        3,
        "one Transform per node plus the parent's own"
    );

    let mut mesh_query = world.query::<(&MeshComponent, &Transform), ()>();
    let (mesh, transform) = mesh_query
        .iter(&mut world)
        .next()
        .expect("the child node entity must carry the applied MeshComponent");

    assert!(
        matches!(mesh.handle, AssetHandle::Strong(..)),
        "spawning must upgrade the serialized Weak mesh handle to Strong via the AssetServer"
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
