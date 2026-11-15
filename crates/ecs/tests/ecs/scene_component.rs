use std::collections::HashMap;

use concerto_ecs::{
    Component, Entity, World,
    component::scene::SceneEntityRef,
    reflect::{AppTypeRegistry, ReflectSceneComponent, SceneCapture},
};
use concerto_reflect::{Reflect, ReflectedValue, ValueWarning, to_value};

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

fn spawn_rig(world: &mut World, rig: &Rig, entity: Entity, nodes: &[Entity]) -> Vec<ValueWarning> {
    let spawn = world
        .get_resource::<AppTypeRegistry>()
        .unwrap()
        .read()
        .get_type_data::<ReflectSceneComponent>(std::any::TypeId::of::<Rig>())
        .unwrap()
        .spawn;
    spawn(world, entity, &to_value(rig).unwrap(), nodes)
}

#[derive(Clone, Copy, Debug)]
enum Ref {
    Node(usize),
    Unset,
}

impl Ref {
    fn stored(self) -> SceneEntityRef {
        match self {
            Ref::Node(index) => SceneEntityRef::Index(index),
            Ref::Unset => SceneEntityRef::default(),
        }
    }

    fn spawned(self, nodes: &[Entity]) -> SceneEntityRef {
        match self {
            Ref::Node(index) => SceneEntityRef::Entity(nodes[index]),
            Ref::Unset => SceneEntityRef::default(),
        }
    }
}

struct RigCase {
    name: &'static str,
    nodes: usize,
    bones: &'static [Ref],
    root: Option<Ref>,
    spawned_bones: &'static [Ref],
    spawned_root: Option<Ref>,
    warnings: &'static [&'static str],
}

#[test]
fn a_stored_reference_spawns_as_its_node_and_one_past_the_scene_spawns_unset() {
    let cases = [
        RigCase {
            name: "references map to the scene's nodes",
            nodes: 3,
            bones: &[Ref::Node(1), Ref::Node(2)],
            root: Some(Ref::Node(1)),
            spawned_bones: &[Ref::Node(1), Ref::Node(2)],
            spawned_root: Some(Ref::Node(1)),
            warnings: &[],
        },
        RigCase {
            name: "a reference past the scene spawns unset and warns",
            nodes: 1,
            bones: &[Ref::Node(7), Ref::Unset],
            root: Some(Ref::Node(0)),
            spawned_bones: &[Ref::Unset, Ref::Unset],
            spawned_root: Some(Ref::Node(0)),
            warnings: &["bones[0]"],
        },
    ];
    for case in cases {
        let mut world = World::new();
        world.register_type::<Rig>();
        let nodes: Vec<Entity> = (0..case.nodes).map(|_| world.spawn(())).collect();
        let rig = Rig {
            weight: 1.0,
            bones: case.bones.iter().map(|bone| bone.stored()).collect(),
            root: case.root.map(Ref::stored),
        };

        let warnings = spawn_rig(&mut world, &rig, nodes[0], &nodes);

        let paths: Vec<&str> = warnings.iter().map(|w| w.path.as_str()).collect();
        assert_eq!(paths, case.warnings, "{}", case.name);
        let expected = Rig {
            weight: 1.0,
            bones: case
                .spawned_bones
                .iter()
                .map(|bone| bone.spawned(&nodes))
                .collect(),
            root: case.spawned_root.map(|root| root.spawned(&nodes)),
        };
        assert_eq!(
            world.get_component_for_entity::<Rig>(nodes[0]),
            Some(&expected),
            "{}",
            case.name
        );
    }
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
