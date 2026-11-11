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

fn stored(value: impl Reflect) -> ReflectedValue {
    to_value(&value).unwrap()
}

struct ApplyCase {
    name: &'static str,
    register: fn(&mut World),
    type_name: fn() -> &'static str,
    stored: fn() -> ReflectedValue,
    applied: bool,
    spawned: Option<Registered>,
}

#[test]
fn a_scene_component_is_found_by_its_short_or_full_name_and_spawned_from_its_stored_value() {
    let cases = [
        ApplyCase {
            name: "the short name",
            register: |world| world.register_type::<Registered>(),
            type_name: || "Registered",
            stored: || stored(Registered { value: 42 }),
            applied: true,
            spawned: Some(Registered { value: 42 }),
        },
        ApplyCase {
            name: "the full type path",
            register: |world| world.register_type::<Registered>(),
            type_name: Registered::name,
            stored: || stored(Registered { value: 8 }),
            applied: true,
            spawned: Some(Registered { value: 8 }),
        },
        ApplyCase {
            name: "a type registered twice",
            register: |world| {
                world.register_type::<Registered>();
                world.register_type::<Registered>();
            },
            type_name: || "Registered",
            stored: || stored(Registered { value: 5 }),
            applied: true,
            spawned: Some(Registered { value: 5 }),
        },
        ApplyCase {
            name: "an unregistered name is reported, not fatal",
            register: |_| {},
            type_name: || "NeverRegistered",
            stored: || ReflectedValue::Struct(Vec::new()),
            applied: false,
            spawned: None,
        },
        ApplyCase {
            name: "stored data of the wrong shape spawns the default",
            register: |world| world.register_type::<Registered>(),
            type_name: || "Registered",
            stored: || ReflectedValue::List(Vec::new()),
            applied: true,
            spawned: Some(Registered::default()),
        },
    ];
    for case in cases {
        let mut world = World::default();
        (case.register)(&mut world);
        let entity = world.spawn(());

        let applied =
            world.apply_scene_component((case.type_name)(), &(case.stored)(), entity, &[entity]);

        assert_eq!(applied, case.applied, "{}", case.name);
        assert_eq!(
            world.get_component_for_entity::<Registered>(entity),
            case.spawned.as_ref(),
            "{}",
            case.name
        );
    }
}
