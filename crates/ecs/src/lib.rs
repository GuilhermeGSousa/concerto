//! The entity component system at the core of Concerto.
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
//! let mut world = World::new();
//! world.spawn((Position(0.0), Velocity(1.0)));
//!
//! let mut schedule = Schedule::new();
//! schedule.add_system(integrate);
//! schedule
//!     .compile::<SingleThreadedExecutor>(&mut world)
//!     .run(&mut world);
//! ```

pub mod archetype;
pub mod command;
pub mod common;
pub mod component;
pub mod entity;
pub mod events;
pub mod intern;
pub mod label;
pub mod query;
pub mod reflect;
pub mod resource;
pub mod signal;
pub mod system;
pub mod table;
pub mod utilities;
pub mod utils;
pub mod world;
pub mod world_id;

pub use command::CommandQueue;
pub use component::Component;
pub use entity::{Entity, EntityWorldMut};
pub use events::Event;
pub use query::{
    Query,
    change_detection::Ref,
    filter::{Added, Changed, Or, With, Without},
};
pub use resource::{Res, ResMut, Resource};
pub use system::{
    IntoSetConfig, IntoSystem, IntoSystemConfig, System, SystemConfig, SystemSet, SystemTag,
    TagFilter, schedule::Schedule,
};
pub use world::World;

#[cfg(test)]
mod change_tick_tests;

#[cfg(test)]
mod testing;

#[cfg(test)]
mod tests {
    use crate::{
        component::{Component, ComponentId},
        entity::Entity,
        events::{
            Event, event_channel::EventChannel, event_reader::EventReader,
            event_writer::EventWriter,
        },
        query::{
            Query,
            filter::{Added, Or, QueryFilter, With, Without},
        },
        resource::{ResMut, Resource},
        system::{
            executor::single_thread::SingleThreadedExecutor,
            schedule::{CompiledSchedule, Schedule},
        },
        world::World,
    };

    #[derive(Component)]
    struct Health;

    #[derive(Component)]
    struct Position;

    #[derive(Component)]
    struct Armour;

    fn count<F: QueryFilter>(world: &mut World) -> usize {
        world.query::<Entity, F>().iter(world).count()
    }

    #[test]
    fn presence_filters_select_entities_by_their_components() {
        let mut world = World::new();
        world.spawn((Health, Position));
        world.spawn(Position);
        world.spawn(Health);
        world.spawn(());

        assert_eq!(count::<With<Health>>(&mut world), 2);
        assert_eq!(count::<Without<Health>>(&mut world), 2);
        assert_eq!(count::<(With<Health>, With<Position>)>(&mut world), 1);
        assert_eq!(count::<Or<(With<Health>, With<Position>)>>(&mut world), 3);
    }

    #[test]
    fn a_migration_keeps_the_ticks_of_the_components_it_carries() {
        let mut world = World::new();
        world.tick();
        let entity = world.spawn(Health);
        world.insert(Position, entity);

        assert_eq!(count::<Added<Health>>(&mut world), 1);
        assert!(world.was_component_added(entity, ComponentId::of::<Health>()));

        world.tick();
        world.insert(Armour, entity);

        assert_eq!(count::<Added<Health>>(&mut world), 0);
        assert!(!world.was_component_added(entity, ComponentId::of::<Health>()));
        assert!(!world.was_component_changed(entity, ComponentId::of::<Health>()));
        assert!(!world.was_component_added(entity, ComponentId::of::<Position>()));
        assert!(world.was_component_added(entity, ComponentId::of::<Armour>()));
        assert!(world.was_component_changed(entity, ComponentId::of::<Armour>()));
    }

    #[test]
    fn get_entity_finds_only_live_entities_the_query_matches() {
        let mut world = World::new();
        let with = world.spawn((Health, Position));
        let without = world.spawn(Position);
        let despawned = world.spawn(Health);
        world.despawn(despawned);

        let mut healths = world.query::<&Health, ()>();
        let mut entities = world.query::<Entity, ()>();
        let ticks = world.out_of_system_ticks();
        let cell = world.as_unsafe_world_cell();
        // SAFETY: both queries only read, and nothing writes `world` while they live.
        let healths = unsafe { Query::new(cell, &mut healths, ticks) };
        // SAFETY: both queries only read, and nothing writes `world` while they live.
        let entities = unsafe { Query::new(cell, &mut entities, ticks) };

        assert!(healths.get_entity(with).is_some());
        assert!(healths.get_entity(without).is_none());
        assert_eq!(entities.get_entity(with), Some(with));
        assert_eq!(entities.get_entity(despawned), None);
    }

    #[derive(Event)]
    struct PlayerDied {
        score: u32,
    }

    #[derive(Resource, Default)]
    struct FirstReader(u32);

    #[derive(Resource, Default)]
    struct SecondReader(u32);

    fn send_death(mut writer: EventWriter<PlayerDied>) {
        writer.write(PlayerDied { score: 77 });
    }

    fn first_reader(mut reader: EventReader<PlayerDied>, mut total: ResMut<FirstReader>) {
        for event in reader.read() {
            total.0 += event.score;
        }
    }

    fn second_reader(mut reader: EventReader<PlayerDied>, mut total: ResMut<SecondReader>) {
        for event in reader.read() {
            total.0 += event.score;
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Frame {
        Write,
        Swap,
        ReadFirst,
        ReadSecond,
    }

    struct EventCase {
        name: &'static str,
        frames: &'static [Frame],
        first: u32,
        second: u32,
    }

    fn compiled<M>(world: &mut World, system: impl crate::IntoSystemConfig<M>) -> CompiledSchedule {
        let mut schedule = Schedule::new();
        schedule.add_system(system);
        schedule.compile::<SingleThreadedExecutor>(world)
    }

    #[test]
    fn a_reader_sees_each_event_once_until_two_swaps_expire_it() {
        use Frame::*;
        let cases = [
            EventCase {
                name: "read in the frame it is written",
                frames: &[Write, ReadFirst],
                first: 77,
                second: 0,
            },
            EventCase {
                name: "read after one swap",
                frames: &[Write, Swap, ReadFirst],
                first: 77,
                second: 0,
            },
            EventCase {
                name: "expired after two swaps",
                frames: &[Write, Swap, Swap, ReadFirst],
                first: 0,
                second: 0,
            },
            EventCase {
                name: "never read twice",
                frames: &[Write, ReadFirst, Swap, ReadFirst],
                first: 77,
                second: 0,
            },
            EventCase {
                name: "each reader has its own cursor",
                frames: &[Write, ReadFirst, ReadSecond],
                first: 77,
                second: 77,
            },
            EventCase {
                name: "a reader that skips frames resumes at the oldest buffered event",
                frames: &[Write, Swap, Write, Swap, Write, Swap, ReadFirst],
                first: 77,
                second: 0,
            },
            EventCase {
                name: "written after the read, seen the next frame",
                frames: &[ReadFirst, Write, Swap, ReadFirst],
                first: 77,
                second: 0,
            },
        ];
        for case in cases {
            let mut world = World::new();
            world.insert_resource(EventChannel::<PlayerDied>::new());
            world.insert_resource(FirstReader::default());
            world.insert_resource(SecondReader::default());
            let mut write = compiled(&mut world, send_death);
            let mut swap = compiled(
                &mut world,
                crate::events::event_channel::update_event_channel::<PlayerDied>,
            );
            let mut read_first = compiled(&mut world, first_reader);
            let mut read_second = compiled(&mut world, second_reader);

            for frame in case.frames {
                match frame {
                    Write => write.run(&mut world),
                    Swap => swap.run(&mut world),
                    ReadFirst => read_first.run(&mut world),
                    ReadSecond => read_second.run(&mut world),
                }
            }

            assert_eq!(
                world.get_resource::<FirstReader>().unwrap().0,
                case.first,
                "{}",
                case.name
            );
            assert_eq!(
                world.get_resource::<SecondReader>().unwrap().0,
                case.second,
                "{}",
                case.name
            );
        }
    }
}
