//! Covers tab-strip body switching: selecting a tab shows exactly one body.
//!
//! `UITabStrip` already tracked which tab was selected, but nothing acted on
//! it, so a strip with several panels rendered all of them on top of each
//! other. This is what the editor's dock stands on.
use ecs::{IntoSystem, System, World};
use ui::node::UINode;
use ui::widgets::{UITabBody, UITabStrip, sync_tab_bodies};

/// Spawns a strip with `count` bodies and returns the strip and its bodies.
fn strip_with_bodies(world: &mut World, count: usize) -> (ecs::Entity, Vec<ecs::Entity>) {
    let strip = world.spawn(UITabStrip::default());
    let bodies = (0..count)
        .map(|index| world.spawn((UINode::default(), UITabBody { strip, index })))
        .collect();
    (strip, bodies)
}

fn run(world: &mut World) {
    let mut system = sync_tab_bodies.into_system();
    system.initialize(world);
    system.run_and_apply(world);
}

fn visible(world: &World, entity: ecs::Entity) -> bool {
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

    // A panel was removed while its tab was selected.
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
