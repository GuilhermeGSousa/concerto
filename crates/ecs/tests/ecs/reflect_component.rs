use std::any::{Any, TypeId};

use concerto_ecs::{
    Component, Entity, World,
    component::scene::{LiveEntityRefs, SceneEntityRef},
    reflect::{AppTypeRegistry, ReflectComponent},
    system::input::ComponentMetadata,
};
use concerto_reflect::{
    Reflect, ReflectDefault, ReflectJson, TypeRegistry, apply_value, apply_value_with,
    json_to_value, json_to_value_with, to_json,
};
use serde_json::{Value, json};

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Health {
    points: f32,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Lens {
    length: f32,
}

fn component<T: 'static>(world: &World) -> ReflectComponent {
    world
        .get_resource::<AppTypeRegistry>()
        .unwrap()
        .get_type_data::<ReflectComponent>(TypeId::of::<T>())
        .unwrap()
}

#[test]
fn a_new_world_registers_only_the_built_in_leaves_and_scene_entity_references() {
    let world = World::new();

    let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
    assert!(registry.get(TypeId::of::<SceneEntityRef>()).is_some());
    assert_eq!(
        registry.iter().count(),
        TypeRegistry::default().iter().count() + 1
    );
}

fn entity_ref_codec(world: &World) -> ReflectJson {
    world
        .get_resource::<AppTypeRegistry>()
        .unwrap()
        .get_type_data::<ReflectJson>(TypeId::of::<SceneEntityRef>())
        .unwrap()
}

fn entity_ref_from_json(world: &World, value: &Value) -> Result<SceneEntityRef, String> {
    let decoded = (entity_ref_codec(world).from_json)(value)?;
    Ok(*(decoded.as_ref() as &dyn Any)
        .downcast_ref::<SceneEntityRef>()
        .unwrap())
}

struct EntityRefCase {
    json: Value,
    reference: Option<SceneEntityRef>,
}

#[test]
fn an_entity_reference_is_its_id_string_when_resolved_and_its_node_index_when_not() {
    let world = World::new();
    let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
    let cases = [
        EntityRefCase {
            json: json!("42v3"),
            reference: Some(SceneEntityRef::Entity(Entity::parse_id("42v3").unwrap())),
        },
        EntityRefCase {
            json: json!(7),
            reference: Some(SceneEntityRef::Index(7)),
        },
        EntityRefCase {
            json: json!("42"),
            reference: None,
        },
        EntityRefCase {
            json: json!("42v0"),
            reference: None,
        },
        EntityRefCase {
            json: json!(-1),
            reference: None,
        },
        EntityRefCase {
            json: json!(true),
            reference: None,
        },
    ];
    for case in cases {
        let decoded = entity_ref_from_json(&world, &case.json);
        match case.reference {
            Some(reference) => {
                assert_eq!(to_json(&reference, &registry), case.json);
                assert_eq!(decoded, Ok(reference), "{}", case.json);
            }
            None => assert!(decoded.is_err(), "{} must not decode", case.json),
        }
    }
}

#[derive(Component, Reflect, Default, Debug, PartialEq)]
#[reflect(Component, Default)]
struct Rope {
    ends: Vec<SceneEntityRef>,
}

fn rope_world() -> World {
    let mut world = World::new();
    world.register_type::<Rope>();
    world
}

#[test]
fn an_unresolved_entity_reference_written_from_json_applies_as_its_index() {
    let world = rope_world();
    let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
    let value = Rope {
        ends: vec![SceneEntityRef::Index(7)],
    };

    let stored =
        json_to_value(&to_json(&value, &registry), TypeId::of::<Rope>(), &registry).unwrap();
    let mut loaded = Rope::default();
    assert!(apply_value(&mut loaded, &stored).is_empty());
    assert_eq!(loaded, value);
}

#[test]
fn a_resolved_entity_reference_is_not_written_from_json_without_live_references() {
    let world = rope_world();
    let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();

    assert_eq!(
        json_to_value(
            &json!({ "ends": ["42v3"] }),
            TypeId::of::<Rope>(),
            &registry
        ),
        Err(
            "ends[0]: a resolved scene entity can only be stored while its scene is captured"
                .to_owned()
        )
    );
}

#[test]
fn live_references_write_resolved_and_unresolved_entity_references_from_json() {
    let world = rope_world();
    let registry = world.get_resource::<AppTypeRegistry>().unwrap().read();
    let value = Rope {
        ends: vec![
            SceneEntityRef::Entity(Entity::parse_id("42v3").unwrap()),
            SceneEntityRef::Index(7),
        ],
    };

    let stored = json_to_value_with(
        &to_json(&value, &registry),
        TypeId::of::<Rope>(),
        &registry,
        &mut LiveEntityRefs,
    )
    .unwrap();
    let mut loaded = Rope::default();
    assert!(apply_value_with(&mut loaded, &stored, &mut LiveEntityRefs).is_empty());
    assert_eq!(loaded, value);
}

#[test]
fn a_reflected_component_is_read_edited_inserted_at_default_and_removed() {
    let mut world = World::new();
    world.register_type::<Health>();
    let reflect = component::<Health>(&world);
    let entity = world.spawn(());

    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_none());
    assert!(!(reflect.edit)(&mut world, entity, &mut |_| {}));

    assert!(reflect.insert_default(TypeId::of::<Health>(), &mut world, entity));
    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_some());

    let mut visited = false;
    assert!((reflect.edit)(&mut world, entity, &mut |_| visited = true));
    assert!(visited);

    (reflect.remove)(&mut world, entity);
    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_none());
}

#[test]
fn a_component_without_default_cannot_be_inserted() {
    let mut world = World::new();
    world.register_type::<Lens>();
    let reflect = component::<Lens>(&world);
    let entity = world.spawn(());

    assert!(!reflect.insert_default(TypeId::of::<Lens>(), &mut world, entity));
    assert!(world.get_component_for_entity::<Lens>(entity).is_none());
    let _ = Lens { length: 1.0 }.length;
}

#[test]
fn a_world_hands_out_its_own_registry_and_two_worlds_do_not_share_one() {
    let mut first = World::new();
    let mut second = World::new();

    first.type_registry().write().register::<Health>();

    assert!(first.registration(TypeId::of::<Health>()).is_some());
    assert!(second.registration(TypeId::of::<Health>()).is_none());
    assert!(
        second
            .type_registry()
            .read()
            .get(TypeId::of::<Health>())
            .is_none()
    );
}

#[test]
fn a_type_registered_through_the_world_is_visible_through_its_shared_registry() {
    let mut world = World::new();
    let shared = world.type_registry();

    world.register_type::<Health>();

    assert!(shared.read().get(TypeId::of::<Health>()).is_some());
}
