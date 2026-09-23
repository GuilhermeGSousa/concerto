//! The UI showcase: every `ui` primitive on one screen, including the
//! anchored-panel feature, which has no automated end-to-end test because
//! its three systems all take `Res<Window>` and cannot be built headless.
//!
//! # Manual verification checklist
//!
//! Run `cargo run -p ui-showcase`. Each item says what to do, what should
//! happen, and what is broken if it does not. `spawn_showcase` requests a
//! 1100×700 window and pins 900×700 as its minimum size — comfortably above
//! the ~626px the LAYOUT column needs not to overflow and the ~450px
//! `MENU ▾` needs to stay unflipped — so items 1, 12 and 13 are performable
//! on a stock run with no resize, and cannot be broken by shrinking the
//! window either (the minimum stops that). The 200px between the requested
//! width and the floor is what item 13's drag has to move in. This holds as
//! long as the window actually honours the requested/minimum size; see the
//! comment in `spawn_showcase` for what happens if a compositor declines it.
//!
//! 1. **Open the dropdown.** Click `MENU ▾` (upper half of the LAYOUT
//!    column). A bordered panel opens *below* the button with their left
//!    edges flush. Wrong side means `place`'s `main_axis_origin`; a
//!    misaligned left edge means `align_cross_axis` for `UIAnchorAlign::Start`.
//! 2. **Close it again.** Click `MENU ▾` a second time. The panel closes and
//!    stays closed. If it flickers and reopens, the owner exemption in
//!    `panels_to_close` is not matching the trigger's rect, so the press
//!    dismisses the panel and `drive_panels`' toggle reopens it.
//! 3. **The closed panel is really gone.** With the dropdown closed, move
//!    the pointer slowly over where its rows just were. No row highlights
//!    and nothing of the menu is painted. A menu row that still lights up
//!    under the pointer means `track_panel_stack` is not clearing
//!    `UINode::visible` on close — the node keeps a real size in the layout
//!    pass and a live `UILayout` for the hit test. This was a real bug; it
//!    has no automated test.
//! 4. **Open the submenu.** With the dropdown open, click `Materials ▸`. A
//!    second panel opens to its *right*, painting *over* the parent menu.
//!    Painting under it means `track_panel_stack` is not giving the deeper
//!    panel a higher `z_index`, or the stack order is wrong.
//! 5. **Child rows paint with the panel.** While both are open, confirm the
//!    row labels and the submenu's text sit on top of the panel background
//!    and on top of everything behind it. Text disappearing behind another
//!    surface means `write_absolute_positions` is not inheriting the panel's
//!    layer into its children.
//! 6. **A press inside the parent closes only the submenu.** With both open,
//!    click `Point Light`. The submenu closes; the dropdown stays open. Both
//!    closing means `panels_to_close` is returning the whole stack instead of
//!    the suffix below the deepest hit panel.
//! 7. **A press outside closes both.** Reopen both, then click the
//!    FOUNDATIONS card. Both panels close in one press.
//! 8. **Escape unwinds one level per press.** Reopen both. First Escape
//!    closes the submenu only; second Escape closes the dropdown. Both
//!    closing at once means `dismiss_panels` is not taking `stack.open()`'s
//!    last entry alone.
//! 9. **A focused field takes the first Escape.** Open the `MENU ▾`
//!    dropdown, click the `Rename…` field at the bottom of it — the panel
//!    stays open, because a press on its own contents is not an outside
//!    press — and type a few characters. Press Escape **once**: the typed
//!    characters stay (Escape cancels focus, not the value — reverting the
//!    field is a listener's job, `cancel_numeric_fields` in the editor, and
//!    this showcase registers none) but the caret disappears, i.e. the field
//!    gives up focus, and the dropdown is **still open**. Press Escape
//!    **again**: the dropdown closes. If one press does both — the field
//!    loses focus and the dropdown closes together — the
//!    `field_cancelled_this_frame` arm of `should_dismiss_on_escape` is not
//!    being fed — `update_text_inputs` clears focus in the same frame it
//!    cancels, so `FocusedWidget` alone cannot tell that frame apart.
//! 10. **Context menu at the cursor.** Right-click a row in the virtual
//!     asset list. A menu opens with its top-left at the pointer, headed
//!     with that row's index. Nothing at all means the rows lost their
//!     `Interactable`, or `update_ui_interaction` is not routing the right
//!     button; a menu at the wrong place means `UIAnchorTarget::Point` is
//!     not being used as a zero-extent anchor box.
//! 11. **The context menu follows the pointer.** Right-click the *same* row
//!     a second time, at a visibly different point inside it. The menu
//!     moves to the new pointer position; its heading is unchanged, because
//!     it names the row, not the point. A menu that stays put means
//!     `compute_ui_nodes`' `panel_snapshot` early-out is not comparing the
//!     panel's `target`, so a move that changes no style skips re-layout.
//!     Right-clicking a *different* row instead does not isolate this: the
//!     row name is also written into the context menu's `TextComponent`,
//!     and text is part of the same invalidation key, so the menu would
//!     move even if `target` were not compared at all.
//! 12. **Flip.** Click `VIEW ▾`, pinned at the bottom-right of the LAYOUT
//!     column. Its dropdown opens *above* the button, because there is no
//!     room below. Opening downward and running off the bottom of the
//!     window means `place`'s `fits`/`opposite` flip is not firing. Closing
//!     it must work the same as item 2.
//! 13. **Shift, on the cross axis only.** The same `VIEW ▾` panel is wider
//!     than the room to the right of its button, so it cannot be left-flush
//!     with it: its right edge stops at the window edge and its left edge
//!     sits well left of the button, while it still hangs off the button's
//!     top edge rather than moving to the button's left. If it overhangs
//!     the window, `shift_cross_axis` is not running; if it lands to the
//!     *side* of the button, the shift is being applied to the main axis.
//!     Dragging the window narrower moves it further left, continuously,
//!     over the 200px between the 1100px the window opens at and the 900px
//!     minimum it stops at. The shift is still observable at that floor:
//!     `VIEW ▾` is right-aligned in the rightmost column, so the room to the
//!     right of it is the same handful of pixels of padding at any width,
//!     and always narrower than the 220px dropdown.
//! 14. **Closing a parent closes its child.** Open the dropdown and the
//!     `Materials ▸` submenu, then click `MENU ▾` to close the dropdown.
//!     Both disappear together — but not via the same path. The click lands
//!     inside the dropdown's *owner* rect (the trigger), so
//!     `panels_to_close` treats the dropdown as exempt and closes everything
//!     below it in the stack, i.e. the submenu, on the press itself;
//!     `drive_panels` then toggles the dropdown closed the following frame.
//!     A submenu left open while the dropdown closes means the owner
//!     exemption in `panels_to_close` is not matching the trigger's rect.
//!     Reopen the dropdown afterwards: the submenu must stay closed, not
//!     come back with it — nothing reopens a panel but its own toggle, so
//!     this checks that `open` is latched per panel, not shared state. This
//!     showcase has no reachable click sequence that leaves a panel open
//!     with its *enclosing* panel already closed, so it cannot exercise
//!     `track_panel_stack`'s orphan pass; that code path remains untested by
//!     this checklist.
//! 15. **Nothing else regressed.** The virtual list scrolls, the split
//!     handle drags, the slider drags, the checkbox toggles and the button
//!     highlights — all on the left button. A right-click on the split
//!     handle, slider, checkbox or button does nothing (the virtual list
//!     rows are excluded from this check — a right-click on one of *them* is
//!     item 10, by design). Left-button widgets reacting to the right button
//!     means a widget lost its `MouseButton::Left` filter.
//! 16. **The diagnostics line agrees.** The trailing `panels N top … side
//!     …` reads the number of open panels, the top one's origin and the
//!     side `place` resolved it to, and matches what is on screen at every
//!     step above — `side` is `Above` only while item 12's flipped panel is
//!     open.
//! 17. **A dismissed panel takes its focus with it.** Open `MENU ▾`, click
//!     the `Rename…` field so it takes the caret, then **right**-click a
//!     row in the virtual asset list. The dropdown is dismissed by that
//!     press and the context menu opens on the row. Now press Escape
//!     **once**: the context menu closes. If it takes two — the first doing
//!     nothing visible — `track_panel_stack` is not clearing
//!     `FocusedWidget` when it hides a panel the focused widget lives in, so
//!     `dismiss_panels` is still deferring Escape to a field that is no
//!     longer on screen. The same stale focus keeps `ActionMap` in text
//!     capture, which swallows every bare-letter global shortcut until the
//!     next left click.

// A system's parameter list is its data dependencies, one per resource or
// query; splitting the diagnostics readout into two systems to shorten it
// would hide those dependencies rather than reduce them. Same convention as
// the ui crate.
#![allow(clippy::too_many_arguments)]

use game_engine::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    ecs::{
        CommandQueue, Component, Entity, Query, Res, events::event_reader::EventReader,
        system::NonSendMarker,
    },
    ui::{
        UIRenderDiagnostics,
        anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack},
        checkbox::UICheckbox,
        focus::FocusedWidget,
        interaction::{Interactable, UIClick, UIInputState, UIInteractionStyle},
        material::UIMaterial,
        node::{
            AlignContent, AlignItems, FlexDirection, UILayout, UILayoutDiagnostics, UINode, UIRect,
        },
        scroll::{UIScrollArea, UISplitAxis, UISplitHandle, UISplitPane, UIVirtualList},
        slider::UISlider,
        text::{FontFamily, TextComponent},
        text_input::UITextInput,
        theme::UITheme,
        transform::UIValue,
        widgets,
    },
    window::input::MouseButton,
    window::plugin::Window,
};
use glam::Vec2;

#[derive(Component)]
struct Diagnostics;

#[derive(Component)]
struct VirtualRow {
    slot: usize,
}

#[derive(Component)]
struct Dropdown {
    trigger: Entity,
}

#[derive(Component)]
struct Submenu {
    row: Entity,
}

#[derive(Component)]
struct ContextMenu;

fn label(value: impl Into<String>, size: f32) -> TextComponent {
    TextComponent {
        text: value.into(),
        font_size: size,
        line_height: size + 5.0,
        ..Default::default()
    }
}

fn panel(width: UIValue, height: UIValue) -> UINode {
    UINode {
        width,
        height,
        flex_direction: FlexDirection::Column,
        gap: Vec2::splat(8.0),
        padding: UIRect::all(12.0),
        ..Default::default()
    }
}

/// One row of a menu. A panel's contents are the caller's — the crate only
/// places, layers and dismisses the panel around whatever children it is
/// given — so the showcase builds its rows out of ordinary UI nodes.
fn menu_row(cmd: &mut CommandQueue, theme: &UITheme, panel: Entity, name: &str) -> Entity {
    let row = cmd
        .spawn((
            UINode {
                height: UIValue::Px(theme.row_height),
                flex_shrink: 0.0,
                padding: UIRect::axes(theme.spacing_xs, theme.spacing_sm),
                ..Default::default()
            },
            UIMaterial::flat(theme.surface_raised),
            UIInteractionStyle {
                normal: theme.surface_raised,
                hovered: theme.surface_hovered,
                pressed: theme.accent,
                disabled: theme.surface,
            },
            Interactable,
            label(name, 12.0),
        ))
        .entity();
    cmd.add_child(panel, row);
    row
}

/// The shell of a floating menu: a bordered column that is a UI **root**, so
/// it paints outside the clip rect of whatever opened it.
fn menu_panel(theme: &UITheme, width: f32) -> (UINode, UIMaterial) {
    (
        UINode {
            width: UIValue::Px(width),
            flex_direction: FlexDirection::Column,
            padding: UIRect::all(theme.spacing_xs),
            visible: false,
            ..Default::default()
        },
        UIMaterial::with_border(theme.surface_raised, theme.border, 1.0),
    )
}

// `set_title` waits for the window's thread to answer, so it must run there.
fn spawn_showcase(
    _: NonSendMarker,
    mut cmd: CommandQueue,
    theme: Res<UITheme>,
    window: Res<Window>,
) {
    window.window_handle.set_title("Wonderland UI Showcase");

    // The LAYOUT column's own floor is ~626 logical px tall (see the comment
    // above `flip_row` below) and MENU's unflipped placement needs ~450 (see
    // the comment above `panel_row`). Neither is a size the OS picks on its
    // own — `WindowPlugin` sets no `inner_size` — so without this, items 1,
    // 12 and 13 depend on a person resizing taller before they can even be
    // attempted. 900x700 clears both floors with headroom, and is the floor
    // pinned below. The window is asked to *open* wider than that floor so
    // that item 13, which asks the reader to drag it narrower, has somewhere
    // to drag to: opening at the minimum would leave the window unshrinkable
    // and the item unperformable. `request_inner_size` can be refused or
    // deferred by the compositor, so its result is not trusted here;
    // `set_min_inner_size` is the actual guarantee — it stops the window
    // (however it started, or however far a person later drags it) from ever
    // getting smaller than the checklist can tolerate.
    let _ = window.request_inner_size(Vec2::new(1100.0, 700.0));
    window.set_min_inner_size(Some(Vec2::new(900.0, 700.0)));

    let root = cmd
        .spawn((
            UINode {
                width: UIValue::Percent(100.0),
                height: UIValue::Percent(100.0),
                flex_direction: FlexDirection::Column,
                gap: Vec2::splat(theme.spacing_md),
                padding: UIRect::all(theme.spacing_lg),
                ..Default::default()
            },
            UIMaterial::flat(theme.canvas),
        ))
        .entity();

    let heading = cmd
        .spawn((
            UINode {
                height: UIValue::Px(54.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            label(
                "WONDERLAND  /  UI SHOWCASE\nLooking Glass foundations and layout",
                18.0,
            ),
        ))
        .entity();
    cmd.add_child(root, heading);

    let body = cmd
        .spawn(UINode {
            flex_grow: 1.0,
            min_height: UIValue::Px(320.0),
            flex_direction: FlexDirection::Row,
            gap: Vec2::splat(theme.spacing_md),
            ..Default::default()
        })
        .entity();
    cmd.add_child(root, body);

    let foundations = cmd
        .spawn((
            panel(UIValue::Percent(46.0), UIValue::Percent(100.0)),
            UIMaterial::with_border(theme.surface, theme.border, 1.0),
        ))
        .entity();
    cmd.add_child(body, foundations);
    let foundation_title = cmd
        .spawn((
            UINode {
                height: UIValue::Px(32.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            label("FOUNDATIONS", 15.0),
        ))
        .entity();
    cmd.add_child(foundations, foundation_title);

    for (name, color) in [
        ("Canvas", theme.canvas),
        ("Surface", theme.surface_raised),
        ("Iris accent", theme.accent),
        ("Focus", theme.focus),
        ("Warning", theme.warning),
        ("Error", theme.error),
    ] {
        let swatch = cmd
            .spawn((
                UINode {
                    height: UIValue::Px(theme.row_height),
                    flex_shrink: 0.0,
                    padding: UIRect::axes(4.0, 8.0),
                    ..Default::default()
                },
                UIMaterial::flat(color),
                label(name, 13.0),
            ))
            .entity();
        cmd.add_child(foundations, swatch);
    }

    let type_sample = cmd.spawn((
        UINode { flex_grow: 1.0, min_height: UIValue::Px(90.0), ..Default::default() },
        label("Display 24\nBody 14 — warm, compact, readable\nMono 12  ABCDEFGHIJKLMNOPQRSTUVWXYZ\nUnicode  Café · 東京 · مرحبًا · 🂡", 14.0),
    )).entity();
    cmd.add_child(foundations, type_sample);

    let layout = cmd
        .spawn((
            panel(UIValue::Auto, UIValue::Percent(100.0)),
            UIMaterial::with_border(theme.surface, theme.border, 1.0),
        ))
        .entity();
    cmd.add_child(body, layout);
    let layout_title = cmd
        .spawn((
            UINode {
                height: UIValue::Px(32.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            label("LAYOUT", 15.0),
        ))
        .entity();
    cmd.add_child(layout, layout_title);

    let centered = cmd
        .spawn((
            UINode {
                height: UIValue::Px(96.0),
                min_width: UIValue::Px(260.0),
                max_width: UIValue::Px(640.0),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                gap: Vec2::splat(theme.spacing_sm),
                align_items: Some(AlignItems::Center),
                justify_content: Some(AlignContent::Center),
                padding: UIRect::all(theme.spacing_md),
                ..Default::default()
            },
            UIMaterial::flat(theme.surface_raised),
        ))
        .entity();
    cmd.add_child(layout, centered);
    for (name, width) in [("MIN", 56.0), ("FLEXIBLE", 110.0), ("MAX", 72.0)] {
        let item = cmd
            .spawn((
                UINode {
                    width: UIValue::Px(width),
                    height: UIValue::Px(theme.control_height),
                    padding: UIRect::axes(6.0, 8.0),
                    flex_shrink: 1.0,
                    ..Default::default()
                },
                UIMaterial::with_border(theme.accent, theme.focus, 1.0),
                label(name, 11.0),
            ))
            .entity();
        cmd.add_child(centered, item);
    }

    // Anchored panels. The trigger lives in the layout column; the panels
    // themselves are spawned as roots — no `add_child` from `root` — which is
    // what lets them paint outside that column's clip rect.
    //
    // This row sits high in the column on purpose: the `MENU` dropdown is the
    // one that must demonstrate the *unflipped* placement, so it needs room
    // below it, which holds once the window is at least ~450 logical px tall
    // — one of the two numbers `spawn_showcase`'s 900×700 minimum exists to
    // clear (see the comment there). Below ~450 the dropdown has nowhere to
    // go but flip, same as `VIEW`'s.
    let panels_title = cmd
        .spawn((
            UINode {
                height: UIValue::Px(32.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            label("ANCHORED PANELS", 15.0),
        ))
        .entity();
    cmd.add_child(layout, panels_title);

    let panel_row = cmd
        .spawn(UINode {
            height: UIValue::Px(theme.control_height),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            gap: Vec2::splat(theme.spacing_sm),
            ..Default::default()
        })
        .entity();
    cmd.add_child(layout, panel_row);

    let (mut menu_node, menu_material, menu_interactable, menu_style, menu_marker) =
        widgets::button(&theme);
    menu_node.width = UIValue::Px(120.0);
    let menu_trigger = cmd
        .spawn((
            menu_node,
            menu_material,
            menu_interactable,
            menu_style,
            menu_marker,
            label("MENU  \u{25be}", 12.0),
        ))
        .entity();
    cmd.add_child(panel_row, menu_trigger);

    let dropdown = cmd
        .spawn((
            menu_panel(&theme, 180.0),
            UIAnchoredPanel {
                target: UIAnchorTarget::Node {
                    entity: menu_trigger,
                },
                owner: Some(menu_trigger),
                side: UIAnchorSide::Below,
                align: UIAnchorAlign::Start,
                gap: 4.0,
                open: false,
                ..Default::default()
            },
            Dropdown {
                trigger: menu_trigger,
            },
        ))
        .entity();

    let mut materials_row = None;
    for (index, name) in [
        "Mesh Renderer",
        "Point Light",
        "Rigid Body",
        "Materials  \u{25b8}",
    ]
    .into_iter()
    .enumerate()
    {
        let row = menu_row(&mut cmd, &theme, dropdown, name);
        if index == 3 {
            materials_row = Some(row);
        }
    }
    let materials_row = materials_row.expect("the materials row is always spawned");

    // A text field *inside* the menu, which is the only place one can be
    // focused while a panel is open: focus follows the left press, and a
    // press outside the panel would dismiss it first. Escape then has two
    // claimants, and the field must win the first press.
    let rename_field = cmd
        .spawn((
            UINode {
                height: UIValue::Px(theme.control_height),
                flex_shrink: 0.0,
                padding: UIRect::axes(theme.spacing_xs, theme.spacing_sm),
                ..Default::default()
            },
            UIMaterial::with_border(theme.surface, theme.border, 1.0),
            TextComponent::default(),
            UITextInput::new("Rename…"),
            Interactable,
        ))
        .entity();
    cmd.add_child(dropdown, rename_field);

    // The submenu anchors to the row that opens it and takes no wiring beyond
    // naming it: nesting is derived from the owner chain.
    let (submenu_node, submenu_material) = menu_panel(&theme, 150.0);
    cmd.spawn((
        submenu_node,
        submenu_material,
        UIAnchoredPanel {
            target: UIAnchorTarget::Node {
                entity: materials_row,
            },
            owner: Some(materials_row),
            side: UIAnchorSide::Right,
            align: UIAnchorAlign::Start,
            gap: 2.0,
            open: false,
            ..Default::default()
        },
        Submenu { row: materials_row },
        label("Standard\nUnlit\nToon", 12.0),
    ));

    // A context menu is placed at the cursor but belongs to the row under it,
    // which is why `target` and `owner` are separate fields. Both are filled
    // in by `drive_panels` when the right-click arrives.
    let (context_node, context_material) = menu_panel(&theme, 170.0);
    cmd.spawn((
        context_node,
        context_material,
        UIAnchoredPanel::default(),
        ContextMenu,
        label("", 12.0),
    ));

    let nested = cmd.spawn((
        UINode {
            flex_grow: 1.0,
            // Lower than it once was (was 120): the fixed siblings in this
            // column — including ANCHORED PANELS' own title and trigger row
            // — add up fast, and this is the only child that shrinks, so its
            // floor sets the column's floor. See the `flip_row` comment
            // below for the arithmetic this keeps under an ordinary window.
            min_height: UIValue::Px(64.0),
            flex_direction: FlexDirection::Column,
            gap: Vec2::new(theme.spacing_sm, theme.spacing_sm),
            padding: UIRect::all(theme.spacing_md),
            ..Default::default()
        },
        UIMaterial::flat(theme.surface_raised),
        label("Nested flex / percent sizing\nResize the window to exercise min/max constraints. This intentionally long label demonstrates content bounds.", 13.0),
    )).entity();
    cmd.add_child(layout, nested);

    let controls = cmd
        .spawn(UINode {
            height: UIValue::Px(40.0),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            gap: Vec2::splat(theme.spacing_sm),
            ..Default::default()
        })
        .entity();
    cmd.add_child(layout, controls);
    let (mut button_node, material, interactable, style, marker) = widgets::button(&theme);
    button_node.width = UIValue::Px(112.0);
    let button = cmd
        .spawn((
            button_node,
            material,
            interactable,
            style,
            marker,
            label("BUTTON", 12.0),
        ))
        .entity();
    cmd.add_child(controls, button);
    let checkbox = cmd
        .spawn((
            UINode {
                width: UIValue::Px(90.0),
                height: UIValue::Px(theme.control_height),
                padding: UIRect::axes(7.0, 9.0),
                ..Default::default()
            },
            UIMaterial::with_border(theme.surface_raised, theme.border, 1.0),
            UICheckbox::new(false),
            Interactable,
            label("CHECK", 12.0),
        ))
        .entity();
    cmd.add_child(controls, checkbox);
    let slider = cmd
        .spawn((
            UINode {
                width: UIValue::Px(140.0),
                height: UIValue::Px(theme.control_height),
                ..Default::default()
            },
            UIMaterial::flat(theme.surface_raised),
            UISlider::new(0.62, 0.0, 1.0),
            Interactable,
        ))
        .entity();
    cmd.add_child(controls, slider);
    let input = cmd
        .spawn((
            UINode {
                width: UIValue::Px(180.0),
                height: UIValue::Px(theme.control_height),
                padding: UIRect::axes(7.0, 9.0),
                ..Default::default()
            },
            UIMaterial::with_border(theme.surface, theme.border, 1.0),
            TextComponent::default(),
            UITextInput::new("Unicode input…"),
            Interactable,
        ))
        .entity();
    cmd.add_child(controls, input);

    let virtual_list = cmd
        .spawn((
            UINode {
                height: UIValue::Px(96.0),
                // Shrinkable (the default), with a floor that still shows
                // two rows — see the `flip_row` comment for why this column
                // needs a second place besides `nested` to give ground.
                min_height: UIValue::Px(56.0),
                flex_direction: FlexDirection::Column,
                padding: UIRect::all(theme.spacing_sm),
                overflow_y: game_engine::ui::node::Overflow::Hidden,
                ..Default::default()
            },
            UIMaterial::flat(theme.surface_raised),
            UIScrollArea {
                offset: 0.0,
                content_extent: 280_000.0,
                content: None,
            },
            UIVirtualList::new(10_000, 28.0),
            Interactable,
        ))
        .entity();
    cmd.add_child(layout, virtual_list);
    for slot in 0..8 {
        let row = cmd
            .spawn((
                UINode {
                    height: UIValue::Px(28.0),
                    flex_shrink: 0.0,
                    ..Default::default()
                },
                // Without this the row is not hit-testable, so a
                // right-click on it never produces a `UIClick` and the
                // context menu has nothing to open from.
                Interactable,
                label("", 12.0),
                VirtualRow { slot },
            ))
            .entity();
        cmd.add_child(virtual_list, row);
    }

    let split_first = cmd
        .spawn((
            UINode::default(),
            UIMaterial::flat(theme.surface),
            label("SPLIT A", 12.0),
        ))
        .entity();
    let split_second = cmd
        .spawn((
            UINode::default(),
            UIMaterial::flat(theme.surface_raised),
            label("SPLIT B", 12.0),
        ))
        .entity();
    let split = cmd
        .spawn((
            UINode {
                height: UIValue::Px(36.0),
                min_height: UIValue::Px(24.0),
                flex_direction: FlexDirection::Row,
                gap: Vec2::splat(4.0),
                ..Default::default()
            },
            UISplitPane::new(UISplitAxis::Horizontal, split_first, split_second),
        ))
        .entity();
    cmd.add_child(layout, split);
    cmd.add_child(split, split_first);
    let split_handle = cmd
        .spawn((
            UINode {
                width: UIValue::Px(5.0),
                height: UIValue::Percent(100.0),
                flex_shrink: 0.0,
                ..Default::default()
            },
            UIMaterial::flat(theme.accent),
            Interactable,
            UISplitHandle { pane: split },
        ))
        .entity();
    cmd.add_child(split, split_handle);
    cmd.add_child(split, split_second);

    // The last child of a column whose middle child grows is pinned to the
    // bottom, and justifying it to the end pins it to the right — which is
    // the point of both. This trigger never has room for its dropdown below
    // it nor to the right of its own left edge, so one click checks the flip
    // and the cross-axis shift together, on two independent axes. Checking
    // either by resizing the window instead would make the check depend on
    // the reader guessing how small to drag it.
    //
    // "Pinned to the bottom" only holds once this column actually fits: of
    // its nine children, `nested`, `virtual_list` and `split` are the only
    // ones that shrink (the rest are `flex_shrink: 0.0`), and each has a
    // floor (`nested` 64, `virtual_list` 56, `split` 24). Below those floors
    // the column overflows and `VIEW` paints off the bottom of the window,
    // same as it did before this fix (the floor was ~800px then) — this fix
    // only lowers the floor, it does not remove it. Summing every child at
    // its floor, the two inter-block gaps and the padding this column and
    // its ancestors add: 18 (root padding) + 54 (heading) + 10 (root gap) +
    // [12 (layout padding) + 32 (LAYOUT title) + 96 (centered) + 32
    // (ANCHORED PANELS title) + 28 (panel_row) + 64 (nested) + 40 (controls)
    // + 56 (virtual_list) + 24 (split) + 28 (flip_row) + 8*8 (gaps) + 12
    // (layout padding)] + 10 (root gap) + 28 (diagnostics) = 626 logical px.
    // Below that, items 12 and 13 cannot be performed — this is the other
    // number `spawn_showcase`'s 900×700 minimum exists to clear.
    let flip_row = cmd
        .spawn(UINode {
            height: UIValue::Px(theme.control_height),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            gap: Vec2::splat(theme.spacing_sm),
            justify_content: Some(AlignContent::End),
            ..Default::default()
        })
        .entity();
    cmd.add_child(layout, flip_row);
    let (mut flip_node, flip_material, flip_interactable, flip_style, flip_marker) =
        widgets::button(&theme);
    flip_node.width = UIValue::Px(120.0);
    let flip_trigger = cmd
        .spawn((
            flip_node,
            flip_material,
            flip_interactable,
            flip_style,
            flip_marker,
            label("VIEW  \u{25be}", 12.0),
        ))
        .entity();
    cmd.add_child(flip_row, flip_trigger);

    let (flip_panel_node, flip_panel_material) = menu_panel(&theme, 220.0);
    let flip_dropdown = cmd
        .spawn((
            flip_panel_node,
            flip_panel_material,
            UIAnchoredPanel {
                target: UIAnchorTarget::Node {
                    entity: flip_trigger,
                },
                owner: Some(flip_trigger),
                side: UIAnchorSide::Below,
                align: UIAnchorAlign::Start,
                gap: 4.0,
                open: false,
                ..Default::default()
            },
            Dropdown {
                trigger: flip_trigger,
            },
        ))
        .entity();
    for name in ["Reset View", "Frame Selected", "Toggle Grid"] {
        menu_row(&mut cmd, &theme, flip_dropdown, name);
    }

    let diagnostics = cmd
        .spawn((
            UINode {
                height: UIValue::Px(28.0),
                flex_shrink: 0.0,
                padding: UIRect::axes(5.0, 8.0),
                ..Default::default()
            },
            UIMaterial::flat(theme.surface_raised),
            TextComponent {
                font_family: FontFamily::Monospace,
                ..label("", 11.0)
            },
            Diagnostics,
        ))
        .entity();
    cmd.add_child(root, diagnostics);
}

/// Opens and closes the showcase's panels.
///
/// A caller owns `open`; the crate only places, layers and dismisses. The
/// button match is what exercises per-button routing: left toggles menus,
/// right raises a context menu, and everything else is ignored.
fn drive_panels(
    mut clicks: EventReader<UIClick>,
    dropdowns: Query<(&Dropdown, &mut UIAnchoredPanel)>,
    submenus: Query<(&Submenu, &mut UIAnchoredPanel)>,
    context_menus: Query<(&ContextMenu, &mut UIAnchoredPanel, &mut TextComponent)>,
    rows: Query<&VirtualRow>,
    lists: Query<&UIVirtualList>,
) {
    for click in clicks.read() {
        match click.button {
            MouseButton::Left => {
                for (dropdown, mut panel) in dropdowns.iter() {
                    if dropdown.trigger == click.entity {
                        panel.open = !panel.open;
                    }
                }
                for (submenu, mut panel) in submenus.iter() {
                    if submenu.row == click.entity {
                        panel.open = !panel.open;
                    }
                }
            }
            MouseButton::Right => {
                // A context menu at the cursor that still names the row it is
                // about — which is how a caller identifies its subject.
                let Some(row) = rows.get_entity(click.entity) else {
                    continue;
                };
                let index = lists
                    .iter()
                    .next()
                    .and_then(|list| list.visible_range.clone().nth(row.slot));
                for (_, mut panel, mut text) in context_menus.iter() {
                    panel.target = UIAnchorTarget::Point {
                        position: click.position,
                    };
                    panel.owner = Some(click.entity);
                    panel.open = true;
                    text.text = match index {
                        Some(index) => format!("Row {index:04}\nRename\nDuplicate\nDelete"),
                        None => String::from("Rename\nDuplicate\nDelete"),
                    };
                }
            }
            _ => {}
        }
    }
}

fn update_diagnostics(
    window: Res<Window>,
    diagnostics: Res<UILayoutDiagnostics>,
    text: Query<&mut TextComponent, game_engine::ecs::With<Diagnostics>>,
    input: Res<UIInputState>,
    focus: Res<FocusedWidget>,
    lists: Query<&UIVirtualList>,
    render_diagnostics: Res<UIRenderDiagnostics>,
    stack: Res<UIPanelStack>,
    panels: Query<(&UIAnchoredPanel, &UILayout)>,
) {
    let physical = window.physical_size();
    let logical = window.logical_size();
    for mut text in text.iter() {
        text.text = format!(
            "logical {:.0}×{:.0}  physical {}×{}  DPI {:.2}  layout {}  tree {}  quads {}  text {}  bindings {}  hovered {:?}  focused {:?}  captured {:?}  virtual {:?}  panels {}  top {:?}  side {:?}",
            logical.x,
            logical.y,
            physical.0,
            physical.1,
            window.scale_factor(),
            diagnostics.layout_passes,
            diagnostics.tree_rebuilds,
            render_diagnostics.geometry_rebuilds(),
            render_diagnostics.text_reshapes(),
            render_diagnostics.binding_rebuilds(),
            input.hovered,
            **focus,
            input.captured(MouseButton::Left),
            lists.iter().next().map(|list| list.visible_range.clone()),
            stack.open().len(),
            stack
                .open()
                .last()
                .and_then(|entity| panels.get_entity(*entity))
                .map(|(_, layout)| layout.rect.min),
            stack
                .open()
                .last()
                .and_then(|entity| panels.get_entity(*entity))
                .map(|(panel, _)| panel.resolved_side),
        );
    }
}

fn update_virtual_rows(
    lists: Query<&UIVirtualList>,
    rows: Query<(&VirtualRow, &mut TextComponent)>,
) {
    let Some(list) = lists.iter().next() else {
        return;
    };
    for (slot, mut text) in rows.iter() {
        text.text = list
            .visible_range
            .clone()
            .nth(slot.slot)
            .map(|index| format!("◇  Virtual asset row {index:04}"))
            .unwrap_or_default();
    }
}

fn main() {
    env_logger::init();
    let mut app = App::new();
    app.register_plugin(DefaultPlugins::default())
        .add_system(Startup, spawn_showcase)
        .add_system(Update, update_diagnostics)
        .add_system(Update, update_virtual_rows)
        .add_system(Update, drive_panels);
    app.run();
}
