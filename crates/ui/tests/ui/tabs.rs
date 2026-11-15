//! Covers tab-strip body switching: selecting a tab shows exactly one body.
use concerto_ecs::{
    IntoSystem, ResMut, Resource, System, World,
    signal::{On, listener::IntoListener},
};
use concerto_ui::interaction::UIClick;
use concerto_ui::node::UINode;
use concerto_ui::widgets::{
    UICollapsibleChanged, UICollapsibleSection, UITabBody, UITabChanged, UITabStrip, select_tab,
    sync_tab_bodies, toggle_collapsible,
};
use concerto_window::input::MouseButton;
use glam::Vec2;

#[derive(Resource, Default)]
struct Heard(Vec<String>);

fn click(world: &mut World, entity: concerto_ecs::Entity) {
    world.trigger_on(
        entity,
        UIClick {
            position: Vec2::ZERO,
            button: MouseButton::Left,
        },
    );
}

fn run(world: &mut World) {
    let mut system = sync_tab_bodies.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

fn visible(world: &World, entity: concerto_ecs::Entity) -> bool {
    world
        .get_component_for_entity::<UINode>(entity)
        .expect("body must still have its node")
        .visible
}

#[test]
fn clicking_a_tab_selects_it_tells_the_strip_and_shows_only_its_body() {
    let mut world = World::default();
    world.insert_resource(Heard::default());
    let strip = world.spawn((
        UITabStrip::default(),
        (|on: On<UITabChanged>, mut heard: ResMut<Heard>| {
            heard.0.push(on.signal().selected.to_string());
        })
        .into_listener(),
    ));
    let bodies: Vec<_> = (0..3)
        .map(|index| world.spawn((UINode::default(), UITabBody { strip, index })))
        .collect();
    let tabs: Vec<_> = (0..3)
        .map(|index| world.spawn(select_tab(strip, index)))
        .collect();
    run(&mut world);
    assert_eq!(
        bodies
            .iter()
            .map(|body| visible(&world, *body))
            .collect::<Vec<_>>(),
        [true, false, false]
    );

    click(&mut world, tabs[2]);
    run(&mut world);

    assert_eq!(
        bodies
            .iter()
            .map(|body| visible(&world, *body))
            .collect::<Vec<_>>(),
        [false, false, true]
    );
    assert_eq!(world.get_resource::<Heard>().unwrap().0, ["2"]);
}

#[test]
fn clicking_a_collapsible_section_hides_and_shows_its_content() {
    let mut world = World::default();
    world.insert_resource(Heard::default());
    let content = world.spawn(UINode::default());
    let section = world.spawn((
        UICollapsibleSection {
            expanded: true,
            content,
        },
        toggle_collapsible.into_listener(),
        (|on: On<UICollapsibleChanged>, mut heard: ResMut<Heard>| {
            heard.0.push(on.signal().expanded.to_string());
        })
        .into_listener(),
    ));

    click(&mut world, section);
    assert!(!visible(&world, content));
    click(&mut world, section);
    assert!(visible(&world, content));

    assert_eq!(world.get_resource::<Heard>().unwrap().0, ["false", "true"]);
}
