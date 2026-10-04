use concerto_ecs::{
    Component, Query, World,
    signal::{On, Signal},
};

#[derive(Default)]
struct TestSignal;

impl Signal for TestSignal {}

#[derive(Component)]
struct TestComponent;

#[test]
fn trigger_signal() {
    let mut world = World::new();

    // This needs to compile
    world.add_listener(|_: On<TestSignal>, a: Query<&TestComponent>| {});

    world.trigger_default::<TestSignal>();
}
