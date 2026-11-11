use std::panic::{AssertUnwindSafe, catch_unwind};

use concerto_ecs::{Component, Entity, World};

#[derive(Component, Default)]
struct Base;

#[derive(Component, Default)]
struct Extra;

#[derive(Component, Default)]
struct Leaf;

#[derive(Component, Default)]
struct Value(u32);

#[derive(Component, Default)]
#[require(Leaf)]
struct Own;

#[derive(Component, Default)]
#[require(Base)]
struct Requirer;

#[derive(Component, Default)]
#[require(Requirer)]
struct Deeper;

#[derive(Component, Default)]
#[require(Value = Value(1))]
struct Declares;

fn holds(world: &World, entity: Entity) -> Vec<&'static str> {
    let mut held = Vec::new();
    if world.get_component_for_entity::<Base>(entity).is_some() {
        held.push("Base");
    }
    if world.get_component_for_entity::<Extra>(entity).is_some() {
        held.push("Extra");
    }
    if world.get_component_for_entity::<Leaf>(entity).is_some() {
        held.push("Leaf");
    }
    if world.get_component_for_entity::<Requirer>(entity).is_some() {
        held.push("Requirer");
    }
    held
}

fn make_value() -> Value {
    Value(7)
}

struct ExternalCase {
    name: &'static str,
    build: fn(&mut World) -> Entity,
    holds: &'static [&'static str],
    value: Option<u32>,
}

#[test]
fn an_external_requirement_reaches_every_spawn_of_the_type() {
    let cases = [
        ExternalCase {
            name: "adds a requirement",
            build: |world| {
                world.register_required_components::<Base, Extra>();
                world.spawn(Base)
            },
            holds: &["Base", "Extra"],
            value: None,
        },
        ExternalCase {
            name: "merges with the type's own requirements",
            build: |world| {
                world.register_required_components::<Own, Extra>();
                world.spawn(Own)
            },
            holds: &["Extra", "Leaf"],
            value: None,
        },
        ExternalCase {
            name: "uses the given constructor",
            build: |world| {
                world.register_required_components_with::<Base, Value>(|| Value(4));
                world.spawn(Base)
            },
            holds: &["Base"],
            value: Some(4),
        },
        ExternalCase {
            name: "reaches types built before it",
            build: |world| {
                world.register_component::<Deeper>();
                world.register_required_components::<Base, Extra>();
                world.spawn(Deeper)
            },
            holds: &["Base", "Extra", "Requirer"],
            value: None,
        },
        ExternalCase {
            name: "reaches types built after it",
            build: |world| {
                world.register_required_components::<Base, Extra>();
                world.spawn(Deeper)
            },
            holds: &["Base", "Extra", "Requirer"],
            value: None,
        },
        ExternalCase {
            name: "is allowed while the archetype is empty",
            build: |world| {
                let entity = world.spawn(Base);
                world.despawn(entity);
                world.register_required_components::<Base, Extra>();
                world.spawn(Base)
            },
            holds: &["Base", "Extra"],
            value: None,
        },
        ExternalCase {
            name: "a declared requirement wins over an external one",
            build: |world| {
                world.register_component::<Declares>();
                world.register_required_components_with::<Declares, Value>(|| Value(5));
                world.spawn(Declares)
            },
            holds: &[],
            value: Some(1),
        },
        ExternalCase {
            name: "the later external registration wins",
            build: |world| {
                world.register_required_components_with::<Base, Value>(|| Value(2));
                world.register_required_components_with::<Base, Value>(|| Value(3));
                world.spawn(Base)
            },
            holds: &["Base"],
            value: Some(3),
        },
        ExternalCase {
            name: "an identical registration is a no-op while an entity holds the type",
            build: |world| {
                world.register_required_components_with::<Base, Value>(make_value);
                let entity = world.spawn(Base);
                world.register_required_components_with::<Base, Value>(make_value);
                entity
            },
            holds: &["Base"],
            value: Some(7),
        },
    ];
    for case in cases {
        let mut world = World::new();

        let entity = (case.build)(&mut world);

        assert_eq!(holds(&world, entity), case.holds, "{}", case.name);
        let value = world
            .get_component_for_entity::<Value>(entity)
            .map(|value| value.0);
        assert_eq!(value, case.value, "{}", case.name);
    }
}

struct RefusedCase {
    name: &'static str,
    build: fn(&mut World),
}

#[test]
fn an_external_requirement_that_would_change_live_entities_or_cycle_panics_naming_the_type() {
    let cases = [
        RefusedCase {
            name: "an entity holds the type",
            build: |world| {
                world.spawn(Base);
                world.register_required_components::<Base, Extra>();
            },
        },
        RefusedCase {
            name: "an entity holds a requirer",
            build: |world| {
                world.spawn(Requirer);
                world.register_required_components::<Base, Extra>();
            },
        },
        RefusedCase {
            name: "a cycle",
            build: |world| {
                world.register_required_components::<Base, Extra>();
                world.register_required_components::<Extra, Base>();
            },
        },
        RefusedCase {
            name: "a self requirement",
            build: |world| world.register_required_components::<Base, Base>(),
        },
        RefusedCase {
            name: "a different constructor while an entity holds the type",
            build: |world| {
                world.register_required_components_with::<Base, Value>(make_value);
                world.spawn(Base);
                world.register_required_components_with::<Base, Value>(|| Value(8));
            },
        },
    ];
    for case in cases {
        let mut world = World::new();

        let payload =
            catch_unwind(AssertUnwindSafe(|| (case.build)(&mut world))).expect_err(case.name);

        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap();
        assert!(message.contains("Base"), "{}: {message}", case.name);
    }
}
