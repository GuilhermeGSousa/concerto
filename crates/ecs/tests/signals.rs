use concerto_ecs::{Query, World, signal::{On, Signal}};


#[derive(Default)]
struct TestSignal;

impl Signal for TestSignal {
    
}

#[test]
fn trigger_signal()
{
    let mut world = World::new();

    // This needs to compile
    world.add_listener(|_: On<TestSignal>, a: Query<&Transform>| {});

    world.trigger_default::<TestSignal>();
}