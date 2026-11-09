//! Covers the read side of the scene-component registry: which registered
//! components an entity carries, and what their current values are.
//!
//! This is what lets a tool inspect the live world instead of a parallel copy
//! of the scene file it was spawned from.
use concerto_ecs::{Component, Entity, World, reflect::ReflectSceneComponent};
use concerto_reflect::Reflect;

#[derive(Component, Reflect, Default, PartialEq, Debug)]
#[reflect(SceneComponent)]
struct Health {
    current: u32,
    max: u32,
}

#[derive(Component, Reflect, Default, PartialEq, Debug)]
#[reflect(SceneComponent)]
struct Armour {
    plates: u8,
}

/// A plain component that is never registered as a scene component — engine
/// plumbing, from the inspector's point of view.
#[derive(Component)]
struct Plumbing;

fn names(world: &World, entity: Entity) -> Vec<&'static str> {
    let mut names: Vec<_> = world
        .component_types(entity)
        .map(|info| info.short())
        .collect();
    names.sort_unstable();
    names
}

struct ListingCase {
    spawn: fn(&mut World) -> Entity,
    names: &'static [&'static str],
}

#[test]
fn an_entity_lists_only_the_registered_components_it_carries() {
    let cases = [
        ListingCase {
            spawn: |world| {
                world.spawn((
                    Health {
                        current: 3,
                        max: 10,
                    },
                    Armour { plates: 2 },
                ))
            },
            names: &["Armour", "Health"],
        },
        ListingCase {
            spawn: |world| world.spawn((Health { current: 1, max: 1 }, Plumbing)),
            names: &["Health"],
        },
        ListingCase {
            spawn: |world| world.spawn(Plumbing),
            names: &[],
        },
    ];
    for case in cases {
        let mut world = World::default();
        world.register_type::<Health>();
        world.register_type::<Armour>();

        let entity = (case.spawn)(&mut world);

        assert_eq!(names(&world, entity), case.names);
    }
}

#[test]
fn component_ids_include_components_that_are_not_scene_components() {
    let mut world = World::default();
    world.register_type::<Health>();

    let entity = world.spawn((
        Health {
            current: 3,
            max: 10,
        },
        Plumbing,
    ));

    let mut ids = world.component_ids(entity).to_vec();
    ids.sort();
    let mut expected = vec![
        std::any::TypeId::of::<Health>(),
        std::any::TypeId::of::<Plumbing>(),
    ];
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn a_stale_entity_has_no_component_ids() {
    let mut world = World::default();
    let entity = world.spawn((Plumbing,));
    world.despawn(entity);

    assert!(world.component_ids(entity).is_empty());
}
