use concerto_ecs::command::CommandQueue;
use concerto_ecs::component::name::Name;
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_ecs::component::Component;
use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::{Entity, IntoSystem, Res, Resource, System, World};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_foundation::transform::Transform;
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_render::components::camera::Camera;
use concerto_scene::capture::capture_scene;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};
use concerto_scene::spawner::spawn_scene;
use glam::Vec3;
use serde_json::Value;
use uuid::Uuid;

fn world() -> World {
    let mut world = World::default();
    world.register_component_type::<Transform>();
    world.register_component_type::<Camera>();
    world.register_component_type::<SkeletonComponent>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.register_component::<Name>();
    world
}

fn transform(x: f32) -> Transform {
    let mut transform = Transform::IDENTITY;
    transform.translation = Vec3::new(x, 0.0, 0.0);
    transform
}

fn node(name: &str, children: Vec<usize>, components: Vec<SerializedComponent>) -> SceneNode {
    SceneNode {
        name: name.into(),
        children,
        components,
    }
}

fn serialized<T: concerto_ecs::component::scene::SceneComponent>(value: &T) -> SerializedComponent {
    SerializedComponent {
        type_name: T::name().into(),
        data: serde_json::to_string(value).unwrap(),
    }
}

fn comparable(scene: &Scene) -> Vec<(String, Vec<usize>, Vec<(String, Value)>)> {
    scene
        .nodes
        .iter()
        .map(|node| {
            (
                node.name.clone(),
                node.children.clone(),
                node.components
                    .iter()
                    .map(|component| {
                        (
                            component.type_name.clone(),
                            serde_json::from_str(&component.data).unwrap(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

#[derive(Resource)]
struct Fixture(Scene, Entity);

fn spawn_fixture(mut cmd: CommandQueue, fixture: Res<Fixture>) {
    spawn_scene(&mut cmd, &fixture.0, fixture.1);
}

fn spawn(world: &mut World, scene: Scene) -> Entity {
    let root = world.spawn(Transform::IDENTITY);
    world.insert_resource(Fixture(scene, root));
    let mut system = spawn_fixture.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
    root
}

#[test]
fn a_spawned_scene_captures_back_to_what_was_spawned() {
    let mut world = world();
    let mut components = vec![serialized(&Camera::default()), serialized(&transform(3.0))];
    components.sort_by(|a, b| a.type_name.cmp(&b.type_name));
    let scene = Scene {
        nodes: vec![
            node("first root", vec![1, 2], vec![serialized(&transform(1.0))]),
            node("child", vec![], components),
            node("sibling", vec![], vec![]),
            node("second root", vec![], vec![serialized(&transform(2.0))]),
        ],
        referenced_assets: vec![],
    };
    let root = spawn(&mut world, scene.clone());

    let captured = capture_scene(&world, root).unwrap();

    assert_eq!(comparable(&captured), comparable(&scene));
    assert!(captured.referenced_assets.is_empty());
}

#[test]
fn the_root_is_not_a_node_and_unnamed_entities_get_an_empty_name() {
    let mut world = world();
    let root = world.spawn((Transform::IDENTITY, Name::new("wrapper")));
    let child = world.spawn(transform(5.0));
    world.entity_mut(root).add_child(child);

    let captured = capture_scene(&world, root).unwrap();

    assert_eq!(captured.nodes.len(), 1);
    assert_eq!(captured.nodes[0].name, "");
    assert_eq!(captured.nodes[0].components[0].type_name, Transform::name());
}

#[test]
fn resolved_skeleton_references_capture_as_node_indices() {
    let mut world = world();
    let root = world.spawn(());
    let mesh = world.spawn(Name::new("mesh"));
    let bone_a = world.spawn(Name::new("a"));
    let bone_b = world.spawn(Name::new("b"));
    world.entity_mut(root).add_child(bone_a);
    world.entity_mut(bone_a).add_child(bone_b);
    world.entity_mut(root).add_child(mesh);
    world.insert(
        SkeletonComponent {
            skeleton: AssetHandle::<Skeleton>::weak(AssetId::from_path("rig#skeleton")),
            bones: vec![SceneEntityRef::Entity(bone_b), SceneEntityRef::Entity(bone_a)],
            bone_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2)],
            root: Some(SceneEntityRef::Entity(bone_a)),
        },
        mesh,
    );

    let captured = capture_scene(&world, root).unwrap();

    let names: Vec<_> = captured.nodes.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "mesh"]);
    let skeleton: SkeletonComponent =
        serde_json::from_str(&captured.nodes[2].components[0].data).unwrap();
    assert_eq!(
        skeleton.bones,
        [SceneEntityRef::Index(1), SceneEntityRef::Index(0)]
    );
    assert_eq!(skeleton.root, Some(SceneEntityRef::Index(0)));
}

#[test]
fn a_component_that_cannot_be_read_back_fails_the_capture() {
    let mut world = world();
    let root = world.spawn(());
    let outsider = world.spawn(());
    let mesh = world.spawn(Name::new("hero"));
    world.entity_mut(root).add_child(mesh);
    world.insert(
        SkeletonComponent {
            skeleton: AssetHandle::<Skeleton>::weak(AssetId::from_path("rig#skeleton")),
            bones: vec![SceneEntityRef::Entity(outsider)],
            bone_ids: vec![Uuid::from_u128(1)],
            root: None,
        },
        mesh,
    );

    let error = capture_scene(&world, root).unwrap_err().to_string();

    assert!(error.contains("hero"), "{error}");
    assert!(error.contains("SkeletonComponent"), "{error}");
}

#[test]
fn two_captures_of_the_same_world_are_identical() {
    let mut world = world();
    let root = world.spawn(());
    for index in 0..4 {
        let child = world.spawn((transform(index as f32), Camera::default()));
        world.entity_mut(root).add_child(child);
    }

    let first = bincode::serialize(&capture_scene(&world, root).unwrap()).unwrap();
    let second = bincode::serialize(&capture_scene(&world, root).unwrap()).unwrap();

    assert_eq!(first, second);
}
