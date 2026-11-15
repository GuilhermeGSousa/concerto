use concerto_ecs::component::scene::SceneEntityRef;
use concerto_ecs::{Entity, World};
use concerto_foundation::assets::{handle::AssetHandle, AssetId};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_reflect::{to_value, ReflectedValue};

struct Authored {
    name: &'static str,
    bones: Vec<SceneEntityRef>,
    root: Option<SceneEntityRef>,
    stale_bone_ids: bool,
    resolved_bones: [Option<usize>; 2],
    resolved_root: Option<usize>,
}

#[test]
fn a_spawned_skeleton_resolves_its_references_in_place_and_leaves_bad_ones_unset() {
    let cases = [
        Authored {
            name: "bones and a root",
            bones: vec![SceneEntityRef::Index(1), SceneEntityRef::Index(2)],
            root: Some(SceneEntityRef::Index(1)),
            stale_bone_ids: false,
            resolved_bones: [Some(1), Some(2)],
            resolved_root: Some(1),
        },
        Authored {
            name: "a scene that still stores bone ids",
            bones: vec![SceneEntityRef::Index(1), SceneEntityRef::Index(2)],
            root: Some(SceneEntityRef::Index(1)),
            stale_bone_ids: true,
            resolved_bones: [Some(1), Some(2)],
            resolved_root: Some(1),
        },
    ];
    for case in cases {
        let mut world = World::default();
        world.register_type::<SkeletonComponent>();
        let nodes: Vec<Entity> = (0..3).map(|_| world.spawn(())).collect();
        let ReflectedValue::Struct(mut fields) = to_value(&SkeletonComponent {
            skeleton: AssetHandle::<Skeleton>::weak(AssetId::from_path("rig.gltf#skeleton/0")),
            bones: case.bones,
            root: case.root,
        })
        .unwrap() else {
            panic!("a component is stored as a struct");
        };
        if case.stale_bone_ids {
            fields.push((
                "bone_ids".to_string(),
                ReflectedValue::List(vec![ReflectedValue::Leaf(vec![0; 16])]),
            ));
        }

        assert!(world.apply_scene_component(
            "SkeletonComponent",
            &ReflectedValue::Struct(fields),
            nodes[0],
            &nodes,
        ));

        let skeleton = world
            .get_component_for_entity::<SkeletonComponent>(nodes[0])
            .expect("spawning inserts the same component type");
        let expected: Vec<SceneEntityRef> = case
            .resolved_bones
            .iter()
            .map(|bone| {
                bone.map_or(SceneEntityRef::default(), |node| {
                    SceneEntityRef::Entity(nodes[node])
                })
            })
            .collect();
        assert_eq!(skeleton.bones(), expected.as_slice(), "{}", case.name);
        assert_eq!(
            skeleton.root(),
            case.resolved_root.map(|node| nodes[node]),
            "{}",
            case.name
        );
    }
}
