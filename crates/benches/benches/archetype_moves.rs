#![allow(missing_docs, reason = "benchmark harness, not public API")]

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

use concerto_ecs::{Component, Entity, World};

const ENTITIES: usize = 10_000;

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Velocity(f32);

#[derive(Component)]
struct Marker;

fn spawn_all(world: &mut World) -> Vec<Entity> {
    (0..ENTITIES)
        .map(|index| world.spawn((Position(index as f32), Velocity(1.0))))
        .collect()
}

fn reach(world: &World, entity: Entity) -> Option<f32> {
    let position = world.get_component_for_entity::<Position>(entity)?;
    let velocity = world.get_component_for_entity::<Velocity>(entity)?;
    Some(position.0 + velocity.0)
}

fn archetype_moves(c: &mut Criterion) {
    let mut group = c.benchmark_group("archetype_moves");

    group.bench_function("insert_then_remove_marker", |b| {
        let mut world = World::new();
        let entities = spawn_all(&mut world);
        b.iter(|| {
            for &entity in &entities {
                world.insert(Marker, entity);
            }
            for &entity in &entities {
                world.remove_component::<Marker>(entity);
            }
            black_box(reach(&world, entities[0]));
        });
    });

    group.bench_function("spawn_one_shape", |b| {
        b.iter_batched(
            World::new,
            |mut world| {
                spawn_all(&mut world);
                world
            },
            BatchSize::LargeInput,
        );
    });

    group.finish();
}

criterion_group!(benches, archetype_moves);
criterion_main!(benches);
