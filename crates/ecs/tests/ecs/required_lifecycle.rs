use concerto_ecs::{
    Query, ResMut, Resource, World,
    component::{Component, ComponentLifecycleCallback, RequiredComponents},
    signal::{On, lifecycle::Add},
    world::RestrictedWorld,
};

#[derive(Resource, Default)]
struct Log(Vec<String>);

fn log(world: &mut RestrictedWorld, line: String) {
    world.get_resource_mut::<Log>().unwrap().0.push(line);
}

#[derive(Default)]
struct Deep;

#[derive(Default)]
struct Mid;

#[derive(Default)]
struct Top;

impl Component for Deep {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, _| log(&mut world, "Deep hook".into()))
    }
}

impl Component for Mid {
    fn register_required(required: &mut RequiredComponents) {
        required.register::<Deep>(Deep::default);
    }

    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let deep = world
                .get_component_for_entity::<Deep>(context.entity)
                .is_some();
            log(&mut world, format!("Mid hook deep={deep}"));
        })
    }
}

impl Component for Top {
    fn register_required(required: &mut RequiredComponents) {
        required.register::<Mid>(Mid::default);
    }

    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let mid = world
                .get_component_for_entity::<Mid>(context.entity)
                .is_some();
            let deep = world
                .get_component_for_entity::<Deep>(context.entity)
                .is_some();
            log(&mut world, format!("Top hook mid={mid} deep={deep}"));
        })
    }
}

fn top_signal(on: On<Add<Top>>, mids: Query<&Mid>, deeps: Query<&Deep>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0.push(format!(
        "Top signal mid={} deep={}",
        mids.get_entity(entity).is_some(),
        deeps.get_entity(entity).is_some()
    ));
}

fn mid_signal(on: On<Add<Mid>>, deeps: Query<&Deep>, mut log: ResMut<Log>) {
    let entity = on.signal().entity;
    log.0.push(format!(
        "Mid signal deep={}",
        deeps.get_entity(entity).is_some()
    ));
}

fn world() -> World {
    let mut world = World::new();
    world.insert_resource(Log::default());
    world.add_listener(top_signal);
    world.add_listener(mid_signal);
    world
}

fn take(world: &mut World) -> Vec<String> {
    std::mem::take(&mut world.get_resource_mut::<Log>().unwrap().0)
}

const CHAIN: [&str; 5] = [
    "Deep hook",
    "Mid hook deep=true",
    "Mid signal deep=true",
    "Top hook mid=true deep=true",
    "Top signal mid=true deep=true",
];

struct HookCase {
    name: &'static str,
    act: fn(&mut World),
}

#[test]
fn requirements_fire_their_hooks_and_signals_before_their_requirers() {
    let cases = [
        HookCase {
            name: "spawn fires a two level chain deepest first",
            act: |world| {
                world.spawn(Top);
            },
        },
        HookCase {
            name: "insert fires a two level chain deepest first",
            act: |world| {
                let entity = world.spawn(());
                world.insert(Top, entity);
            },
        },
        HookCase {
            name: "a bundle member that is a requirement fires before its requirer",
            act: |world| {
                world.spawn((Top, Mid));
            },
        },
    ];
    for case in cases {
        let mut world = world();

        (case.act)(&mut world);

        assert_eq!(take(&mut world), CHAIN, "{}", case.name);
    }
}
