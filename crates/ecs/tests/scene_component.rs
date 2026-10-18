use std::collections::HashMap;

use concerto_ecs::{
    Component, Entity, World,
    component::scene::SceneEntityRef,
    reflect::{ReflectSceneComponent, SceneCapture},
};
use concerto_reflect::{Reflect, ReflectedValue, to_value};

#[derive(Component, Reflect, Default, Debug, PartialEq, Clone)]
#[reflect(SceneComponent)]
struct Rig {
    weight: f32,
    bones: Vec<SceneEntityRef>,
    root: Option<SceneEntityRef>,
}

#[derive(Component, Reflect, Default, Debug, PartialEq)]
#[reflect(SceneComponent)]
struct PlainMarker;

fn stored_rig(bones: Vec<usize>, root: Option<usize>) -> ReflectedValue {
    to_value(&Rig {
        weight: 2.0,
        bones: bones.into_iter().map(SceneEntityRef::Index).collect(),
        root: root.map(SceneEntityRef::Index),
    })
    .unwrap()
}

fn capture(
    world: &World,
    entity: Entity,
    indices: HashMap<Entity, usize>,
) -> Option<ReflectedValue> {
    let mut capture = SceneCapture::new(indices);
    world
        .component_types(entity)
        .next()
        .map(|component| component.capture(world, entity, &mut capture).unwrap())
}

#[test]
fn a_scene_component_spawns_from_a_stored_value_with_its_references_mapped() {
    let mut world = World::new();
    world.register_type::<Rig>();
    let nodes = [world.spawn(()), world.spawn(()), world.spawn(())];

    assert!(world.apply_scene_component("Rig", &stored_rig(vec![1, 2], Some(1)), nodes[0], &nodes));

    assert_eq!(
        world.get_component_for_entity::<Rig>(nodes[0]),
        Some(&Rig {
            weight: 2.0,
            bones: vec![
                SceneEntityRef::Entity(nodes[1]),
                SceneEntityRef::Entity(nodes[2])
            ],
            root: Some(SceneEntityRef::Entity(nodes[1])),
        })
    );
}

#[test]
fn capture_maps_entities_back_to_their_node_indices() {
    let mut world = World::new();
    world.register_type::<Rig>();
    let nodes = [world.spawn(()), world.spawn(()), world.spawn(())];
    let stored = stored_rig(vec![2, 1], None);
    world.apply_scene_component("Rig", &stored, nodes[0], &nodes);

    let indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (*node, index))
        .collect();
    assert_eq!(capture(&world, nodes[0], indices), Some(stored));
}

#[test]
fn a_reference_out_of_range_keeps_its_unresolved_default() {
    let mut world = World::new();
    world.register_type::<Rig>();
    let nodes = [world.spawn(())];

    assert!(world.apply_scene_component("Rig", &stored_rig(vec![7], Some(0)), nodes[0], &nodes));

    let rig = world.get_component_for_entity::<Rig>(nodes[0]).unwrap();
    assert_eq!(rig.bones[0], SceneEntityRef::default());
    assert_eq!(rig.bones[0].entity(), None);
    assert_eq!(rig.root, Some(SceneEntityRef::Entity(nodes[0])));
}

#[test]
fn capturing_a_reference_to_an_entity_outside_the_scene_fails() {
    let mut world = World::new();
    world.register_type::<Rig>();
    let outside = world.spawn(());
    let entity = world.spawn(Rig {
        weight: 1.0,
        bones: vec![SceneEntityRef::Entity(outside)],
        root: None,
    });

    let mut capture = SceneCapture::new(HashMap::from([(entity, 0)]));
    let component = world.component_types(entity).next().unwrap();
    let error = component.capture(&world, entity, &mut capture).unwrap_err();
    assert_eq!(error.path, "bones[0]");
}

#[test]
fn an_unresolved_reference_encodes_as_its_index_without_a_world() {
    let stored = to_value(&SceneEntityRef::Index(3)).unwrap();
    let ReflectedValue::Leaf(bytes) = &stored else {
        panic!("SceneEntityRef is a leaf");
    };
    assert!(!bytes.is_empty());

    let mut world = World::new();
    let entity = world.spawn(());
    assert!(to_value(&SceneEntityRef::Entity(entity)).is_err());
}

#[test]
fn a_unit_component_spawns_and_captures() {
    let mut world = World::new();
    world.register_type::<PlainMarker>();
    let entity = world.spawn(());
    let stored = to_value(&PlainMarker).unwrap();

    assert!(world.apply_scene_component("PlainMarker", &stored, entity, &[entity]));
    assert!(
        world
            .get_component_for_entity::<PlainMarker>(entity)
            .is_some()
    );
    assert_eq!(
        capture(&world, entity, HashMap::from([(entity, 0)])),
        Some(stored)
    );
}

#[test]
fn name_is_the_full_type_path_and_distinguishes_generics() {
    #[derive(Component)]
    struct Wrapper<T: Send + Sync + 'static> {
        _marker: std::marker::PhantomData<T>,
    }

    assert!(PlainMarker::name().ends_with("PlainMarker"));
    assert_ne!(Wrapper::<u8>::name(), Wrapper::<u16>::name());
}
