//! Covers where an anchored panel lands.
use concerto_ecs::entity::hierarchy::ChildOf;
use concerto_ecs::events::event_channel::EventChannel;
use concerto_ecs::{IntoSystem, System, World, entity::Entity};
use concerto_ui::anchor::{
    PanelRects, UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack,
    panels_to_close, place, track_panel_stack,
};
use concerto_ui::focus::{FocusedWidget, UIFocusGained, UIFocusLost};
use concerto_ui::node::{UIBox, UINode};
use glam::Vec2;

const WINDOW: Vec2 = Vec2::new(1000.0, 800.0);

fn anchor(min: Vec2, size: Vec2) -> UIBox {
    UIBox { min, size }
}

fn panel(side: UIAnchorSide, align: UIAnchorAlign) -> UIAnchoredPanel {
    UIAnchoredPanel::new(UIAnchorTarget::Point {
        position: Vec2::ZERO,
    })
    .with_side(side)
    .with_align(align)
    .with_gap(4.0)
    .with_open(true)
}

#[test]
fn a_panel_sits_beside_its_anchor_on_the_side_that_fits_and_inside_the_window() {
    struct Case {
        name: &'static str,
        anchor: UIBox,
        size: Vec2,
        side: UIAnchorSide,
        align: UIAnchorAlign,
        placed_side: UIAnchorSide,
        origin: Vec2,
    }
    let trigger = anchor(Vec2::new(300.0, 100.0), Vec2::new(120.0, 30.0));
    let small = Vec2::new(60.0, 40.0);
    let menu = Vec2::new(200.0, 160.0);
    let below = UIAnchorSide::Below;
    for case in [
        Case {
            name: "a panel that fits sits where it asked, clear of the gap",
            anchor: anchor(Vec2::new(100.0, 100.0), Vec2::new(120.0, 30.0)),
            size: menu,
            side: below,
            align: UIAnchorAlign::Start,
            placed_side: below,
            origin: Vec2::new(100.0, 134.0),
        },
        Case {
            name: "no room below flips above",
            anchor: anchor(Vec2::new(100.0, 760.0), Vec2::new(120.0, 30.0)),
            size: menu,
            side: below,
            align: UIAnchorAlign::Start,
            placed_side: UIAnchorSide::Above,
            origin: Vec2::new(100.0, 596.0),
        },
        Case {
            name: "fitting on neither side keeps the side asked for and clamps",
            anchor: anchor(Vec2::new(100.0, 380.0), Vec2::new(120.0, 30.0)),
            size: Vec2::new(200.0, 400.0),
            side: below,
            align: UIAnchorAlign::Start,
            placed_side: below,
            origin: Vec2::new(100.0, 400.0),
        },
        Case {
            name: "near the right edge shifts back inside without flipping",
            anchor: anchor(Vec2::new(950.0, 100.0), Vec2::new(40.0, 30.0)),
            size: menu,
            side: below,
            align: UIAnchorAlign::Start,
            placed_side: below,
            origin: Vec2::new(800.0, 134.0),
        },
        Case {
            name: "centre alignment on the anchor's width",
            anchor: trigger,
            size: small,
            side: below,
            align: UIAnchorAlign::Center,
            placed_side: below,
            origin: Vec2::new(330.0, 134.0),
        },
        Case {
            name: "end alignment puts the right edges flush",
            anchor: trigger,
            size: small,
            side: below,
            align: UIAnchorAlign::End,
            placed_side: below,
            origin: Vec2::new(360.0, 134.0),
        },
        Case {
            name: "a right anchor aligns on the vertical axis",
            anchor: trigger,
            size: small,
            side: UIAnchorSide::Right,
            align: UIAnchorAlign::Start,
            placed_side: UIAnchorSide::Right,
            origin: Vec2::new(424.0, 100.0),
        },
        Case {
            name: "a left anchor clears the anchor's left edge",
            anchor: anchor(Vec2::new(500.0, 200.0), Vec2::new(120.0, 30.0)),
            size: Vec2::new(80.0, 40.0),
            side: UIAnchorSide::Left,
            align: UIAnchorAlign::Start,
            placed_side: UIAnchorSide::Left,
            origin: Vec2::new(416.0, 200.0),
        },
    ] {
        let placement = place(
            case.anchor,
            case.size,
            WINDOW,
            &panel(case.side, case.align),
        );
        assert_eq!(placement.side, case.placed_side, "{}", case.name);
        assert_eq!(placement.origin, case.origin, "{}", case.name);
    }
}

fn entities(count: usize) -> Vec<Entity> {
    let mut world = World::default();
    (0..count).map(|_| world.spawn(UINode::default())).collect()
}

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
fn a_press_closes_the_panels_it_stepped_out_of() {
    struct Case {
        name: &'static str,
        standalone: bool,
        press: Vec2,
        closes: &'static [usize],
    }
    let case = |name, standalone, press: [f32; 2], closes| Case {
        name,
        standalone,
        press: Vec2::from(press),
        closes,
    };
    let ids = entities(2);
    for case in [
        case(
            "empty space closes every panel",
            false,
            [700.0, 600.0],
            &[0, 1],
        ),
        case(
            "the deepest panel closes nothing",
            false,
            [350.0, 160.0],
            &[],
        ),
        case(
            "back in the parent closes the submenu",
            false,
            [150.0, 260.0],
            &[1],
        ),
        case(
            "a panel's own trigger closes only its submenu",
            false,
            [150.0, 70.0],
            &[1],
        ),
        case("outside a panel with no owner", true, [100.0, 100.0], &[0]),
    ] {
        let stack = if case.standalone {
            vec![PanelRects {
                panel: ids[0],
                rect: anchor(Vec2::new(400.0, 400.0), Vec2::new(180.0, 200.0)),
                owner_rect: None,
            }]
        } else {
            two_deep(&ids)
        };
        let closing = panels_to_close(&stack, case.press)
            .iter()
            .map(|panel| panel.panel)
            .collect::<Vec<_>>();
        let expected = case
            .closes
            .iter()
            .map(|&index| ids[index])
            .collect::<Vec<_>>();
        assert_eq!(closing, expected, "{}", case.name);
    }
}

fn stack_world() -> World {
    let mut world = World::default();
    world.insert_resource(UIPanelStack::default());
    world.insert_resource(FocusedWidget::default());
    world.insert_resource(EventChannel::<UIFocusLost>::default());
    world.insert_resource(EventChannel::<UIFocusGained>::default());
    world
}

fn run(world: &mut World) {
    let mut system = track_panel_stack.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

fn node(world: &mut World) -> Entity {
    world.spawn(UINode::default())
}

fn open_panel(world: &mut World, owner: Entity, target: UIAnchorTarget) -> Entity {
    world.spawn((
        UINode::default().with_visible(false),
        UIAnchoredPanel::new(target)
            .with_owner(owner)
            .with_open(true),
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
fn an_open_panel_shows_on_the_panel_layer_and_a_submenu_nests_above_it() {
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

    assert!(visible(&world, menu), "an open panel must be shown");
    assert_eq!(layer(&world, menu), 200, "on the panel layer");
    let stack = world.get_resource::<UIPanelStack>().unwrap();
    assert_eq!(stack.open(), &[menu, submenu], "outermost first");
    assert_eq!(
        layer(&world, submenu),
        201,
        "a submenu paints above its parent"
    );
}

#[test]
fn a_panel_closes_itself_with_whatever_it_belongs_to() {
    struct Chain {
        section: Entity,
        menu: Entity,
    }
    struct Case {
        name: &'static str,
        close: fn(&mut World, &Chain),
    }
    for case in [
        Case {
            name: "its parent panel closed",
            close: |world, chain| {
                world
                    .get_component_for_entity_mut::<UIAnchoredPanel>(chain.menu)
                    .unwrap()
                    .open = false;
            },
        },
        Case {
            name: "its owner hidden by an ancestor",
            close: |world, chain| {
                world
                    .get_component_for_entity_mut::<UINode>(chain.section)
                    .unwrap()
                    .visible = false;
            },
        },
    ] {
        let mut world = stack_world();
        let section = node(&mut world);
        let owner = node(&mut world);
        world.insert(ChildOf::new(section), owner);
        let target = node(&mut world);
        let menu = open_panel(&mut world, owner, UIAnchorTarget::Node { entity: target });
        let row = node(&mut world);
        world.insert(ChildOf::new(menu), row);
        let submenu = open_panel(&mut world, row, UIAnchorTarget::Node { entity: row });
        run(&mut world);
        assert!(
            visible(&world, menu) && visible(&world, submenu),
            "{}",
            case.name
        );

        (case.close)(&mut world, &Chain { section, menu });
        run(&mut world);

        assert!(!visible(&world, menu), "{}: the menu", case.name);
        assert!(
            !visible(&world, submenu),
            "{}: a chain cannot outlive its root",
            case.name
        );
        assert!(
            world
                .get_resource::<UIPanelStack>()
                .unwrap()
                .open()
                .is_empty(),
            "{}",
            case.name
        );
    }
}

#[test]
fn hiding_a_panel_takes_the_focus_inside_it_away() {
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
fn opening_a_panel_focuses_its_focus_on_open_widget_once() {
    let mut world = stack_world();
    let trigger = node(&mut world);
    let field = node(&mut world);
    let menu = world.spawn((
        UINode::default().with_visible(false),
        UIAnchoredPanel::new(UIAnchorTarget::Node { entity: trigger })
            .with_owner(trigger)
            .with_focus_on_open(field),
    ));
    world.insert(ChildOf::new(menu), field);
    run(&mut world);
    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        None,
        "a closed panel does not take focus"
    );

    world
        .get_component_for_entity_mut::<UIAnchoredPanel>(menu)
        .unwrap()
        .open = true;
    run(&mut world);
    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        Some(field),
        "opening the panel hands the keyboard to its field"
    );

    let mut focused = FocusedWidget::default();
    *focused = Some(trigger);
    world.insert_resource(focused);
    run(&mut world);
    assert_eq!(
        **world.get_resource::<FocusedWidget>().unwrap(),
        Some(trigger),
        "focus moves only on the frame the panel opens, not every frame it stays open"
    );
}
