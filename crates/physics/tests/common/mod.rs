//! Shared setup for the physics integration tests.
#![allow(dead_code)] // not every test file uses every helper

use ecs::system::executor::single_thread::SingleThreadedExecutor;
use ecs::system::schedule::Schedule;
use ecs::world::World;
use essential::transform::Transform;
use physics::collider::{register_colliders, Collider};
use physics::physics_state::PhysicsState;

/// A world with the component lifecycles and resources `PhysicsPlugin` sets
/// up in a real app.
///
/// `Transform`'s lifecycle matters as much as `Collider`'s: it creates the
/// `GlobalTransform` that [`register_bodies`] poses bodies from, and without
/// it colliders are skipped with no diagnostic.
pub fn physics_world() -> World {
    let mut world = World::new();
    world.register_component::<Collider>();
    world.register_component::<Transform>();
    world.insert_resource(PhysicsState::new());
    world
}

/// Creates the physics bodies for every `Collider` spawned so far.
///
/// Bodies come from the `register_colliders` system rather than `Collider`'s
/// `on_add`, so spawning a `Collider` alone leaves the entity without a
/// `BodyId`. Call this after spawning and before stepping or querying.
pub fn register_bodies(world: &mut World) {
    let mut schedule = Schedule::new();
    schedule.add_system(register_colliders);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);
}
