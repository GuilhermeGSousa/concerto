//! Covers finding a scene component by name: a registered type spawns from its
//! stored value; an unregistered type name is reported rather than panicking.
use concerto_ecs::{Component, World, reflect::ReflectSceneComponent};
use concerto_reflect::{Reflect, ReflectedValue, to_value};

#[derive(Component, Reflect, Default, PartialEq, Debug)]
#[reflect(SceneComponent)]
struct Registered {
    value: u32,
}

// Two distinct types whose final `::` segment is the same identifier, so they
// collide on the short alias but not on the canonical full type path.
mod alias_clash_a {
    use super::*;

    #[derive(Component, Reflect, Default, PartialEq, Debug)]
    #[reflect(SceneComponent)]
    pub struct Widget {
        pub a: u32,
    }
}

mod alias_clash_b {
    use super::*;

    #[derive(Component, Reflect, Default, PartialEq, Debug)]
    #[reflect(SceneComponent)]
    pub struct Widget {
        pub b: u32,
    }
}

#[test]
fn registered_component_is_deserialized_and_applied() {
    let mut world = World::default();
    world.register_type::<Registered>();
    let entity = world.spawn(());

    let applied = world.apply_scene_component(
        "Registered",
        &stored(Registered { value: 42 }),
        entity,
        &[entity],
    );

    assert!(
        applied,
        "a registered type name must be found in the registry"
    );
    assert_eq!(
        world.get_component_for_entity::<Registered>(entity),
        Some(&Registered { value: 42 }),
        "the JSON payload must be deserialized into the real component and inserted"
    );
}

#[test]
fn short_name_alias_resolves_for_blender_authored_extras() {
    let mut world = World::default();
    world.register_type::<Registered>();
    let entity = world.spawn(());

    // Blender authors the bare identifier, not the full Rust path.
    let applied = world.apply_scene_component(
        "Registered",
        &stored(Registered { value: 7 }),
        entity,
        &[entity],
    );

    assert!(
        applied,
        "a short, Blender-style name must resolve to the registered type"
    );
    assert_eq!(
        world.get_component_for_entity::<Registered>(entity),
        Some(&Registered { value: 7 }),
        "the short alias must apply the same component the full path would"
    );
}

#[test]
fn full_type_path_also_resolves() {
    let mut world = World::default();
    world.register_type::<Registered>();
    let entity = world.spawn(());

    let applied = world.apply_scene_component(
        Registered::name(),
        &stored(Registered { value: 8 }),
        entity,
        &[entity],
    );

    assert!(
        applied,
        "the full type path is the canonical registry key and must resolve"
    );
    assert_eq!(
        world.get_component_for_entity::<Registered>(entity),
        Some(&Registered { value: 8 }),
        "applying via the full type path must insert the real component"
    );
}

#[test]
fn re_registering_the_same_type_is_idempotent() {
    let mut world = World::default();
    // Two plugins registering the same component, or one plugin added twice.
    world.register_type::<Registered>();
    world.register_type::<Registered>();
    let entity = world.spawn(());

    let applied = world.apply_scene_component(
        "Registered",
        &stored(Registered { value: 5 }),
        entity,
        &[entity],
    );

    assert!(
        applied,
        "a double registration must leave the short name resolvable, not corrupt the map"
    );
    assert_eq!(
        world.get_component_for_entity::<Registered>(entity),
        Some(&Registered { value: 5 }),
        "re-registering the same type must still deserialize and insert the component"
    );
}

#[test]
fn a_genuine_two_type_short_name_clash_resolves_to_the_last_registered() {
    let mut world = World::default();
    world.register_type::<alias_clash_a::Widget>();
    world.register_type::<alias_clash_b::Widget>();

    let by_short = world.spawn(());
    let by_full_a = world.spawn(());
    let by_full_b = world.spawn(());

    // The shared short name resolves to the type registered last under it.
    assert!(world.apply_scene_component(
        "Widget",
        &stored(alias_clash_b::Widget { b: 2 }),
        by_short,
        &[by_short]
    ));
    assert_eq!(
        world.get_component_for_entity::<alias_clash_b::Widget>(by_short),
        Some(&alias_clash_b::Widget { b: 2 }),
        "a colliding short name must resolve to the most recently registered type"
    );
    assert!(
        world
            .get_component_for_entity::<alias_clash_a::Widget>(by_short)
            .is_none(),
        "the shadowed type must not be applied through the shared short name"
    );

    // Each canonical full type path still resolves to its own type.
    assert!(world.apply_scene_component(
        alias_clash_a::Widget::name(),
        &stored(alias_clash_a::Widget { a: 1 }),
        by_full_a,
        &[by_full_a],
    ));
    assert_eq!(
        world.get_component_for_entity::<alias_clash_a::Widget>(by_full_a),
        Some(&alias_clash_a::Widget { a: 1 }),
        "the first type's canonical key must still resolve to the first type"
    );

    assert!(world.apply_scene_component(
        alias_clash_b::Widget::name(),
        &stored(alias_clash_b::Widget { b: 9 }),
        by_full_b,
        &[by_full_b],
    ));
    assert_eq!(
        world.get_component_for_entity::<alias_clash_b::Widget>(by_full_b),
        Some(&alias_clash_b::Widget { b: 9 }),
        "the second type's canonical key must still resolve to the second type"
    );
}

#[test]
fn unregistered_component_name_is_reported_not_fatal() {
    let mut world = World::default();
    let entity = world.spawn(());

    let applied = world.apply_scene_component(
        "NeverRegistered",
        &ReflectedValue::Struct(Vec::new()),
        entity,
        &[entity],
    );

    assert!(
        !applied,
        "an unregistered type name must report false rather than panicking"
    );
}

#[test]
fn stored_data_of_the_wrong_shape_spawns_the_default() {
    let mut world = World::default();
    world.register_type::<Registered>();
    let entity = world.spawn(());

    let applied = world.apply_scene_component(
        "Registered",
        &ReflectedValue::List(Vec::new()),
        entity,
        &[entity],
    );

    assert!(
        applied,
        "a known type still spawns when its data does not fit"
    );
    assert_eq!(
        world.get_component_for_entity::<Registered>(entity),
        Some(&Registered::default()),
        "data that does not fit is skipped, leaving the default"
    );
}

fn stored(value: impl Reflect) -> ReflectedValue {
    to_value(&value).unwrap()
}
