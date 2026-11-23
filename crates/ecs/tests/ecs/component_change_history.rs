use concerto_ecs::{
    Changed, Component, Entity, IntoSystem, Query, Ref, ResMut, Resource, System, World,
    query::change_detection::DetectChanges,
};
use std::any::TypeId;

#[derive(Component, PartialEq)]
struct Value(f32);
#[derive(Component)]
struct Other;
struct Seen {
    entity: Entity,
    value: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sighting {
    value: f32,
    added: bool,
    changed: bool,
}
#[derive(Resource, Default)]
struct Sightings(Vec<Sighting>);

fn watch(values: Query<Ref<Value>, Changed<Value>>, mut sightings: ResMut<Sightings>) {
    sightings.0 = values
        .iter()
        .map(|value| Sighting {
            value: value.0,
            added: value.is_added(),
            changed: value.is_changed(),
        })
        .collect();
}

#[test]
fn changes_survive_tick_advance_and_the_read_boundary_is_inclusive() {
    let mut world = World::default();
    let entity = world.spawn(Value(1.0));
    world.tick();
    let read_at = world.change_tick();
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    // This write happens after the consumer read, in the same tick.
    world
        .get_component_for_entity_mut::<Value>(entity)
        .unwrap()
        .0 = 2.0;
    world.tick();
    world.tick();
    assert!(!world.was_component_changed(entity, TypeId::of::<Value>()));
    assert!(world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    let reread_at = world.change_tick();
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Value>(), reread_at));
    assert!(!world.was_component_changed(entity, TypeId::of::<Value>()));
}

#[test]
fn query_mutations_and_replacements_are_observed_without_touching_other_components() {
    fn mutate(mut values: Query<&mut Value>) {
        for mut value in values.iter_mut() {
            value.0 += 1.0;
        }
    }
    fn rewrite(mut values: Query<&mut Value>) {
        for mut value in values.iter_mut() {
            assert!(!value.set_if_neq(Value(2.0)));
        }
    }
    let mut world = World::default();
    world.init_resource::<Sightings>();
    let entity = world.spawn((Value(1.0), Other));
    world.spawn(Other);
    let mut watcher = watch.into_system();
    watcher.initialize(&mut world);
    let mut sightings = |world: &mut World| {
        watcher.run_and_apply((), world);
        world.get_resource::<Sightings>().unwrap().0.clone()
    };
    let fresh = |value| Sighting {
        value,
        added: true,
        changed: true,
    };
    assert_eq!(sightings(&mut world), [fresh(1.0)]);
    assert_eq!(sightings(&mut world), []);
    world.tick();
    let read_at = world.change_tick();
    fn read(mut values: Query<(Entity, &mut Value)>) {
        let seen = values
            .iter_mut()
            .map(|(entity, value)| Seen {
                entity,
                value: value.0,
            })
            .collect::<Vec<_>>();
        assert_eq!(seen.len(), 1);
        for seen in seen {
            assert_eq!(
                values.get_entity_mut(seen.entity).map(|(_, same)| same.0),
                Some(seen.value)
            );
        }
    }
    let mut reader = read.into_system();
    reader.initialize(&mut world);
    reader.run_and_apply((), &mut world);
    world.tick();
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    assert_eq!(sightings(&mut world), []);
    let mut system = mutate.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    let rewritten = Sighting {
        value: 2.0,
        added: false,
        changed: true,
    };
    assert_eq!(sightings(&mut world), [rewritten]);
    let mut rewriter = rewrite.into_system();
    rewriter.initialize(&mut world);
    rewriter.run_and_apply((), &mut world);
    assert_eq!(sightings(&mut world), []);
    world.tick();
    assert!(world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Other>(), read_at));
    let read_at = world.change_tick();
    world.insert(Value(9.0), entity);
    world.tick();
    assert!(world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Other>(), read_at));
}
