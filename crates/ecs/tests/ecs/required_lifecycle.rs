use concerto_ecs::{
    Query, ResMut, Resource, World,
    component::{Component, ComponentLifecycleCallback, RequiredComponents},
    signal::{On, lifecycle::Add},
    world::RestrictedWorld,
};
use std::any::TypeId;

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
    world.register_component::<P>();
    world.register_component::<Q>();
    world.register_component::<ReqP>();
    world.register_component::<ReqQ>();
    world.register_component::<Plain>();
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

#[derive(Default)]
struct ReqP;

#[derive(Default)]
struct ReqQ;

#[derive(Default)]
struct P;

#[derive(Default)]
struct Q;

#[derive(Default)]
struct Plain;

macro_rules! hook {
    ($name:literal) => {
        fn on_add() -> Option<ComponentLifecycleCallback> {
            Some(|mut world, _| log(&mut world, $name.into()))
        }
    };
}

impl Component for ReqP {
    hook!("ReqP");
}

impl Component for ReqQ {
    hook!("ReqQ");
}

impl Component for Plain {
    hook!("Plain");
}

impl Component for P {
    hook!("P");

    fn register_required(required: &mut RequiredComponents) {
        required.register::<ReqP>(ReqP::default);
    }
}

impl Component for Q {
    hook!("Q");

    fn register_required(required: &mut RequiredComponents) {
        required.register::<ReqQ>(ReqQ::default);
    }
}

#[derive(Default)]
struct Silent;

impl Component for Silent {}

struct Needs;

impl Component for Needs {
    fn register_required(required: &mut RequiredComponents) {
        required.register::<Silent>(Silent::default);
    }
}

fn lines(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| line.to_string()).collect()
}

fn p_and_q() -> Vec<String> {
    let order = if TypeId::of::<P>() < TypeId::of::<Q>() {
        ["P", "Q"]
    } else {
        ["Q", "P"]
    };
    order
        .iter()
        .flat_map(|name| [format!("Req{name}"), name.to_string()])
        .collect()
}

struct HookCase {
    name: &'static str,
    act: fn(&mut World),
    log: fn() -> Vec<String>,
}

#[test]
fn requirements_fire_their_hooks_and_signals_before_their_requirers() {
    let cases = [
        HookCase {
            name: "spawn fires a two level chain deepest first",
            act: |world| {
                world.spawn(Top);
            },
            log: || lines(&CHAIN),
        },
        HookCase {
            name: "insert fires a two level chain deepest first",
            act: |world| {
                let entity = world.spawn(());
                world.insert(Top, entity);
            },
            log: || lines(&CHAIN),
        },
        HookCase {
            name: "bundle members fire in type id order, each after its requirements",
            act: |world| {
                world.spawn((P, Q));
            },
            log: p_and_q,
        },
        HookCase {
            name: "insert orders bundle members the same way",
            act: |world| {
                let entity = world.spawn(());
                world.insert((Q, P), entity);
            },
            log: p_and_q,
        },
        HookCase {
            name: "members without requirements follow",
            act: |world| {
                world.spawn((Plain, P));
            },
            log: || lines(&["ReqP", "P", "Plain"]),
        },
        HookCase {
            name: "a bundle member that is a requirement fires before its requirer",
            act: |world| {
                world.spawn((Top, Mid));
            },
            log: || lines(&CHAIN),
        },
        HookCase {
            name: "requirements without callbacks stay silent",
            act: |world| {
                world.spawn(Needs);
            },
            log: Vec::new,
        },
    ];
    for case in cases {
        let mut world = world();

        (case.act)(&mut world);

        assert_eq!(take(&mut world), (case.log)(), "{}", case.name);
    }
}
