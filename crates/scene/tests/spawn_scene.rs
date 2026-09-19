//! Covers spawn_scene_components end-to-end: one entity per node, components
//! deserialized and applied from their JSON payloads, hierarchy wired, and
//! the spawner component removed so the scene expands exactly once. The
//! `spawner_expands_nodes_and_upgrades_weak_handles` case additionally
//! drives the real system with a wired `AssetServer`, so the Weak->Strong
//! mesh-handle upgrade in `MeshComponent::apply` is exercised for real.
use ecs::component::name::Name;
use ecs::component::Component;
use ecs::entity::hierarchy::{ChildOf, Children};
use ecs::{IntoSystem, Res, ResMut, Resource, System, World};
use essential::assets::asset_server::AssetServer;
use essential::assets::asset_store::AssetStore;
use essential::assets::handle::AssetHandle;
use essential::assets::AssetId;
use essential::transform::Transform;
use glam::Vec3;
use mesh::mesh::{Mesh, MeshComponent};
use render::components::camera::Camera;
use scene::scene::{Scene, SceneNode, SerializedComponent};
use scene::spawner::{spawn_scene, spawn_scene_components, SceneSpawnerComponent, SpawnedScene};

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
        data: serde_json::to_string(&transform).unwrap(),
    }
}

fn mesh_component(id: AssetId) -> SerializedComponent {
    SerializedComponent {
        type_name: MeshComponent::name().to_string(),
        data: serde_json::to_string(&MeshComponent {
            handle: AssetHandle::weak(id),
        })
        .unwrap(),
    }
}

#[derive(Resource)]
struct SceneFixture(Scene);

#[derive(Resource)]
struct SpawnParent(ecs::Entity);

#[derive(Resource, Default)]
struct SpawnOutput(Option<SpawnedScene>);

fn spawn_fixture_scene(
    mut cmd: ecs::command::CommandQueue,
    fixture: Res<SceneFixture>,
    parent: Res<SpawnParent>,
    mut output: ResMut<SpawnOutput>,
) {
    if output.0.is_none() {
        output.0 = Some(spawn_scene(&mut cmd, &fixture.0, parent.0));
    }
}

#[test]
fn reusable_spawn_returns_source_mapping_and_preserves_registered_components() {
    let mut world = World::default();
    world.register_component_type::<Transform>();
    world.register_component_type::<Camera>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();

    let parent = world.spawn(());
    let camera = SerializedComponent {
        type_name: Camera::name().to_string(),
        data: serde_json::to_string(&Camera::default()).unwrap(),
    };
    world.insert_resource(SceneFixture(Scene {
        nodes: vec![
            node("first root", vec![1], vec![transform_component(1.0)]),
            node("child", vec![], vec![camera]),
            node("second root", vec![], vec![transform_component(2.0)]),
        ],
        referenced_assets: vec![],
    }));
    world.insert_resource(SpawnParent(parent));
    world.insert_resource(SpawnOutput::default());

    let mut system = spawn_fixture_scene.into_system();
    system.initialize(&mut world);
    system.run_and_apply(&mut world);

    let output = world
        .get_resource::<SpawnOutput>()
        .unwrap()
        .0
        .as_ref()
        .unwrap();
    assert_eq!(output.node_entities.len(), 3);
    assert_eq!(
        output.root_entities,
        vec![output.node_entities[0], output.node_entities[2]]
    );
    assert!(world
        .get_component_for_entity::<Transform>(output.node_entities[0])
        .is_some());
    assert!(
        world
            .get_component_for_entity::<Camera>(output.node_entities[1])
            .is_some(),
        "the generic spawner preserves registered scene components"
    );
    assert_eq!(
        world
            .get_component_for_entity::<Children>(parent)
            .unwrap()
            .into_iter()
            .copied()
            .collect::<Vec<_>>(),
        output.root_entities
    );
}

#[test]
fn spawned_nodes_carry_their_authored_name() {
    let mut world = World::default();
    world.register_component_type::<Transform>();

    let parent = world.spawn(());
    world.insert_resource(SceneFixture(Scene {
        nodes: vec![
            node("Armature", vec![1], vec![transform_component(1.0)]),
            node("spine_01", vec![], vec![]),
        ],
        referenced_assets: vec![],
    }));
    world.insert_resource(SpawnParent(parent));
    world.insert_resource(SpawnOutput::default());

    let mut system = spawn_fixture_scene.into_system();
    system.initialize(&mut world);
    system.run_and_apply(&mut world);

    let entities = world
        .get_resource::<SpawnOutput>()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .node_entities
        .clone();

    // Without this the authored name dies with the scene file and a tree view
    // has nothing to show but entity indices.
    assert_eq!(
        world
            .get_component_for_entity::<Name>(entities[0])
            .map(Name::as_str),
        Some("Armature"),
        "a spawned node must carry the name its scene node was authored with"
    );
    assert_eq!(
        world
            .get_component_for_entity::<Name>(entities[1])
            .map(Name::as_str),
        Some("spine_01"),
        "nodes without components must be named too"
    );
}

#[test]
fn scene_round_trips_through_bincode_with_component_payloads() {
    let scene = Scene {
        nodes: vec![
            node("root", vec![1], vec![transform_component(0.0)]),
            node("child", vec![], vec![transform_component(5.0)]),
        ],
        referenced_assets: vec![],
    };

    let bytes = bincode::serialize(&scene).expect("Scene must serialize");
    let decoded: Scene = bincode::deserialize(&bytes).expect("Scene must round-trip");

    assert_eq!(
        decoded.nodes.len(),
        2,
        "both nodes must survive the round-trip"
    );
    assert_eq!(
        decoded.nodes[0].children,
        vec![1],
        "hierarchy indices must survive"
    );
    assert_eq!(
        decoded.nodes[1].components[0].type_name, "Transform",
        "component type names must survive"
    );

    let transform: Transform =
        serde_json::from_str(&decoded.nodes[1].components[0].data).expect("payload must parse");
    assert_eq!(
        transform.translation,
        Vec3::new(5.0, 0.0, 0.0),
        "the component payload must carry its real data"
    );
}

#[test]
fn unregistered_component_is_skipped_without_failing_the_node() {
    let mut world = World::default();
    world.register_component_type::<Transform>();
    let entity = world.spawn(());

    let applied_unknown =
        world.apply_scene_component("NotARegisteredType", "{}", entity, &[entity]);
    let applied_known = world.apply_scene_component(
        "Transform",
        &serde_json::to_string(&Transform::IDENTITY).unwrap(),
        entity,
        &[entity],
    );

    assert!(!applied_unknown, "an unknown component must be skipped");
    assert!(
        applied_known,
        "a known component on the same node must still apply"
    );
    assert!(
        world
            .get_component_for_entity::<Transform>(entity)
            .is_some(),
        "skipping one component must not abort the rest of the node"
    );
}

#[test]
fn spawner_expands_nodes_and_upgrades_weak_handles() {
    let mut world = World::default();
    world.register_component_type::<Transform>();
    world.register_component_type::<MeshComponent>();
    world.register_component::<SceneSpawnerComponent>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();

    // A wired AssetServer with the Mesh lifetime sender registered lets
    // `MeshComponent::apply` upgrade a serialized Weak handle to Strong exactly
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
    system.run_and_apply(&mut world);

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
    let (handle_id, is_strong, translation) = mesh_query
        .iter(&mut world)
        .map(|(mesh, transform)| {
            (
                mesh.handle.id(),
                matches!(mesh.handle, AssetHandle::Strong(..)),
                transform.translation,
            )
        })
        .next()
        .expect("the child node entity must carry the applied MeshComponent");

    assert!(
        is_strong,
        "the spawner must upgrade the serialized Weak mesh handle to Strong via the AssetServer"
    );
    assert_eq!(
        handle_id, mesh_id,
        "the upgraded handle must still resolve to the serialized AssetId"
    );
    assert_eq!(
        translation,
        Vec3::new(5.0, 0.0, 0.0),
        "the child node's Transform payload must be applied alongside its mesh"
    );
}
