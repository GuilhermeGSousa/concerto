use concerto_ecs::{Component, Entity, IntoSystem, Query, System, World};
use std::any::TypeId;

#[derive(Component)]
struct Value(f32);
#[derive(Component)]
struct Other;

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
    let mut world = World::default();
    let entity = world.spawn((Value(1.0), Other));
    world.tick();
    let read_at = world.change_tick();
    fn read(values: Query<(Entity, &mut Value)>) {
        for (entity, value) in values.iter() {
            assert_eq!(
                values.get_entity(entity).map(|(_, same)| same.0),
                Some(value.0)
            );
        }
    }
    let mut reader = read.into_system();
    reader.initialize(&mut world);
    reader.run_and_apply((), &mut world);
    world.tick();
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    let mut system = mutate.into_system();
    system.initialize(&mut world);
    system.run_and_apply((), &mut world);
    world.tick();
    assert!(world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Other>(), read_at));
    let read_at = world.change_tick();
    world.insert(Value(9.0), entity);
    world.tick();
    assert!(world.has_component_changed_since(entity, TypeId::of::<Value>(), read_at));
    assert!(!world.has_component_changed_since(entity, TypeId::of::<Other>(), read_at));
}
