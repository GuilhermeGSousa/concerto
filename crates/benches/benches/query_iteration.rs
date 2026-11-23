use criterion::{black_box, criterion_group, criterion_main, Criterion};

use concerto_ecs::{
    Changed, Component, Entity, IntoSystem, Query, Res, Resource, System, With, World,
};

const ENTITIES: usize = 100_000;
const ARCHETYPES: usize = 100;
const LOOKUPS: usize = 10_000;

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Velocity(f32);

#[derive(Component)]
struct Marker;

#[derive(Component)]
struct Shape1;

#[derive(Component)]
struct Shape2;

#[derive(Component)]
struct Shape3;

#[derive(Component)]
struct Shape4;

#[derive(Component)]
struct Shape5;

#[derive(Component)]
struct Shape6;

#[derive(Resource)]
struct Targets(Vec<Entity>);

fn spawn_in_shape(world: &mut World, shape: usize, value: f32) -> Entity {
    let entity = world.spawn((Position(value), Velocity(1.0)));
    if shape & 1 != 0 {
        world.insert(Marker, entity);
    }
    if shape & 2 != 0 {
        world.insert(Shape1, entity);
    }
    if shape & 4 != 0 {
        world.insert(Shape2, entity);
    }
    if shape & 8 != 0 {
        world.insert(Shape3, entity);
    }
    if shape & 16 != 0 {
        world.insert(Shape4, entity);
    }
    if shape & 32 != 0 {
        world.insert(Shape5, entity);
    }
    if shape & 64 != 0 {
        world.insert(Shape6, entity);
    }
    entity
}

fn world_of(archetypes: usize) -> World {
    let mut world = World::new();
    let per_archetype = ENTITIES / archetypes;
    let mut entities = Vec::with_capacity(ENTITIES);
    for shape in 0..archetypes {
        for index in 0..per_archetype {
            entities.push(spawn_in_shape(&mut world, shape, index as f32));
        }
    }
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let targets = (0..LOOKUPS)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            entities[(state >> 33) as usize % entities.len()]
        })
        .collect();
    world.insert_resource(Targets(targets));
    world
}

fn integrate(mut bodies: Query<(&Velocity, &mut Position)>) {
    for (velocity, mut position) in bodies.iter_mut() {
        position.0 += velocity.0;
    }
}

fn sum_marked(bodies: Query<&Velocity, With<Marker>>) {
    black_box(bodies.iter().map(|velocity| velocity.0).sum::<f32>());
}

fn sum_changed(bodies: Query<&Velocity, Changed<Velocity>>) {
    black_box(bodies.iter().map(|velocity| velocity.0).sum::<f32>());
}

fn look_up(bodies: Query<&Velocity>, targets: Res<Targets>) {
    let sum = targets
        .0
        .iter()
        .filter_map(|&entity| bodies.get_entity(entity))
        .map(|velocity| velocity.0)
        .sum::<f32>();
    black_box(sum);
}

fn started<M>(world: &mut World, system: impl IntoSystem<(), M>) -> Box<dyn System<In = ()>> {
    let mut system = system.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
    Box::new(system)
}

fn query_iteration(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_iteration");

    group.bench_function("read_and_write_one_archetype", |b| {
        let mut world = world_of(1);
        let mut system = started(&mut world, integrate);
        b.iter(|| system.run_and_apply((), &mut world));
    });

    group.bench_function("read_and_write_100_archetypes", |b| {
        let mut world = world_of(ARCHETYPES);
        let mut system = started(&mut world, integrate);
        b.iter(|| system.run_and_apply((), &mut world));
    });

    group.bench_function("with_marker_on_half_the_archetypes", |b| {
        let mut world = world_of(ARCHETYPES);
        let mut system = started(&mut world, sum_marked);
        b.iter(|| system.run_and_apply((), &mut world));
    });

    group.bench_function("changed_when_nothing_changed", |b| {
        let mut world = world_of(ARCHETYPES);
        let mut system = started(&mut world, sum_changed);
        b.iter(|| system.run_and_apply((), &mut world));
    });

    group.bench_function("get_entity_10000_random", |b| {
        let mut world = world_of(ARCHETYPES);
        let mut system = started(&mut world, look_up);
        b.iter(|| system.run_and_apply((), &mut world));
    });

    group.finish();
}

criterion_group!(benches, query_iteration);
criterion_main!(benches);
