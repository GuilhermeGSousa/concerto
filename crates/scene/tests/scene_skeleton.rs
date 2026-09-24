use concerto_ecs::component::scene::{SceneComponent, SceneEntityRef, SceneSpawnContext};
use concerto_ecs::World;
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use uuid::Uuid;

fn authored_skeleton(root: Option<SceneEntityRef>) -> SkeletonComponent {
    SkeletonComponent {
        skeleton: AssetHandle::<Skeleton>::weak(AssetId::from_path("rig.gltf#skeleton/0")),
        bones: vec![SceneEntityRef::Index(1), SceneEntityRef::Index(2)],
        bone_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2)],
        root,
    }
}

#[test]
fn skeleton_component_resolves_scene_references_in_place() {
    let mut world = World::default();
    let owner = world.spawn(());
    let bone_a = world.spawn(());
    let bone_b = world.spawn(());
    let nodes = [owner, bone_a, bone_b];

    let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
    authored_skeleton(Some(SceneEntityRef::Index(1))).apply(owner, &mut ctx);

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .expect("apply must insert the same component type");
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

    let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
    authored_skeleton(None).apply(owner, &mut ctx);

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

    let mut ctx = SceneSpawnContext::new(&mut world, &nodes);
    skeleton.apply(owner, &mut ctx);

    let skeleton = world
        .get_component_for_entity::<SkeletonComponent>(owner)
        .unwrap();
    assert_eq!(skeleton.bones()[0], SceneEntityRef::Index(9));
    assert_eq!(skeleton.bones()[1], SceneEntityRef::Entity(bone));
    assert_eq!(skeleton.root(), None);
}
