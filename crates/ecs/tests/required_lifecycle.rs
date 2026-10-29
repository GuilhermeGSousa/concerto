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

#[test]
fn spawn_fires_a_two_level_chain_deepest_first_with_requirements_present() {
    let mut world = world();

    world.spawn(Top);

    assert_eq!(take(&mut world), CHAIN);
}

#[test]
fn insert_fires_a_two_level_chain_deepest_first_with_requirements_present() {
    let mut world = world();
    let entity = world.spawn(());

    world.insert(Top, entity);

    assert_eq!(take(&mut world), CHAIN);
}

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

fn expected_for_p_and_q() -> Vec<String> {
    let mut members = [(TypeId::of::<P>(), "P"), (TypeId::of::<Q>(), "Q")];
    members.sort_by_key(|(id, _)| *id);
    members
        .iter()
        .flat_map(|(_, name)| [format!("Req{name}"), (*name).to_string()])
        .collect()
}

#[test]
fn bundle_members_with_requirements_fire_in_type_id_order_each_after_its_requirements() {
    let mut world = world();

    world.spawn((P, Q));

    assert_eq!(take(&mut world), expected_for_p_and_q());
}

#[test]
fn insert_orders_bundle_members_the_same_way() {
    let mut world = world();
    let entity = world.spawn(());

    world.insert((Q, P), entity);

    assert_eq!(take(&mut world), expected_for_p_and_q());
}

#[test]
fn a_requirement_fires_before_the_requirer_then_members_without_requirements_follow() {
    let mut world = world();

    world.spawn((Plain, P));

    let expected = ["ReqP", "P", "Plain"];
    assert_eq!(take(&mut world), expected);
}

#[test]
fn a_bundle_member_that_is_a_requirement_fires_before_its_requirer() {
    let mut world = world();

    world.spawn((Top, Mid));

    assert_eq!(
        take(&mut world),
        [
            "Deep hook",
            "Mid hook deep=true",
            "Mid signal deep=true",
            "Top hook mid=true deep=true",
            "Top signal mid=true deep=true",
        ]
    );
}

#[test]
fn required_components_without_callbacks_stay_silent() {
    #[derive(Default)]
    struct Silent;
    impl Component for Silent {}

    struct Needs;
    impl Component for Needs {
        fn register_required(required: &mut RequiredComponents) {
            required.register::<Silent>(Silent::default);
        }
    }

    let mut world = world();

    let entity = world.spawn(Needs);

    assert!(world.get_component_for_entity::<Silent>(entity).is_some());
    assert!(take(&mut world).is_empty());
}
