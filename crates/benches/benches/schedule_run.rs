use criterion::{black_box, criterion_group, criterion_main, Criterion};

use concerto_ecs::{
    system::{
        executor::multi_thread::{MainThreadExecutor, MultiThreadedExecutor},
        schedule::CompiledSchedule,
    },
    CommandQueue, Component, Entity, IntoSystemConfig, Query, Res, Resource, Schedule, World,
};

const ENTITIES: usize = 1_000;

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Velocity(f32);

#[derive(Component)]
struct Marker;

#[derive(Resource)]
struct Target(Entity);

fn sum(query: &Query<(&Position, &Velocity)>) -> f32 {
    query
        .iter()
        .map(|(position, velocity)| position.0 + velocity.0)
        .sum()
}

fn read<const N: usize>(query: Query<(&Position, &Velocity)>) {
    black_box(sum(&query));
}

fn queue<const N: usize>(
    query: Query<(&Position, &Velocity)>,
    target: Res<Target>,
    mut cmd: CommandQueue,
) {
    black_box(sum(&query));
    if N % 4 == 0 {
        cmd.insert(Marker, target.0);
    }
}

macro_rules! add_pairs {
    ($schedule:ident; $($n:literal)*) => {
        $($schedule.add_system(read::<$n>).add_system(queue::<$n>);)*
    };
}

fn schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add_system((read::<0>, queue::<0>, read::<1>, queue::<1>).chain());
    schedule.add_system((queue::<2>, queue::<3>, read::<2>).chain());
    schedule.add_system((read::<3>, queue::<4>, read::<4>).chain());
    add_pairs!(schedule; 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31);
    schedule
}

fn compiled(world: &mut World) -> CompiledSchedule {
    world.insert_resource(MainThreadExecutor::new());
    for index in 0..ENTITIES {
        world.spawn((Position(index as f32), Velocity(1.0)));
    }
    let target = world.spawn(Position(0.0));
    world.insert_resource(Target(target));
    schedule().compile::<MultiThreadedExecutor>(world)
}

fn schedule_run(c: &mut Criterion) {
    let mut world = World::new();
    let mut schedule = compiled(&mut world);
    c.bench_function("schedule_run", |b| b.iter(|| schedule.run(&mut world)));
}

criterion_group!(benches, schedule_run);
criterion_main!(benches);
