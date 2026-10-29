use concerto_ecs::component::scene::SceneEntityRef;
use concerto_ecs::{Entity, World};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_reflect::{to_value, ReflectedValue};

fn authored_skeleton(root: Option<SceneEntityRef>) -> SkeletonComponent {
    SkeletonComponent {
        skeleton: AssetHandle::<Skeleton>::weak(AssetId::from_path("rig.gltf#skeleton/0")),
        bones: vec![SceneEntityRef::Index(1), SceneEntityRef::Index(2)],
        root,
    }
}

fn spawn(world: &mut World, skeleton: SkeletonComponent, owner: Entity, nodes: &[Entity]) {
    world.register_type::<SkeletonComponent>();
    assert!(world.apply_scene_component(
        "SkeletonComponent",
        &to_value(&skeleton).unwrap(),
        owner,
        nodes,
    ));
}

#[test]
fn skeleton_component_resolves_scene_references_in_place() {
    let mut world = World::default();
    let owner = world.spawn(());
    let bone_a = world.spawn(());
    let bone_b = world.spawn(());
    let nodes = [owner, bone_a, bone_b];

    spawn(
        &mut world,
        authored_skeleton(Some(SceneEntityRef::Index(1))),
        owner,
        &nodes,
    );

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .expect("spawning must insert the same component type");
    assert_eq!(
        skeleton.bones(),
        &[
            SceneEntityRef::Entity(bone_a),
            SceneEntityRef::Entity(bone_b),
        ]
    );
    assert_eq!(skeleton.root(), Some(bone_a));
}

#[test]
fn rootless_skeleton_component_supports_primitive_bindings() {
    let mut world = World::default();
    let owner = world.spawn(());
    let bone_a = world.spawn(());
    let bone_b = world.spawn(());
    let nodes = [owner, bone_a, bone_b];

    spawn(&mut world, authored_skeleton(None), owner, &nodes);

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .unwrap();
    assert_eq!(skeleton.root(), None);
    assert!(skeleton.bones().iter().all(|bone| bone.entity().is_some()));
}

#[test]
fn invalid_references_remain_unresolved_without_shifting_bone_indices() {
    let mut world = World::default();
    let owner = world.spawn(());
    let bone = world.spawn(());
    let nodes = [owner, bone];
    let mut skeleton = authored_skeleton(Some(SceneEntityRef::Index(9)));
    skeleton.bones = vec![SceneEntityRef::Index(9), SceneEntityRef::Index(1)];

    spawn(&mut world, skeleton, owner, &nodes);

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .unwrap();
    assert_eq!(skeleton.bones()[0], SceneEntityRef::default());
    assert_eq!(skeleton.bones()[0].entity(), None);
    assert_eq!(skeleton.bones()[1], SceneEntityRef::Entity(bone));
    assert_eq!(skeleton.root(), None);
}

#[test]
fn a_scene_that_still_stores_bone_ids_loads_with_the_field_ignored() {
    let mut world = World::default();
    let owner = world.spawn(());
    let bone_a = world.spawn(());
    let bone_b = world.spawn(());
    let nodes = [owner, bone_a, bone_b];
    let ReflectedValue::Struct(mut fields) =
        to_value(&authored_skeleton(Some(SceneEntityRef::Index(1)))).unwrap()
    else {
        panic!("a component is stored as a struct");
    };
    fields.push((
        "bone_ids".to_string(),
        ReflectedValue::List(vec![ReflectedValue::Leaf(vec![0; 16])]),
    ));

    world.register_type::<SkeletonComponent>();
    assert!(world.apply_scene_component(
        "SkeletonComponent",
        &ReflectedValue::Struct(fields),
        owner,
        &nodes,
    ));

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .expect("the component must still spawn");
    assert_eq!(
        skeleton.bones(),
        &[
            SceneEntityRef::Entity(bone_a),
            SceneEntityRef::Entity(bone_b),
        ]
    );
    assert_eq!(skeleton.root(), Some(bone_a));
}
