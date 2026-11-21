use std::sync::atomic::{AtomicIsize, Ordering};

use concerto_ecs::{Component, Entity, World};

static LIVE: AtomicIsize = AtomicIsize::new(0);

#[derive(Component)]
struct Tracked(u32);

impl Tracked {
    fn new(value: u32) -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Tracked(value)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Health;

#[derive(Debug, PartialEq)]
struct Row {
    tracked: Option<u32>,
    position: Option<f32>,
}

fn row(tracked: Option<u32>, position: Option<f32>) -> Row {
    Row { tracked, position }
}

struct StorageCase {
    name: &'static str,
    spawn: fn(&mut World) -> Vec<Entity>,
    act: fn(&mut World, &[Entity]),
    rows: Vec<Row>,
    live: isize,
    with_both: usize,
}

fn read(world: &World, entity: Entity) -> Row {
    row(
        world
            .get_component_for_entity::<Tracked>(entity)
            .map(|tracked| tracked.0),
        world
            .get_component_for_entity::<Position>(entity)
            .map(|position| position.0),
    )
}

#[test]
fn structural_changes_keep_every_row_and_drop_each_value_once() {
    let cases = [
        StorageCase {
            name: "despawn",
            spawn: |world| {
                vec![
                    world.spawn((Tracked::new(1), Position(1.0))),
                    world.spawn((Tracked::new(2), Position(3.0))),
                ]
            },
            act: |world, entities| {
                world.despawn(entities[0]);
                world.spawn((Tracked::new(9), Position(9.0)));
            },
            rows: vec![row(None, None), row(Some(2), Some(3.0))],
            live: 2,
            with_both: 2,
        },
        StorageCase {
            name: "replacing drops the displaced value",
            spawn: |world| vec![world.spawn(Tracked::new(1))],
            act: |world, entities| world.insert(Tracked::new(2), entities[0]),
            rows: vec![row(Some(2), None)],
            live: 1,
            with_both: 0,
        },
        StorageCase {
            name: "removing drops the component",
            spawn: |world| vec![world.spawn((Tracked::new(1), Position(1.0)))],
            act: |world, entities| world.remove_component::<Tracked>(entities[0]),
            rows: vec![row(None, Some(1.0))],
            live: 0,
            with_both: 0,
        },
        StorageCase {
            name: "a bundle that replaces one component and adds another",
            spawn: |world| vec![world.spawn(Tracked::new(1))],
            act: |world, entities| world.insert((Tracked::new(2), Position(5.0)), entities[0]),
            rows: vec![row(Some(2), Some(5.0))],
            live: 1,
            with_both: 1,
        },
        StorageCase {
            name: "a middle row migrates out",
            spawn: |world| (0..4).map(|i| world.spawn(Position(i as f32))).collect(),
            act: |world, entities| {
                world.insert(Health, entities[1]);
                world.spawn(Position(9.0));
            },
            rows: vec![
                row(None, Some(0.0)),
                row(None, Some(1.0)),
                row(None, Some(2.0)),
                row(None, Some(3.0)),
            ],
            live: 0,
            with_both: 0,
        },
        StorageCase {
            name: "removing from a row that is not the last",
            spawn: |world| {
                vec![
                    world.spawn((Tracked::new(1), Position(1.0))),
                    world.spawn((Tracked::new(2), Position(2.0))),
                ]
            },
            act: |world, entities| {
                world.remove_component::<Position>(entities[0]);
                world.spawn((Tracked::new(9), Position(9.0)));
            },
            rows: vec![row(Some(1), None), row(Some(2), Some(2.0))],
            live: 3,
            with_both: 2,
        },
    ];
    for case in cases {
        LIVE.store(0, Ordering::SeqCst);
        let mut world = World::new();
        let entities = (case.spawn)(&mut world);

        (case.act)(&mut world, &entities);

        let rows: Vec<Row> = entities
            .iter()
            .map(|entity| read(&world, *entity))
            .collect();
        assert_eq!(rows, case.rows, "{}", case.name);
        assert_eq!(LIVE.load(Ordering::SeqCst), case.live, "{}", case.name);
        let with_both = world
            .query::<(&Tracked, &Position), ()>()
            .iter(&mut world)
            .count();
        assert_eq!(with_both, case.with_both, "{}", case.name);
        drop(world);
        assert_eq!(LIVE.load(Ordering::SeqCst), 0, "{}", case.name);
    }
}
