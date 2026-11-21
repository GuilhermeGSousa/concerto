//! Covers finding a scene component by name: a registered type spawns from its
//! stored value; an unregistered type name is reported rather than panicking.
use concerto_ecs::{Component, World, reflect::ReflectSceneComponent};
use concerto_reflect::{Reflect, ReflectedValue, to_value};

#[derive(Component, Reflect, Default, PartialEq, Debug)]
#[reflect(SceneComponent)]
struct Registered {
    value: u32,
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
