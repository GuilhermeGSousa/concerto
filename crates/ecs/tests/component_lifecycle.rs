//! Component lifecycle callbacks: when they fire, what they can see, and
//! whether structural changes made *inside* callbacks (inserts, removals,
//! despawns) leave every other entity's location intact.

use ecs::component::{Component, ComponentLifecycleCallback};
use ecs::entity::Entity;
use ecs::resource::Resource;
use ecs::world::World;

#[derive(Component)]
struct Value(i32);

#[derive(Component)]
struct Extra;

#[derive(Component)]
struct Marker;

/// Records every lifecycle firing and whether the component could read
/// itself at that moment.
#[derive(Resource, Default)]
struct TrackLog {
    added: Vec<(Entity, bool)>,
    removed: Vec<(Entity, bool)>,
}

struct Tracked;

impl Component for Tracked {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let readable = world
                .get_component_for_entity::<Tracked>(context.entity)
                .is_some();
            if let Some(log) = world.get_resource_mut::<TrackLog>() {
                log.added.push((context.entity, readable));
            }
        })
    }


    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let readable = world
                .get_component_for_entity::<Tracked>(context.entity)
                .is_some();
            if let Some(log) = world.get_resource_mut::<TrackLog>() {
                log.removed.push((context.entity, readable));
            }
        })
    }
}

#[derive(Resource, Default)]
struct SiblingLog(Vec<bool>);

/// Records whether the sibling `Marker` was visible from `on_add`.
struct WithSibling;

impl Component for WithSibling {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let sibling = world
                .get_component_for_entity::<Marker>(context.entity)
                .is_some();
            if let Some(log) = world.get_resource_mut::<SiblingLog>() {
                log.0.push(sibling);
            }
        })
    }
}

#[derive(Resource, Default)]
struct CompanionLog {
    adds: u32,
    removes: u32,
}

struct Companion;

impl Component for Companion {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _context| {
            if let Some(log) = world.get_resource_mut::<CompanionLog>() {
                log.adds += 1;
            }
        })
    }


    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _context| {
            if let Some(log) = world.get_resource_mut::<CompanionLog>() {
                log.removes += 1;
            }
        })
    }
}

/// Inserts a `Companion` from `on_add` and removes it from `on_remove`. During a
/// despawn the `Companion` gets one `on_despawn` firing from despawn's own pass
/// over the component list captured at despawn start. Its queued removal is a
/// no-op once the entity has been despawned.
struct Body;

impl Component for Body {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.insert(Companion, context.entity);
        })
    }


    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<Companion>(context.entity);
        })
    }
}

#[derive(Resource, Default)]
struct DespawnTarget(Option<Entity>);

/// Despawns the entity named in `DespawnTarget` (taken, so nested firings
/// see `None`) from `on_remove`.
struct Reaper;

impl Component for Reaper {

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _context| {
            let target = world
                .get_resource_mut::<DespawnTarget>()
                .and_then(|target| target.0.take());
            if let Some(target) = target {
                world.despawn(target);
            }
        })
    }
}

fn value_of(world: &World, entity: Entity) -> Option<i32> {
    world
        .get_component_for_entity::<Value>(entity)
        .map(|value| value.0)
}

#[test]
fn spawn_fires_on_add_with_bundle_fully_inserted() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.register_component::<WithSibling>();
    world.insert_resource(TrackLog::default());
    world.insert_resource(SiblingLog::default());

    let first = world.spawn((WithSibling, Tracked, Marker));
    let second = world.spawn((Marker, Tracked, WithSibling));

    let log = world.get_resource::<TrackLog>().unwrap();
    assert_eq!(
        log.added,
        vec![(first, true), (second, true)],
        "on_add should fire once per spawn, for the right entity, with the component readable"
    );

    let siblings = world.get_resource::<SiblingLog>().unwrap();
    assert_eq!(
        siblings.0,
        vec![true, true],
        "the whole bundle should be inserted before any on_add fires, regardless of bundle order"
    );
}

#[test]
fn insert_fires_on_add() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.insert_resource(TrackLog::default());

    let entity = world.spawn((Value(1),));
    world.insert(Tracked, entity);

    let log = world.get_resource::<TrackLog>().unwrap();
    assert_eq!(log.added, vec![(entity, true)]);
}

#[test]
fn remove_component_fires_on_remove_before_removal() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.insert_resource(TrackLog::default());

    let entity = world.spawn((Tracked, Value(1)));
    world.remove_component::<Tracked>(entity);

    let log = world.get_resource::<TrackLog>().unwrap();
    assert_eq!(
        log.removed,
        vec![(entity, true)],
        "remove_component fires on_remove while the outgoing component is readable"
    );
    assert!(world.get_component_for_entity::<Tracked>(entity).is_none());
}

#[test]
fn despawn_fires_on_despawn_while_still_readable() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.insert_resource(TrackLog::default());

    let entity = world.spawn((Tracked, Value(1)));
    world.despawn(entity);

    let log = world.get_resource::<TrackLog>().unwrap();
    assert_eq!(
        log.removed,
        vec![(entity, true)],
        "despawn fires on_despawn while the component is still readable"
    );
}

#[test]
fn on_add_can_insert_a_companion_and_trigger_its_callbacks() {
    let mut world = World::new();
    world.register_component::<Body>();
    world.register_component::<Companion>();
    world.insert_resource(CompanionLog::default());

    let entity = world.spawn((Body, Value(1)));

    assert!(
        world
            .get_component_for_entity::<Companion>(entity)
            .is_some(),
        "Body::on_add should have inserted the Companion"
    );
    assert_eq!(value_of(&world, entity), Some(1));
    let log = world.get_resource::<CompanionLog>().unwrap();
    assert_eq!(
        log.adds, 1,
        "the nested insert should fire Companion::on_add exactly once"
    );
}

#[test]
fn on_despawn_removing_a_sibling_component() {
    let mut world = World::new();
    world.register_component::<Body>();
    world.register_component::<Companion>();
    world.insert_resource(CompanionLog::default());

    // Two entities share the archetype so the swap-remove paths engage.
    let doomed = world.spawn((Body, Value(1)));
    let survivor = world.spawn((Body, Value(2)));

    // Body::on_despawn queues Companion removal, which must not prevent its
    // own despawn callback from running during the complete hook pass.
    world.despawn(doomed);

    assert_eq!(
        value_of(&world, doomed),
        None,
        "doomed entity should be gone"
    );
    assert_eq!(
        value_of(&world, survivor),
        Some(2),
        "the surviving entity's location must not be corrupted by the mid-despawn migration"
    );
    let log = world.get_resource::<CompanionLog>().unwrap();
    assert_eq!(log.adds, 2);
    assert_eq!(
        log.removes, 1,
        "despawn fires on_despawn once per component; queued cleanup of the \
         already-despawned entity must not fire again"
    );
}

#[test]
fn on_despawn_despawning_another_entity() {
    let mut world = World::new();
    world.register_component::<Reaper>();
    world.insert_resource(DespawnTarget::default());

    // The victim sits in row 0; the reaper is the archetype's last row, so
    // the victim's swap-removal moves the reaper mid-despawn.
    let victim = world.spawn((Reaper, Value(1)));
    let reaper = world.spawn((Reaper, Value(2)));
    world.get_resource_mut::<DespawnTarget>().unwrap().0 = Some(victim);

    world.despawn(reaper);

    assert_eq!(value_of(&world, victim), None, "victim should be despawned");
    assert_eq!(value_of(&world, reaper), None, "reaper should be despawned");
}

#[test]
fn noop_remove_of_missing_component_is_harmless() {
    let mut world = World::new();

    // `first` is not the archetype's last row: a buggy no-op removal would
    // clobber `last`'s location with `first`'s.
    let first = world.spawn((Value(1), Marker));
    let last = world.spawn((Value(2), Marker));

    world.remove_component::<Extra>(first);

    assert_eq!(value_of(&world, first), Some(1));
    assert_eq!(value_of(&world, last), Some(2));

    world.get_component_for_entity_mut::<Value>(last).unwrap().0 = 20;
    assert_eq!(
        value_of(&world, first),
        Some(1),
        "mutating the last entity must not write through a corrupted location"
    );
    assert_eq!(value_of(&world, last), Some(20));
}

#[test]
fn remove_component_keeps_swapped_entity_intact() {
    let mut world = World::new();

    let first = world.spawn((Value(1), Extra));
    let last = world.spawn((Value(2), Extra));

    world.remove_component::<Extra>(first);

    assert_eq!(value_of(&world, first), Some(1));
    assert!(world.get_component_for_entity::<Extra>(first).is_none());
    assert_eq!(value_of(&world, last), Some(2));
    assert!(world.get_component_for_entity::<Extra>(last).is_some());
}

#[test]
fn despawn_keeps_swapped_entity_intact() {
    let mut world = World::new();

    let first = world.spawn((Value(1), Extra));
    let last = world.spawn((Value(2), Extra));

    world.despawn(first);

    assert_eq!(value_of(&world, first), None);
    assert_eq!(value_of(&world, last), Some(2));
}

#[test]
fn callbacks_balance_across_mixed_operations() {
    let mut world = World::new();
    world.register_component::<Tracked>();
    world.insert_resource(TrackLog::default());

    let a = world.spawn((Tracked, Value(1)));
    let _b = world.spawn((Tracked, Value(2)));
    let c = world.spawn((Tracked, Value(3)));

    world.remove_component::<Tracked>(a);
    world.despawn(c);

    let log = world.get_resource::<TrackLog>().unwrap();
    assert_eq!(log.added.len(), 3);
    assert_eq!(
        log.removed.len(),
        2,
        "one remove_component + one despawn should each run cleanup exactly once"
    );
}

#[test]
fn on_remove_can_queue_companion_removal() {
    let mut world = World::new();
    world.register_component::<Body>();
    world.register_component::<Companion>();
    world.insert_resource(CompanionLog::default());
    let removed = world.spawn((Body, Value(1)));
    let survivor = world.spawn((Body, Value(2)));

    world.remove_component::<Body>(removed);

    assert!(world.get_component_for_entity::<Body>(removed).is_none());
    assert!(
        world
            .get_component_for_entity::<Companion>(removed)
            .is_none()
    );
    assert_eq!(value_of(&world, removed), Some(1));
    assert_eq!(value_of(&world, survivor), Some(2));
    assert!(world.get_component_for_entity::<Body>(survivor).is_some());
    assert!(
        world
            .get_component_for_entity::<Companion>(survivor)
            .is_some()
    );
    assert_eq!(
        world.get_resource::<CompanionLog>().unwrap().removes,
        1,
        "the companion removal queued from on_remove fires its own callback"
    );
}

#[test]
fn on_remove_can_queue_despawning_another_entity() {
    let mut world = World::new();
    world.register_component::<Reaper>();
    world.insert_resource(DespawnTarget::default());
    let victim = world.spawn((Reaper, Value(1)));
    let reaper = world.spawn((Reaper, Value(2)));
    world.get_resource_mut::<DespawnTarget>().unwrap().0 = Some(victim);

    world.remove_component::<Reaper>(reaper);

    assert!(!world.entity_is_valid(victim));
    assert_eq!(value_of(&world, reaper), Some(2));
    assert!(world.get_component_for_entity::<Reaper>(reaper).is_none());
}

#[test]
fn on_remove_can_queue_component_insertion() {
    struct LeavesMarker;
    impl Component for LeavesMarker {
        fn on_remove() -> Option<ComponentLifecycleCallback> {
            Some(|mut world, context| {
                assert!(
                    world
                        .get_component_for_entity::<Self>(context.entity)
                        .is_some()
                );
                world.insert(Marker, context.entity);
            })
        }
    }

    let mut world = World::new();
    world.register_component::<LeavesMarker>();
    let removed = world.spawn((LeavesMarker, Value(1)));
    let survivor = world.spawn((LeavesMarker, Value(2)));

    world.remove_component::<LeavesMarker>(removed);

    assert!(
        world
            .get_component_for_entity::<LeavesMarker>(removed)
            .is_none()
    );
    assert!(world.get_component_for_entity::<Marker>(removed).is_some());
    assert_eq!(value_of(&world, removed), Some(1));
    assert_eq!(value_of(&world, survivor), Some(2));
    assert!(
        world
            .get_component_for_entity::<LeavesMarker>(survivor)
            .is_some()
    );
    assert!(world.get_component_for_entity::<Marker>(survivor).is_none());
}

#[test]
fn on_remove_can_remove_itself_and_drop_once() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct RemovesItself(Arc<AtomicUsize>);
    impl Drop for RemovesItself {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Component for RemovesItself {
        fn on_remove() -> Option<ComponentLifecycleCallback> {
            Some(|mut world, context| {
                let component = world
                    .get_component_for_entity::<Self>(context.entity)
                    .unwrap();
                assert_eq!(component.0.load(Ordering::SeqCst), 0);
                world.remove_component::<Self>(context.entity);
            })
        }
    }

    let mut world = World::new();
    world.register_component::<RemovesItself>();
    let drops = Arc::new(AtomicUsize::new(0));
    let entity = world.spawn((RemovesItself(drops.clone()), Value(1)));

    world.remove_component::<RemovesItself>(entity);

    assert!(
        world
            .get_component_for_entity::<RemovesItself>(entity)
            .is_none()
    );
    assert_eq!(value_of(&world, entity), Some(1));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(world);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn on_remove_can_despawn_its_entity() {
    struct DespawnsItself;
    impl Component for DespawnsItself {
        fn on_remove() -> Option<ComponentLifecycleCallback> {
            Some(|mut world, context| {
                world.despawn(context.entity);
            })
        }
    }

    let mut world = World::new();
    world.register_component::<DespawnsItself>();
    let entity = world.spawn((DespawnsItself, Value(1)));
    let survivor = world.spawn((DespawnsItself, Value(2)));

    world.remove_component::<DespawnsItself>(entity);

    assert!(!world.entity_is_valid(entity));
    assert_eq!(value_of(&world, survivor), Some(2));
    assert!(
        world
            .get_component_for_entity::<DespawnsItself>(survivor)
            .is_some()
    );
}

/// Records the value each `on_replace` could still read before being overwritten.
#[derive(Resource, Default)]
struct ReplaceLog(Vec<i32>);

struct Replaceable(i32);

impl Component for Replaceable {
    fn on_replace() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let old = world
                .get_component_for_entity::<Replaceable>(context.entity)
                .map(|value| value.0)
                .expect("the old value must still be readable");
            if let Some(log) = world.get_resource_mut::<ReplaceLog>() {
                log.0.push(old);
            }
        })
    }
}

#[test]
fn on_replace_reads_the_old_value_and_skips_the_first_insert() {
    let mut world = World::new();
    world.register_component::<Replaceable>();
    world.insert_resource(ReplaceLog::default());

    let entity = world.spawn(Replaceable(1));
    assert!(
        world.get_resource::<ReplaceLog>().unwrap().0.is_empty(),
        "spawning must not replace anything"
    );

    world.insert(Replaceable(2), entity);
    world.insert(Replaceable(3), entity);

    assert_eq!(world.get_resource::<ReplaceLog>().unwrap().0, vec![1, 2]);
    assert_eq!(
        world
            .get_component_for_entity::<Replaceable>(entity)
            .unwrap()
            .0,
        3
    );
}

#[test]
fn on_replace_does_not_fire_for_a_component_added_alongside_an_existing_one() {
    let mut world = World::new();
    world.register_component::<Replaceable>();
    world.insert_resource(ReplaceLog::default());

    let entity = world.spawn(Replaceable(1));
    world.insert((Extra, Marker), entity);

    assert!(world.get_resource::<ReplaceLog>().unwrap().0.is_empty());
}
