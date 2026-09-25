//! The entity component system at the core of Concerto.
//!
//! - [`World`] holds every entity, component and resource.
//! - [`Entity`] is a lightweight handle to a game object.
//! - [`Component`] is data attached to entities; derive it with `#[derive(Component)]`.
//! - [`Resource`] is data shared across the world; derive it with `#[derive(Resource)]`.
//! - [`Query`] iterates the entities that have a given set of components.
//! - [`Event`] is a message passed between systems; derive it with `#[derive(Event)]`.
//! - [`Schedule`] runs systems in an order given by explicit constraints, named
//!   [`SystemSet`]s and, where neither applies, registration order.
//!
//! # Examples
//!
//! ```
//! use concerto_ecs::{
//!     Component, IntoSystemConfig, Query, Schedule, World,
//!     system::executor::single_thread::SingleThreadedExecutor,
//! };
//!
//! #[derive(Component)]
//! struct Position(f32);
//!
//! #[derive(Component)]
//! struct Velocity(f32);
//!
//! fn integrate(query: Query<(&mut Position, &Velocity)>) {
//!     for (mut position, velocity) in query.iter() {
//!         position.0 += velocity.0;
//!     }
//! }
//!
//! fn report(query: Query<&Position>) {
//!     for position in query.iter() {
//!         println!("{}", position.0);
//!     }
//! }
//!
//! let mut world = World::new();
//! world.spawn((Position(0.0), Velocity(1.0)));
//!
//! let mut schedule = Schedule::new();
//! schedule.add_system(report.after(integrate)).add_system(integrate);
//! schedule
//!     .compile::<SingleThreadedExecutor>(&mut world)
//!     .run(&mut world);
//! ```

/// Storage grouping entities that share the same component types.
pub mod archetype;
/// Deferred world mutations queued by systems.
pub mod command;
/// Identifiers derived from sets of component types.
pub mod common;
/// Components and their registration.
pub mod component;
/// Entities, entity handles and hierarchies.
pub mod entity;
/// Events passed between systems.
pub mod events;
/// Interning of values into cheap, copyable handles.
pub mod intern;
/// Support for label traits such as [`SystemSet`].
pub mod label;
/// Queries over entities and their components.
pub mod query;
/// Resources shared across a world.
pub mod resource;
/// Systems, schedules and their ordering.
pub mod system;
/// Column storage for component data.
pub mod table;
/// Hashers keyed by `TypeId` and cells shareable across threads.
pub mod utilities;
/// Hashers with a fixed seed.
pub mod utils;
/// The [`World`] and access to it.
pub mod world;

pub use command::CommandQueue;
pub use component::Component;
pub use entity::{Entity, EntityWorldMut};
pub use events::Event;
pub use query::{
    Query,
    filter::{Added, Changed, Or, With, Without},
};
pub use resource::{Res, ResMut, Resource};
pub use system::{
    IntoSetConfig, IntoSystem, IntoSystemConfig, System, SystemConfig, SystemSet,
    schedule::Schedule,
};
pub use world::World;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use crate::{
        command::CommandQueue,
        component::Component,
        entity::Entity,
        events::{Event, event_channel::EventChannel},
        query::{
            Query,
            filter::{Added, Changed, Or, With},
        },
        resource::{Res, ResMut, Resource},
        system::{
            executor::single_thread::SingleThreadedExecutor,
            schedule::{CompiledSchedule, Schedule},
        },
        world::World,
    };

    #[derive(Component)]
    struct Health;

    #[derive(Component)]
    struct Position {
        pub x: f32,
        pub y: f32,
    }

    fn system_query_pos_hp(query: Query<(Entity, &Position, &mut Health)>) {
        for (_, position, _) in query.iter() {
            print!("{}", position.x);
        }
    }

    fn system_query_pos(query: Query<(Entity, &Position)>) {
        for (_, position) in query.iter() {
            print!("{}", position.x);
        }
    }

    fn system_query_added(query: Query<(&Position,), Added<Position>>) {
        for _ in query.iter() {
            print!("Found Added");
        }
    }

    fn system_query_add_hp(query: Query<(&mut Health,)>) {
        for _ in query.iter() {}
    }

    fn system_query_hp_changed(query: Query<(&Health,), Changed<Health>>) {
        for _ in query.iter() {
            println!("Health change detected");
        }
    }

    fn system_filter_or(query: Query<Entity, Or<(With<Health>, With<Position>)>>) {
        for entity in query.iter() {
            println!("Entity {:?}", entity);
        }
    }

    fn spawn(mut cmd: CommandQueue) {
        cmd.spawn((Position { x: 0.0, y: 0.0 }, Health));
    }

    static TRACKED_LIVE: AtomicIsize = AtomicIsize::new(0);

    #[derive(Component)]
    struct Tracked(u32);

    impl Tracked {
        fn new(value: u32) -> Self {
            TRACKED_LIVE.fetch_add(1, Ordering::SeqCst);
            Tracked(value)
        }
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            TRACKED_LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn assert_no_leaked_components(body: impl FnOnce()) {
        static SERIALISE: Mutex<()> = Mutex::new(());
        let _guard = SERIALISE.lock().unwrap_or_else(|err| err.into_inner());

        TRACKED_LIVE.store(0, Ordering::SeqCst);
        body();
        assert_eq!(
            TRACKED_LIVE.load(Ordering::SeqCst),
            0,
            "every Tracked component should have been dropped exactly once"
        );
    }

    #[test]
    fn despawn_drops_components() {
        assert_no_leaked_components(|| {
            let mut world = World::new();

            let entity = world.spawn((Tracked::new(1), Position { x: 1.0, y: 2.0 }));
            world.spawn((Tracked::new(2), Position { x: 3.0, y: 4.0 }));

            assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), 2);

            world.despawn(entity);
            assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), 1);

            drop(world);
        });
    }

    #[test]
    fn migration_moves_components_without_dropping_them() {
        assert_no_leaked_components(|| {
            let mut world = World::new();

            let entity = world.spawn(Tracked::new(7));

            // Migrates the row to a new archetype: `Tracked` is carried over, not re-created.
            world.insert(Position { x: 1.0, y: 2.0 }, entity);
            assert_eq!(
                TRACKED_LIVE.load(Ordering::SeqCst),
                1,
                "the carried-over component must not be dropped by the migration"
            );
            assert_eq!(
                world.get_component_for_entity::<Tracked>(entity).unwrap().0,
                7,
                "the carried-over value must survive the migration intact"
            );

            drop(world);
        });
    }

    #[test]
    fn replacing_a_component_drops_the_old_value() {
        assert_no_leaked_components(|| {
            let mut world = World::new();

            let entity = world.spawn(Tracked::new(1));
            world.insert(Tracked::new(2), entity);

            assert_eq!(
                TRACKED_LIVE.load(Ordering::SeqCst),
                1,
                "replacing a component should drop the value it displaced"
            );
            assert_eq!(
                world.get_component_for_entity::<Tracked>(entity).unwrap().0,
                2
            );

            drop(world);
        });
    }

    #[test]
    fn removing_a_component_drops_it() {
        assert_no_leaked_components(|| {
            let mut world = World::new();

            let entity = world.spawn((Tracked::new(1), Position { x: 1.0, y: 2.0 }));
            world.remove_component::<Tracked>(entity);

            assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), 0);
            assert!(world.get_component_for_entity::<Tracked>(entity).is_none());
            assert_eq!(
                world
                    .get_component_for_entity::<Position>(entity)
                    .unwrap()
                    .x,
                1.0,
                "the surviving component must be carried over by the migration"
            );

            drop(world);
        });
    }

    #[test]
    fn bundle_insert_replacing_and_adding_drops_only_the_displaced_value() {
        assert_no_leaked_components(|| {
            let mut world = World::new();

            let entity = world.spawn(Tracked::new(1));

            // `Tracked` is replaced while `Position` is added, so the row both migrates and
            // has one of its carried components deliberately left behind.
            world.insert((Tracked::new(2), Position { x: 5.0, y: 6.0 }), entity);

            assert_eq!(TRACKED_LIVE.load(Ordering::SeqCst), 1);
            assert_eq!(
                world.get_component_for_entity::<Tracked>(entity).unwrap().0,
                2
            );
            assert_eq!(
                world
                    .get_component_for_entity::<Position>(entity)
                    .unwrap()
                    .x,
                5.0
            );

            drop(world);
        });
    }

    #[test]
    fn migrating_a_non_final_row_keeps_other_entities_intact() {
        let mut world = World::new();

        let entities: Vec<Entity> = (0..4)
            .map(|i| {
                world.spawn(Position {
                    x: i as f32,
                    y: 0.0,
                })
            })
            .collect();

        // The middle row migrates out, so the archetype's last row is swapped into its slot.
        world.insert(Health, entities[1]);

        for (index, entity) in entities.iter().enumerate() {
            assert_eq!(
                world
                    .get_component_for_entity::<Position>(*entity)
                    .unwrap()
                    .x,
                index as f32,
                "entity {index} should still resolve to its own row after the swap"
            );
        }
    }

    #[test]
    fn remove_last_component_leaves_empty_entity() {
        let mut world = World::new();

        let entity = world.spawn((Position { x: 1.0, y: 2.0 },));
        world.remove_component::<Position>(entity);

        assert!(
            world.get_component_for_entity::<Position>(entity).is_none(),
            "component should be gone after removal"
        );
    }

    #[test]
    fn test_query() {
        let mut world = World::new();
        let mut schedule = Schedule::new();

        schedule.add_system(system_query_pos_hp);
        schedule.add_system(system_query_pos);

        world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        world.spawn((Position { x: 20.0, y: 20.0 },));

        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
    }

    #[test]
    fn test_added() {
        let mut world = World::new();
        let mut schedule = Schedule::new();

        schedule.add_system(system_query_added);

        world.spawn((Position { x: 0.0, y: 0.0 },));
        let mut compiled_schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
        compiled_schedule.run(&mut world);

        world.tick();

        world.spawn((Position { x: 0.0, y: 0.0 },));
        compiled_schedule.run(&mut world);

        world.tick();
        compiled_schedule.run(&mut world);
    }

    #[test]
    fn spawn_despawn() {
        let mut world = World::new();

        world.spawn((Position { x: 0.0, y: 0.0 },));
        let e2 = world.spawn((Position { x: 0.0, y: 0.0 },));
        world.spawn((Position { x: 0.0, y: 0.0 },));

        world.despawn(e2);

        let e4 = world.spawn((Position { x: 0.0, y: 0.0 },));

        assert!(e4.generation().get() == e2.generation().get() + 1);
        assert!(e4.index() == e2.index());
    }

    #[test]
    fn spawn_from_system() {
        let mut world = World::new();
        let mut schedule = Schedule::new();
        schedule.add_system(spawn);

        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        let mut state = world.query::<(&Position, &Health), ()>();
        let results: Vec<_> = state.iter(&mut world).collect();

        assert_eq!(results.len(), 1);

        for (position, _) in results {
            assert_eq!(position.x, 0.0);
            assert_eq!(position.y, 0.0);
        }
    }

    #[test]
    fn insert_on_new_archetype() {
        let mut world = World::new();

        let entity = world.spawn(Health);

        world.insert(Position { x: 10.0, y: 11.0 }, entity);

        let mut state = world.query::<(&Position, &Health), ()>();

        let mut count = 0;
        for (pos, _) in state.iter(&mut world) {
            assert_eq!(pos.x, 10.0);
            assert_eq!(pos.y, 11.0);
            count += 1;
        }

        assert_eq!(count, 1);
    }

    #[test]
    fn insert_on_existing_archetype() {
        let mut world = World::new();

        let entity = world.spawn(Health);
        world.spawn((Health, Position { x: 10.0, y: 11.0 }));

        world.insert(Position { x: 10.0, y: 11.0 }, entity);

        let mut state = world.query::<(&Position, &Health), ()>();

        let mut count = 0;
        for (pos, _) in state.iter(&mut world) {
            assert_eq!(pos.x, 10.0);
            assert_eq!(pos.y, 11.0);
            count += 1;
        }

        assert_eq!(count, 2);
    }

    #[test]
    fn insert_twice() {
        let mut world = World::new();

        let entity = world.spawn(Health);

        world.insert(Position { x: 0.0, y: 0.0 }, entity);
        world.insert(Position { x: 10.0, y: 11.0 }, entity);

        let mut state = world.query::<(&Position, &Health), ()>();

        let mut count = 0;
        for (pos, _) in state.iter(&mut world) {
            assert_eq!(pos.x, 10.0);
            assert_eq!(pos.y, 11.0);
            count += 1;
        }

        assert_eq!(count, 1);
    }

    #[test]
    fn insert_bundle_replaces_existing_and_adds_new_component() {
        let mut world = World::new();

        let entity = world.spawn(Health);

        // Bundle re-inserts `Health` (already present) alongside a brand new `Position`,
        // landing on an archetype that has never been created before.
        world.insert((Health, Position { x: 3.0, y: 4.0 }), entity);

        let mut state = world.query::<(&Position, &Health), ()>();

        let mut count = 0;
        for (pos, _) in state.iter(&mut world) {
            assert_eq!(pos.x, 3.0);
            assert_eq!(pos.y, 4.0);
            count += 1;
        }

        assert_eq!(count, 1);
    }

    #[test]
    fn test_change_detection() {
        let mut world = World::new();

        world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        world.spawn((Health, Position { x: 10.0, y: 20.0 }));

        world.tick();
        let mut schedule = Schedule::new();
        schedule.add_system(system_query_add_hp);
        schedule.add_system(system_query_hp_changed);

        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
    }

    #[test]
    fn test_or_query() {
        let mut world = World::new();

        world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        world.spawn(Position { x: 10.0, y: 20.0 });

        let mut schedule = Schedule::new();
        schedule.add_system(system_filter_or);

        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
    }

    #[test]
    fn test_add_children() {
        let mut world = World::new();

        let entity_parent = world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        let entity_child_1 = world.spawn((Health, Position { x: 10.0, y: 20.0 }));
        let entity_child_2 = world.spawn((Health, Position { x: 10.0, y: 20.0 }));

        world.entity_mut(entity_parent).add_child(entity_child_1);
        world.entity_mut(entity_parent).add_child(entity_child_2);
    }

    #[derive(Resource)]
    struct Score(u32);

    #[derive(Resource)]
    struct DoubleScore(u32);

    #[test]
    fn insert_and_get_resource() {
        let mut world = World::new();
        world.insert_resource(Score(42));
        assert_eq!(world.get_resource::<Score>().unwrap().0, 42);
    }

    #[test]
    fn remove_resource() {
        let mut world = World::new();
        world.insert_resource(Score(10));
        let removed = world.remove_resource::<Score>();
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().0, 10);
        assert!(world.get_resource::<Score>().is_none());
    }

    #[test]
    fn overwrite_resource() {
        let mut world = World::new();
        world.insert_resource(Score(1));
        world.insert_resource(Score(99));
        assert_eq!(world.get_resource::<Score>().unwrap().0, 99);
    }

    fn read_score(score: Res<Score>, mut result: ResMut<DoubleScore>) {
        result.0 = score.0 * 2;
    }

    #[test]
    fn resource_system_read_write() {
        let mut world = World::new();
        world.insert_resource(Score(5));
        world.insert_resource(DoubleScore(0));

        let mut schedule = Schedule::new();
        schedule.add_system(read_score);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<DoubleScore>().unwrap().0, 10);
    }

    // ----- event tests -----

    #[derive(Event)]
    struct PlayerDied {
        score: u32,
    }

    fn send_death(mut writer: crate::events::event_writer::EventWriter<PlayerDied>) {
        writer.write(PlayerDied { score: 77 });
    }

    fn count_deaths(
        mut reader: crate::events::event_reader::EventReader<PlayerDied>,
        mut counter: ResMut<Score>,
    ) {
        for event in reader.read() {
            counter.0 += event.score;
        }
    }

    fn count_deaths_again(
        mut reader: crate::events::event_reader::EventReader<PlayerDied>,
        mut counter: ResMut<DoubleScore>,
    ) {
        for event in reader.read() {
            counter.0 += event.score;
        }
    }

    fn event_schedules(
        world: &mut World,
    ) -> (CompiledSchedule, CompiledSchedule, CompiledSchedule) {
        let mut write = Schedule::new();
        write.add_system(send_death);
        let write = write.compile::<SingleThreadedExecutor>(world);

        let mut update = Schedule::new();
        update.add_system(crate::events::event_channel::update_event_channel::<PlayerDied>);
        let update = update.compile::<SingleThreadedExecutor>(world);

        let mut read = Schedule::new();
        read.add_system(count_deaths);
        let read = read.compile::<SingleThreadedExecutor>(world);

        (write, update, read)
    }

    #[test]
    fn events_sent_and_read_same_frame() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let mut schedule = Schedule::new();
        schedule.add_system(send_death);
        schedule.add_system(count_deaths);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
    }

    #[test]
    fn event_survives_one_buffer_swap() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let (mut write, mut update, mut read) = event_schedules(&mut world);

        write.run(&mut world);
        update.run(&mut world);
        read.run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
    }

    #[test]
    fn event_expires_after_two_buffer_swaps() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let (mut write, mut update, mut read) = event_schedules(&mut world);

        write.run(&mut world);
        update.run(&mut world);
        update.run(&mut world);
        read.run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 0);
        assert!(
            world
                .get_resource::<EventChannel<PlayerDied>>()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_event_is_never_read_twice() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let (mut write, mut update, mut read) = event_schedules(&mut world);

        write.run(&mut world);
        read.run(&mut world);
        update.run(&mut world);
        read.run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
    }

    #[test]
    fn readers_of_the_same_event_have_independent_cursors() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));
        world.insert_resource(DoubleScore(0));

        let mut schedule = Schedule::new();
        schedule.add_system(send_death);
        schedule.add_system(count_deaths);
        schedule.add_system(count_deaths_again);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
        assert_eq!(world.get_resource::<DoubleScore>().unwrap().0, 77);
    }

    #[test]
    fn a_reader_that_skips_frames_resumes_from_the_oldest_buffered_event() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let (mut write, mut update, mut read) = event_schedules(&mut world);

        // Three frames of writes go by without the reader ever running.
        for _ in 0..3 {
            write.run(&mut world);
            update.run(&mut world);
        }
        read.run(&mut world);

        // The cursor is clamped forward instead of panicking: only the one event still
        // buffered is delivered, the two older ones are gone.
        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
    }

    #[test]
    fn event_written_after_a_read_is_visible_next_frame() {
        let mut world = World::new();
        world.insert_resource(EventChannel::<PlayerDied>::new());
        world.insert_resource(Score(0));

        let mut read = Schedule::new();
        read.add_system(count_deaths);
        let mut read = read.compile::<SingleThreadedExecutor>(&mut world);

        let mut write = Schedule::new();
        write.add_system(send_death);
        let mut write = write.compile::<SingleThreadedExecutor>(&mut world);

        let mut update = Schedule::new();
        update.add_system(crate::events::event_channel::update_event_channel::<PlayerDied>);
        let mut update = update.compile::<SingleThreadedExecutor>(&mut world);

        // Frame 1: the reader runs before the writer, so it sees nothing.
        read.run(&mut world);
        write.run(&mut world);
        update.run(&mut world);

        // Frame 2: the event must still be there.
        read.run(&mut world);

        assert_eq!(world.get_resource::<Score>().unwrap().0, 77);
    }

    // ----- remove_component tests -----

    #[test]
    fn remove_component_leaves_other_components() {
        let mut world = World::new();
        let e = world.spawn((Health, Position { x: 3.0, y: 4.0 }));

        world.remove_component::<Health>(e);

        let mut state = world.query::<&Position, ()>();
        assert_eq!(state.iter(&mut world).count(), 1);

        // Entity should no longer appear in a query that requires Health.
        let mut state2 = world.query::<Entity, With<Health>>();
        assert_eq!(state2.iter(&mut world).count(), 0);
    }

    // ----- Without filter test -----

    #[test]
    fn without_filter() {
        let mut world = World::new();
        world.spawn((Health, Position { x: 1.0, y: 0.0 }));
        world.spawn(Position { x: 2.0, y: 0.0 });

        // Only the entity without Health should be returned.
        let mut state = world.query::<&Position, crate::query::filter::Without<Health>>();
        let results: Vec<_> = state.iter(&mut world).collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].x, 2.0);
    }

    // ----- Added filter after insert test -----

    #[test]
    fn added_filter_survives_insert() {
        let mut world = World::new();

        world.tick();

        let entity = world.spawn(Health);

        let mut state = world.query::<Entity, Added<Health>>();

        // Health was added this tick — the filter must match.
        assert_eq!(state.iter(&mut world).count(), 1);

        // Insert a second component on the same entity (moves it to a new archetype).
        world.insert(Position { x: 1.0, y: 2.0 }, entity);

        // Health was still added this tick — filter must still match.
        assert_eq!(state.iter(&mut world).count(), 1);
    }

    // ----- tick preservation test -----

    #[test]
    fn component_ticks_preserved_after_sibling_insert() {
        use crate::component::ComponentId;

        let mut world = World::new();

        // Advance to tick 1 so the added_tick for Health is distinct from the initial
        // changed_tick (which is always 0), making each assertion unambiguous.
        world.tick();

        let entity = world.spawn(Health);

        // Health was just added at tick 1.
        assert!(world.was_component_added(entity, ComponentId::of::<Health>()));
        assert!(world.was_component_changed(entity, ComponentId::of::<Health>()));

        // Advance to tick 2 before inserting Position.
        world.tick();

        // Insert a second component — entity migrates to a new archetype.
        world.insert(Position { x: 1.0, y: 2.0 }, entity);

        // Health's added_tick must still be tick 1, not the new tick 2.
        assert!(!world.was_component_added(entity, ComponentId::of::<Health>()));
        // Health was never mutated, so changed_tick is still 0 — not the current tick 2.
        assert!(!world.was_component_changed(entity, ComponentId::of::<Health>()));

        // Position was freshly added at tick 2.
        assert!(world.was_component_added(entity, ComponentId::of::<Position>()));
        assert!(world.was_component_changed(entity, ComponentId::of::<Position>()));
    }

    // ----- get_entity test -----

    #[test]
    fn get_entity_returns_none_for_missing_component() {
        let mut world = World::new();
        let e_with = world.spawn((Health, Position { x: 5.0, y: 0.0 }));
        let e_without = world.spawn(Position { x: 6.0, y: 0.0 });

        // `get_entity` lives on `Query`, not `QueryState`, so this one still goes
        // through `Query::new` rather than the `world.query().iter()` shorthand.
        let mut state = world.query::<&Health, ()>();
        let q = Query::new(world.as_unsafe_world_cell_mut(), &mut state);
        assert!(q.get_entity(e_with).is_some());
        assert!(q.get_entity(e_without).is_none());
    }
}
