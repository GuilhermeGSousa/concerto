//! Covers tab-strip body switching: selecting a tab shows exactly one body.
use concerto_ecs::{
    IntoSystem, ResMut, Resource, System, World,
    signal::{On, listener::IntoListener},
};
use concerto_ui::interaction::UIClick;
use concerto_ui::node::UINode;
use concerto_ui::widgets::{
    UICollapsibleChanged, UICollapsibleSection, UITab, UITabBody, UITabChanged, UITabStrip,
    select_tab, sync_tab_bodies, toggle_collapsible,
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

fn strip_with_bodies(
    world: &mut World,
    count: usize,
) -> (concerto_ecs::Entity, Vec<concerto_ecs::Entity>) {
    let strip = world.spawn(UITabStrip::default());
    let bodies = (0..count)
        .map(|index| world.spawn((UINode::default(), UITabBody { strip, index })))
        .collect();
    (strip, bodies)
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
fn only_the_selected_body_is_visible() {
    let mut world = World::default();
    let (_, bodies) = strip_with_bodies(&mut world, 3);

    run(&mut world);

    assert!(visible(&world, bodies[0]), "tab 0 is selected by default");
    assert!(
        !visible(&world, bodies[1]),
        "an unselected body must be hidden"
    );
    assert!(
        !visible(&world, bodies[2]),
        "an unselected body must be hidden"
    );
}

#[test]
fn selecting_a_tab_swaps_which_body_shows() {
    let mut world = World::default();
    let (strip, bodies) = strip_with_bodies(&mut world, 3);

    world
        .get_component_for_entity_mut::<UITabStrip>(strip)
        .unwrap()
        .selected = 2;
    run(&mut world);

    assert!(!visible(&world, bodies[0]));
    assert!(!visible(&world, bodies[1]));
    assert!(
        visible(&world, bodies[2]),
        "the newly selected body must show"
    );
}

#[test]
fn a_selection_past_the_end_shows_nothing_rather_than_everything() {
    let mut world = World::default();
    let (strip, bodies) = strip_with_bodies(&mut world, 2);

    world
        .get_component_for_entity_mut::<UITabStrip>(strip)
        .unwrap()
        .selected = 7;
    run(&mut world);

    assert!(
        bodies.iter().all(|body| !visible(&world, *body)),
        "a stale selection must not fall back to showing every body at once"
    );
}

#[test]
fn strips_do_not_interfere_with_each_other() {
    let mut world = World::default();
    let (left, left_bodies) = strip_with_bodies(&mut world, 2);
    let (_, right_bodies) = strip_with_bodies(&mut world, 2);

    world
        .get_component_for_entity_mut::<UITabStrip>(left)
        .unwrap()
        .selected = 1;
    run(&mut world);

    assert!(!visible(&world, left_bodies[0]));
    assert!(visible(&world, left_bodies[1]));
    assert!(
        visible(&world, right_bodies[0]),
        "the other strip keeps its own selection"
    );
    assert!(!visible(&world, right_bodies[1]));
}

#[test]
fn clicking_a_tab_selects_it_in_its_strip_and_tells_the_strip() {
    let mut world = World::default();
    world.insert_resource(Heard::default());
    let strip = world.spawn((
        UITabStrip::default(),
        (|on: On<UITabChanged>, mut heard: ResMut<Heard>| {
            heard.0.push(on.signal().selected.to_string());
        })
        .into_listener(),
    ));
    let tabs: Vec<_> = (0..3)
        .map(|index| world.spawn((UITab { strip, index }, select_tab.into_listener())))
        .collect();

    click(&mut world, tabs[2]);
    click(&mut world, tabs[1]);

    assert_eq!(
        world
            .get_component_for_entity::<UITabStrip>(strip)
            .unwrap()
            .selected,
        1
    );
    assert_eq!(world.get_resource::<Heard>().unwrap().0, ["2", "1"]);
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
