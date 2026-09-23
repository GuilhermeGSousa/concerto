//! Covers where an anchored panel lands: the side it prefers, the side it
//! settles for near a window edge, and the point a context menu opens at.
use std::collections::HashMap;

use ecs::entity::hierarchy::ChildOf;
use ecs::events::event_channel::EventChannel;
use ecs::{IntoSystem, System, World, entity::Entity};
use glam::Vec2;
use ui::anchor::{
    PanelRects, UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack,
    panels_to_close, place, track_panel_stack,
};
use ui::focus::{FocusedWidget, UIFocusLost};
use ui::node::{UIBox, UINode};

const WINDOW: Vec2 = Vec2::new(1000.0, 800.0);

fn anchor(min: Vec2, size: Vec2) -> UIBox {
    UIBox { min, size }
}

fn panel(side: UIAnchorSide, align: UIAnchorAlign) -> UIAnchoredPanel {
    UIAnchoredPanel {
        target: UIAnchorTarget::Point {
            position: Vec2::ZERO,
        },
        owner: None,
        side,
        align,
        gap: 4.0,
        open: true,
        ..Default::default()
    }
}

#[test]
fn a_panel_that_fits_sits_on_the_side_it_asked_for() {
    let trigger = anchor(Vec2::new(100.0, 100.0), Vec2::new(120.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(200.0, 160.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(placement.side, UIAnchorSide::Below);
    assert_eq!(
        placement.origin,
        Vec2::new(100.0, 134.0),
        "below-start puts the panel's left edge on the anchor's and clears the gap"
    );
}

#[test]
fn a_panel_with_no_room_below_flips_above() {
    // A trigger near the bottom edge: 760 + 30 + 4 + 160 is well past 800.
    let trigger = anchor(Vec2::new(100.0, 760.0), Vec2::new(120.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(200.0, 160.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(
        placement.side,
        UIAnchorSide::Above,
        "the caller needs to know which way it ended up facing"
    );
    assert_eq!(
        placement.origin.y, 596.0,
        "above means the panel's bottom edge clears the anchor's top by the gap"
    );
}

#[test]
fn a_panel_that_fits_on_neither_side_stays_on_the_one_it_asked_for() {
    // 400 tall in an 800 window with the anchor dead centre: neither side fits.
    let trigger = anchor(Vec2::new(100.0, 380.0), Vec2::new(120.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(200.0, 400.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(
        placement.side,
        UIAnchorSide::Below,
        "flipping only helps when the other side actually fits"
    );
    assert!(
        placement.origin.y + 400.0 <= WINDOW.y,
        "and the clamp still keeps it on screen"
    );
}

#[test]
fn a_panel_near_the_right_edge_shifts_back_inside() {
    let trigger = anchor(Vec2::new(950.0, 100.0), Vec2::new(40.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(200.0, 160.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(
        placement.origin.x, 800.0,
        "shifted left just enough to sit flush with the window edge"
    );
    assert_eq!(
        placement.side,
        UIAnchorSide::Below,
        "shifting along the cross axis is not a flip"
    );
}

#[test]
fn a_panel_larger_than_the_window_is_pinned_to_the_origin() {
    let trigger = anchor(Vec2::new(400.0, 400.0), Vec2::new(40.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(1200.0, 900.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(
        placement.origin,
        Vec2::ZERO,
        "nothing can make it fit, so show the top-left of it rather than the middle"
    );
}

#[test]
fn alignment_moves_the_panel_along_the_anchors_own_edge() {
    let trigger = anchor(Vec2::new(300.0, 100.0), Vec2::new(120.0, 30.0));
    let size = Vec2::new(60.0, 40.0);

    let start = place(
        trigger,
        size,
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );
    let centre = place(
        trigger,
        size,
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Center),
    );
    let end = place(
        trigger,
        size,
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::End),
    );

    assert_eq!(start.origin.x, 300.0);
    assert_eq!(
        centre.origin.x, 330.0,
        "centred on the anchor's 120px width"
    );
    assert_eq!(end.origin.x, 360.0, "right edges flush");
}

#[test]
fn a_side_anchor_aligns_on_the_vertical_axis() {
    let trigger = anchor(Vec2::new(300.0, 100.0), Vec2::new(120.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(60.0, 40.0),
        WINDOW,
        &panel(UIAnchorSide::Right, UIAnchorAlign::Start),
    );

    assert_eq!(
        placement.origin.x, 424.0,
        "to the right of the anchor, past the gap"
    );
    assert_eq!(placement.origin.y, 100.0, "start aligns the top edges");
}

#[test]
fn a_context_menu_opens_at_the_cursor() {
    // A Point target is a zero-size anchor, so below-start is exactly the
    // cursor plus the gap — the standard context-menu offset.
    let cursor = anchor(Vec2::new(520.0, 240.0), Vec2::ZERO);
    let placement = place(
        cursor,
        Vec2::new(180.0, 120.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::Start),
    );

    assert_eq!(placement.origin, Vec2::new(520.0, 244.0));
}

#[test]
fn a_left_anchored_panel_sits_past_the_anchors_left_edge() {
    // `Left` is the one side with no other coverage.
    let trigger = anchor(Vec2::new(500.0, 200.0), Vec2::new(120.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(80.0, 40.0),
        WINDOW,
        &panel(UIAnchorSide::Left, UIAnchorAlign::Start),
    );
    assert_eq!(placement.side, UIAnchorSide::Left);
    assert_eq!(
        placement.origin.x, 416.0,
        "clears the anchor's left edge by the gap"
    );
    assert_eq!(placement.origin.y, 200.0, "start aligns the top edges");
}

#[test]
fn an_end_aligned_panel_near_the_edge_is_still_shifted_inside() {
    // Alignment and shift are separate steps; nothing pinned them together.
    let trigger = anchor(Vec2::new(940.0, 100.0), Vec2::new(40.0, 30.0));
    let placement = place(
        trigger,
        Vec2::new(200.0, 160.0),
        WINDOW,
        &panel(UIAnchorSide::Below, UIAnchorAlign::End),
    );
    assert_eq!(
        placement.origin.x, 780.0,
        "end alignment would put it at 780; that already fits, so no shift"
    );
    assert!(
        placement.origin.x + 200.0 <= WINDOW.x,
        "and it stays on screen"
    );
}

/// Entities have no public raw constructor — generations are `NonZero` — so a
/// throwaway world mints the distinct ids these tests compare.
fn entities(count: usize) -> Vec<Entity> {
    let mut world = World::default();
    (0..count).map(|_| world.spawn(UINode::default())).collect()
}

/// A menu at 100,100 and a submenu at 300,120, each with a trigger.
fn two_deep(ids: &[Entity]) -> Vec<PanelRects> {
    vec![
        PanelRects {
            panel: ids[0],
            rect: anchor(Vec2::new(100.0, 100.0), Vec2::new(180.0, 200.0)),
            owner_rect: Some(anchor(Vec2::new(100.0, 60.0), Vec2::new(120.0, 30.0))),
        },
        PanelRects {
            panel: ids[1],
            rect: anchor(Vec2::new(300.0, 120.0), Vec2::new(180.0, 140.0)),
            owner_rect: Some(anchor(Vec2::new(100.0, 120.0), Vec2::new(180.0, 24.0))),
        },
    ]
}

#[test]
fn a_press_on_empty_space_closes_every_panel() {
    let ids = entities(2);
    let stack = two_deep(&ids);
    let closing = panels_to_close(&stack, Vec2::new(700.0, 600.0));
    assert_eq!(closing.len(), 2);
    assert_eq!(closing[0].panel, ids[0]);
}

#[test]
fn a_press_in_the_deepest_panel_closes_nothing() {
    let ids = entities(2);
    let stack = two_deep(&ids);
    let closing = panels_to_close(&stack, Vec2::new(350.0, 160.0));
    assert!(closing.is_empty());
}

#[test]
fn a_press_back_in_the_parent_closes_only_the_submenu() {
    let ids = entities(2);
    let stack = two_deep(&ids);
    // 150,260 is inside the parent panel but below its submenu row.
    let closing = panels_to_close(&stack, Vec2::new(150.0, 260.0));
    assert_eq!(closing.len(), 1);
    assert_eq!(
        closing[0].panel, ids[1],
        "stepping back up a menu chain closes what you stepped out of"
    );
}

#[test]
fn a_press_on_a_panels_own_trigger_does_not_close_that_panel() {
    // Otherwise the press would dismiss the menu and the caller's toggle
    // would immediately reopen it, and the trigger could never close it.
    let ids = entities(2);
    let stack = two_deep(&ids);
    let closing = panels_to_close(&stack, Vec2::new(150.0, 70.0));
    assert!(
        !closing.iter().any(|panel| panel.panel == ids[0]),
        "a press on the owner counts as a press inside the panel"
    );
    assert_eq!(
        closing.len(),
        1,
        "but a submenu opened from it is still stepped out of"
    );
}

#[test]
fn a_press_on_the_submenus_row_leaves_both_open() {
    let ids = entities(2);
    let stack = two_deep(&ids);
    let closing = panels_to_close(&stack, Vec2::new(150.0, 130.0));
    assert!(closing.is_empty(), "that row is the submenu's owner");
}

#[test]
fn a_standalone_panel_with_no_owner_closes_only_on_a_press_outside_it() {
    // A panel opened without a trigger — `UIAnchoredPanel::default()` has
    // `owner: None` — is a legitimate standalone overlay, not a bug. A
    // missing owner must be "never exempt", not "always exempt": inside its
    // own rect still keeps it open, but outside it, it has nothing left to
    // fall back on and must close like any other panel.
    let ids = entities(1);
    let stack = vec![PanelRects {
        panel: ids[0],
        rect: anchor(Vec2::new(400.0, 400.0), Vec2::new(180.0, 200.0)),
        owner_rect: None,
    }];

    let inside = panels_to_close(&stack, Vec2::new(450.0, 450.0));
    assert!(
        inside.is_empty(),
        "a press inside the panel's own rect closes nothing, owner or not"
    );

    let outside = panels_to_close(&stack, Vec2::new(100.0, 100.0));
    assert_eq!(
        outside.len(),
        1,
        "with no owner rect to fall back on, a press outside the panel closes it"
    );
}

/// A world with everything `track_panel_stack` reads: the stack it rebuilds,
/// the focus it clears out of a panel it hides, and the channel that clearing
/// announces itself on.
fn stack_world() -> World {
    let mut world = World::default();
    world.insert_resource(UIPanelStack::default());
    world.insert_resource(FocusedWidget::default());
    world.insert_resource(EventChannel::<UIFocusLost>::default());
    world
}

fn run(world: &mut World) {
    let mut system = track_panel_stack.into_system();
    system.initialize(world);
    system.run_and_apply(world);
}

/// A visible node standing in for a trigger or a menu row.
fn node(world: &mut World) -> Entity {
    world.spawn(UINode::default())
}

fn open_panel(world: &mut World, owner: Entity, target: UIAnchorTarget) -> Entity {
    world.spawn((
        UINode {
            visible: false,
            ..Default::default()
        },
        UIAnchoredPanel {
            target,
            owner: Some(owner),
            open: true,
            ..Default::default()
        },
    ))
}

fn layer(world: &World, entity: Entity) -> i32 {
    world
        .get_component_for_entity::<UINode>(entity)
        .expect("panel must still have its node")
        .z_index
}

fn visible(world: &World, entity: Entity) -> bool {
    world
        .get_component_for_entity::<UINode>(entity)
        .expect("panel must still have its node")
        .visible
}

#[test]
fn an_open_panel_becomes_visible_on_the_panel_layer() {
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );

    run(&mut world);

    assert!(visible(&world, menu), "an open panel must be shown");
    assert_eq!(
        layer(&world, menu),
        200,
        "and must paint on the panel layer"
    );
    assert_eq!(
        world.get_resource::<UIPanelStack>().unwrap().open(),
        &[menu]
    );
}

#[test]
fn a_submenu_nests_under_the_panel_its_owner_lives_in() {
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );

    // The row that opens the submenu is a child of the menu.
    let row = node(&mut world);
    world.insert(ChildOf::new(menu), row);
    let submenu = open_panel(&mut world, row, UIAnchorTarget::Node { entity: row });

    run(&mut world);

    let stack = world.get_resource::<UIPanelStack>().unwrap();
    assert_eq!(stack.open(), &[menu, submenu], "outermost first");
    assert_eq!(
        layer(&world, submenu),
        201,
        "a submenu paints above its parent"
    );
}

#[test]
fn a_context_menu_at_a_point_nests_by_its_owner_too() {
    // The regression the owner field exists to prevent: a point-anchored menu
    // opened from a row inside another panel must not read as top-level.
    //
    // A depth-blind (target-keyed) implementation would still pass a naive
    // version of this test: with everything at depth 0, the (depth, spawn
    // order) tie-break alone produces [menu, context] and layer 201, since
    // layer is UI_PANEL_LAYER + stack index, not depth. A second top-level
    // panel spawned after `context` makes depth observable: keyed off owner,
    // `context` sits deeper than `top2` and must sort after it regardless of
    // spawn order, landing on layer 202, not 201.
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );
    let row = node(&mut world);
    world.insert(ChildOf::new(menu), row);
    let context = open_panel(
        &mut world,
        row,
        UIAnchorTarget::Point {
            position: Vec2::new(500.0, 300.0),
        },
    );
    let trigger2 = node(&mut world);
    let top2 = open_panel(
        &mut world,
        trigger2,
        UIAnchorTarget::Node { entity: trigger2 },
    );

    run(&mut world);

    let stack = world.get_resource::<UIPanelStack>().unwrap();
    assert_eq!(
        stack.open(),
        &[menu, top2, context],
        "context nests one level deeper than the other top-level panel, so it \
         sorts after it even though it was spawned first"
    );
    assert_eq!(
        layer(&world, context),
        202,
        "layer is stack index; a target-keyed implementation would read this \
         panel as top-level and put it at 201"
    );
}

#[test]
fn closing_a_parent_closes_the_submenu_under_it() {
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );
    let row = node(&mut world);
    world.insert(ChildOf::new(menu), row);
    let submenu = open_panel(&mut world, row, UIAnchorTarget::Node { entity: row });
    run(&mut world);

    world
        .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
        .unwrap()
        .open = false;
    run(&mut world);

    assert!(!visible(&world, submenu), "a chain cannot outlive its root");
    assert!(
        world
            .get_resource::<UIPanelStack>()
            .unwrap()
            .open()
            .is_empty()
    );
}

#[test]
fn a_panel_whose_owner_is_hidden_closes_itself() {
    // A context menu on a row that scrolled out of existence should go with it.
    let mut world = stack_world();
    let row = node(&mut world);
    let menu = open_panel(
        &mut world,
        row,
        UIAnchorTarget::Point {
            position: Vec2::ZERO,
        },
    );
    run(&mut world);
    assert!(visible(&world, menu));

    world
        .get_component_for_entity_mut::<UINode>(row)
        .unwrap()
        .visible = false;
    run(&mut world);

    assert!(!visible(&world, menu), "the thing it was about is gone");

    // The closure must be latched onto UIAnchoredPanel::open, not just
    // UINode::visible: otherwise the row reappearing makes the menu pop back
    // open on its own, at whatever stale position it last had, without the
    // user ever asking for it again.
    world
        .get_component_for_entity_mut::<UINode>(row)
        .unwrap()
        .visible = true;
    run(&mut world);

    assert!(
        !visible(&world, menu),
        "the owner coming back must not resurrect a menu nobody reopened"
    );
    assert!(
        !world
            .get_component_for_entity::<UIAnchoredPanel>(menu)
            .unwrap()
            .open,
        "open must be latched to false so a toggle callback sees the real state"
    );
}

#[test]
fn a_panel_spawned_before_its_first_layout_stays_open() {
    // Absence of UILayout is the normal state of a fresh entity, not
    // staleness — treating it as staleness would close every new panel.
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );

    run(&mut world);

    assert!(visible(&world, menu), "no node here has ever been laid out");
}

#[test]
fn a_point_target_anchors_to_a_zero_size_box_at_the_point() {
    let rects = HashMap::new();
    let target = UIAnchorTarget::Point {
        position: Vec2::new(40.0, 90.0),
    };

    assert_eq!(
        target.anchor_box(&rects),
        Some(anchor(Vec2::new(40.0, 90.0), Vec2::ZERO)),
        "a cursor is a box with no extent, so placement reads its edges as the point"
    );
}

#[test]
fn a_node_target_anchors_to_the_rect_it_was_laid_out_at() {
    let mut world = World::default();
    let trigger = node(&mut world);
    let rect = anchor(Vec2::new(100.0, 100.0), Vec2::new(120.0, 30.0));
    let mut rects = HashMap::new();
    rects.insert(trigger, rect);

    assert_eq!(
        UIAnchorTarget::Node { entity: trigger }.anchor_box(&rects),
        Some(rect)
    );
}

#[test]
fn a_node_target_with_no_rect_has_nothing_to_anchor_to() {
    // Hidden, or hidden by an ancestor, or gone: the caller must leave the
    // panel where it is rather than place it against the window origin.
    let mut world = World::default();
    let trigger = node(&mut world);

    assert_eq!(
        UIAnchorTarget::Node { entity: trigger }.anchor_box(&HashMap::new()),
        None
    );
}

#[test]
fn a_dropdown_that_named_no_owner_is_still_exempt_from_its_own_trigger() {
    // `UIAnchoredPanel::default()` has `owner: None`, so the obvious way to
    // write a dropdown — set `target` to the trigger and leave the rest —
    // used to get no press exemption at all: the second click on the trigger
    // read as an outside press, dismissal closed the panel and the caller's
    // toggle reopened it, so the menu could never be closed by its own
    // button. A `Node` target stands in for the missing owner.
    let mut world = World::default();
    let trigger = node(&mut world);
    let spec = UIAnchoredPanel {
        target: UIAnchorTarget::Node { entity: trigger },
        open: true,
        ..Default::default()
    };
    assert_eq!(spec.press_exempt_entity(), Some(trigger));

    let trigger_rect = anchor(Vec2::new(100.0, 60.0), Vec2::new(120.0, 30.0));
    let stack = vec![PanelRects {
        panel: node(&mut world),
        rect: anchor(Vec2::new(100.0, 100.0), Vec2::new(180.0, 200.0)),
        owner_rect: spec.press_exempt_entity().map(|_| trigger_rect),
    }];
    assert!(
        panels_to_close(&stack, Vec2::new(150.0, 70.0)).is_empty(),
        "a press on the trigger must leave the panel for the caller's toggle to close"
    );
}

#[test]
fn an_owned_panel_keeps_exempting_its_owner_not_its_target() {
    // The fallback must not disturb a caller that does set `owner`: a context
    // menu is placed at a point but belongs to the row under it.
    let mut world = World::default();
    let row = node(&mut world);
    let spec = UIAnchoredPanel {
        target: UIAnchorTarget::Point {
            position: Vec2::new(500.0, 300.0),
        },
        owner: Some(row),
        open: true,
        ..Default::default()
    };
    assert_eq!(spec.press_exempt_entity(), Some(row));
}

#[test]
fn a_standalone_overlay_exempts_nothing() {
    // No owner and no node to fall back on: a press outside it closes it,
    // which is what makes a bare overlay dismissable at all.
    assert_eq!(UIAnchoredPanel::default().press_exempt_entity(), None);
}

#[test]
fn a_panel_whose_owner_is_hidden_by_an_ancestor_closes_itself() {
    // The trigger's own `visible` is still true, but the layout pass records
    // no anchor rect for anything under a hidden ancestor, so the panel would
    // otherwise keep its last `UILayout` and hang over the screen at a stale
    // position with the section it belongs to collapsed out of sight.
    let mut world = stack_world();
    let section = node(&mut world);
    let trigger = node(&mut world);
    world.insert(ChildOf::new(section), trigger);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );
    run(&mut world);
    assert!(visible(&world, menu));

    world
        .get_component_for_entity_mut::<UINode>(section)
        .unwrap()
        .visible = false;
    run(&mut world);

    assert!(
        !visible(&world, menu),
        "a collapsed section takes the menus opened from inside it with it"
    );
}

#[test]
fn a_panel_whose_owner_was_despawned_closes_itself() {
    let mut world = stack_world();
    let row = node(&mut world);
    let menu = open_panel(
        &mut world,
        row,
        UIAnchorTarget::Point {
            position: Vec2::ZERO,
        },
    );
    run(&mut world);
    assert!(visible(&world, menu));

    world.despawn(row);
    run(&mut world);

    assert!(
        !visible(&world, menu),
        "a context menu cannot outlive the row it acts on"
    );
}

#[test]
fn a_panel_whose_node_target_was_despawned_closes_itself() {
    // Separate from the owner case: a panel is closed for having nothing to
    // anchor to, not only for having nothing to be about.
    let mut world = stack_world();
    let owner = node(&mut world);
    let trigger = node(&mut world);
    let menu = open_panel(&mut world, owner, UIAnchorTarget::Node { entity: trigger });
    run(&mut world);
    assert!(visible(&world, menu));

    world.despawn(trigger);
    run(&mut world);

    assert!(
        !visible(&world, menu),
        "nothing left to anchor to, so it is not left floating at its last position"
    );
}

#[test]
fn hiding_a_panel_takes_the_focus_inside_it_away() {
    // `update_focus` only reassigns focus on a left press, so a field left
    // focused inside a dismissed panel keeps it until the next click: text
    // capture stays on, swallowing every bare-letter shortcut, and Escape
    // keeps being deferred to a field nobody can see.
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );
    let field = node(&mut world);
    world.insert(ChildOf::new(menu), field);
    let mut focused = FocusedWidget::default();
    *focused = Some(field);
    world.insert_resource(focused);

    run(&mut world);
    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        Some(field),
        "the panel is open, so the field keeps focus"
    );

    world
        .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
        .unwrap()
        .open = false;
    run(&mut world);

    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        None,
        "the field went off screen with the panel and cannot keep the keyboard"
    );
}

#[test]
fn focus_outside_every_panel_is_left_alone() {
    // The inspector field a menu was opened from must not lose focus when
    // that menu closes.
    let mut world = stack_world();
    let trigger = node(&mut world);
    let menu = open_panel(
        &mut world,
        trigger,
        UIAnchorTarget::Node { entity: trigger },
    );
    let mut focused = FocusedWidget::default();
    *focused = Some(trigger);
    world.insert_resource(focused);
    run(&mut world);

    world
        .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
        .unwrap()
        .open = false;
    run(&mut world);

    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        Some(trigger),
        "the trigger is not inside the panel, so closing it says nothing about focus"
    );
}
