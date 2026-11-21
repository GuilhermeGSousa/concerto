//! Component lifecycle callbacks: when they fire, what they can see, and
//! whether structural changes made *inside* callbacks (inserts, removals,
//! despawns) leave every other entity's location intact.

use concerto_ecs::component::{Component, ComponentLifecycleCallback};
use concerto_ecs::entity::Entity;
use concerto_ecs::resource::Resource;
use concerto_ecs::world::World;

#[derive(Component)]
struct Value(i32);

#[derive(Component)]
struct Extra;

#[derive(Component)]
struct Marker;

#[derive(Debug, PartialEq)]
struct Firing {
    entity: Entity,
    readable: bool,
}

#[derive(Resource, Default)]
struct TrackLog {
    added: Vec<Firing>,
    removed: Vec<Firing>,
}

struct Tracked;

impl Component for Tracked {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let readable = world
                .get_component_for_entity::<Tracked>(context.entity)
                .is_some();
            if let Some(log) = world.get_resource_mut::<TrackLog>() {
                log.added.push(Firing {
                    entity: context.entity,
                    readable,
                });
            }
        })
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let readable = world
                .get_component_for_entity::<Tracked>(context.entity)
                .is_some();
            if let Some(log) = world.get_resource_mut::<TrackLog>() {
                log.removed.push(Firing {
                    entity: context.entity,
                    readable,
                });
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
        vec![
            Firing {
                entity: first,
                readable: true
            },
            Firing {
                entity: second,
                readable: true
            }
        ],
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

struct TrackCase {
    name: &'static str,
    act: fn(&mut World) -> Vec<Entity>,
    added: &'static [usize],
    removed: &'static [usize],
}

#[test]
fn add_and_remove_callbacks_fire_once_each_while_the_component_is_readable() {
    let cases = [
        TrackCase {
            name: "insert fires on_add",
            act: |world| {
                let entity = world.spawn(Value(1));
                world.insert(Tracked, entity);
                vec![entity]
            },
            added: &[0],
            removed: &[],
        },
        TrackCase {
            name: "mixed operations balance",
            act: |world| {
                let entities = vec![
                    world.spawn((Tracked, Value(1))),
                    world.spawn((Tracked, Value(2))),
                    world.spawn((Tracked, Value(3))),
                ];
                world.remove_component::<Tracked>(entities[0]);
                world.despawn(entities[2]);
                entities
            },
            added: &[0, 1, 2],
            removed: &[0, 2],
        },
    ];
    for case in cases {
        let mut world = World::new();
        world.register_component::<Tracked>();
        world.insert_resource(TrackLog::default());

        let entities = (case.act)(&mut world);

        let firings = |indices: &[usize]| -> Vec<Firing> {
            indices
                .iter()
                .map(|index| Firing {
                    entity: entities[*index],
                    readable: true,
                })
                .collect()
        };
        let log = world.get_resource::<TrackLog>().unwrap();
        assert_eq!(log.added, firings(case.added), "{}", case.name);
        assert_eq!(log.removed, firings(case.removed), "{}", case.name);
    }
}

#[test]
fn on_replace_reads_the_old_value_only_when_a_value_is_overwritten() {
    let mut world = World::new();
    world.register_component::<Replaceable>();
    world.insert_resource(ReplaceLog::default());

    let entity = world.spawn(Replaceable(1));
    world.insert((Extra, Marker), entity);
    assert!(world.get_resource::<ReplaceLog>().unwrap().0.is_empty());

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
