#![allow(missing_docs, reason = "benchmark harness, not public API")]

use criterion::{criterion_group, criterion_main, Criterion};
use glam::Vec3;

use concerto_ecs::{
    entity::hierarchy::{ChildOf, Children},
    Entity, IntoSystem, System, World,
};
use concerto_foundation::transform::{systems::propagate_global_transforms, Transform};

const HIERARCHIES: usize = 100;
const CHILDREN: usize = 9;
const GRANDCHILDREN: usize = 10;

struct Scene {
    world: World,
    entities: Vec<Entity>,
}

fn scene() -> Scene {
    let mut world = World::new();
    world.register_component::<Transform>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();

    let mut entities = Vec::new();
    for _ in 0..HIERARCHIES {
        let root = world.spawn(Transform::from_translation(Vec3::X));
        entities.push(root);
        for _ in 0..CHILDREN {
            let child = world.spawn(Transform::from_translation(Vec3::Y));
            world.entity_mut(root).add_child(child);
            entities.push(child);
            for _ in 0..GRANDCHILDREN {
                let grandchild = world.spawn(Transform::from_translation(Vec3::Z));
                world.entity_mut(child).add_child(grandchild);
                entities.push(grandchild);
            }
        }
    }

    Scene { world, entities }
}

fn settled() -> Settled {
    let mut scene = scene();
    let mut system = propagate_global_transforms.into_system();
    system.initialize(&mut scene.world);
    system.run_and_apply((), &mut scene.world);
    Settled {
        scene,
        system: Box::new(system),
        height: 0.0,
    }
}

struct Settled {
    scene: Scene,
    system: Box<dyn System<In = ()>>,
    height: f32,
}

impl Settled {
    fn run(&mut self) {
        self.system.run_and_apply((), &mut self.scene.world);
    }

    fn raise(&mut self, moved: usize) {
        self.height += 1.0;
        for &entity in self.scene.entities.iter().rev().take(moved) {
            self.scene
                .world
                .get_component_for_entity_mut::<Transform>(entity)
                .expect("the entity has a transform")
                .translation = Vec3::new(0.0, self.height, 0.0);
        }
    }
}

fn transform_propagation(c: &mut Criterion) {
    let mut group = c.benchmark_group("transform_propagation");

    group.bench_function("static_scene", |b| {
        let mut settled = settled();
        b.iter(|| settled.run());
    });

    group.bench_function("one_leaf_moved", |b| {
        let mut settled = settled();
        b.iter(|| {
            settled.raise(1);
            settled.run();
        });
    });

    group.bench_function("every_entity_moved", |b| {
        let mut settled = settled();
        let every = settled.scene.entities.len();
        b.iter(|| {
            settled.raise(every);
            settled.run();
        });
    });

    group.finish();
}

criterion_group!(benches, transform_propagation);
criterion_main!(benches);
