use ecs::{Component, IntoSystem, System, World, command::CommandQueue};

#[derive(Component)]
struct Marker;

/// Recorded by the probe below when the reservation resolved to a live row.
#[derive(Component)]
struct StaleLookup;

/// `CommandQueue::spawn` reserves an entity immediately and spawns it when the
/// queue is applied. A reservation that reused a freed index used to keep the
/// freed entity's location, so reading a component off it during that window
/// returned whatever now occupied that row — a live entity's data handed out
/// under a handle that does not exist yet.
#[test]
fn a_reserved_entity_carries_no_components_before_it_is_spawned() {
    fn probe(world: &World, mut cmd: CommandQueue) {
        let reserved = cmd.spawn(Marker).entity();
        if world.entity_is_valid(reserved)
            || world.get_component_for_entity::<Marker>(reserved).is_some()
        {
            cmd.spawn(StaleLookup);
        }
    }

    let mut world = World::new();
    // Occupy, then free, a batch of indexes for the reservation to recycle.
    let occupants: Vec<_> = (0..8).map(|_| world.spawn(Marker)).collect();
    for entity in occupants {
        world.despawn(entity);
    }

    let mut probe = probe.into_system();
    probe.initialize(&mut world);
    probe.run_and_apply(&mut world);

    let mut stale = world.query::<&StaleLookup, ()>();
    assert_eq!(
        stale.iter(&mut world).count(),
        0,
        "a reserved entity resolved to a recycled index's previous location"
    );
}
