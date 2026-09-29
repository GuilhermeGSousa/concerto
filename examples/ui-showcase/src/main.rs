//! The UI showcase: every `ui` primitive on one screen.
#![allow(clippy::too_many_arguments)]

use concerto::{
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
        anchor::{UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack},
        elements::prelude::*,
        focus::FocusedWidget,
        interaction::{Interactable, UIClick, UIInputState},
        node::{
            AlignContent, AlignItems, FlexDirection, Overflow, UILayout, UILayoutDiagnostics,
            UINode, UIRect,
        },
        scroll::{UIScrollArea, UISplitAxis, UISplitHandle, UISplitPane, UIVirtualList},
        text::UIText,
        theme::UITheme,
        transform::UIValue,
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
struct ContextMenu;

fn menu_row(cmd: &mut CommandQueue, theme: &UITheme, panel: Entity, name: &str) -> Entity {
    let row = cmd
        .spawn((
            theme
                .pressable()
                .solid()
                .radius(0.0)
                .height(UIValue::Px(theme.row_height))
                .padding(UIRect::axes(theme.spacing_xs, theme.spacing_sm)),
            theme.text(name).font_size(12.0),
        ))
        .entity();
    cmd.add_child(panel, row);
    row
}

fn spawn_showcase(
    _: NonSendMarker,
    mut cmd: CommandQueue,
    theme: Res<UITheme>,
    window: Res<Window>,
) {
    window.window_handle.set_title("Concerto UI Showcase");

    let _ = window.request_inner_size(Vec2::new(1100.0, 700.0));
    window.set_min_inner_size(Some(Vec2::new(900.0, 700.0)));

    let root = cmd
        .spawn(
            theme
                .canvas()
                .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                .column()
                .gap(theme.spacing_md)
                .padding(theme.spacing_lg),
        )
        .entity();

    let heading = cmd
        .spawn(
            theme
                .label("CONCERTO  /  UI SHOWCASE\nLooking Glass foundations and layout")
                .font_size(18.0)
                .height(UIValue::Px(54.0))
                .fixed(),
        )
        .entity();
    cmd.add_child(root, heading);

    let body = cmd
        .spawn(
            theme
                .row()
                .grow()
                .min_height(UIValue::Px(320.0))
                .gap(theme.spacing_md)
                .align_items(AlignItems::Stretch),
        )
        .entity();
    cmd.add_child(root, body);

    let foundations = cmd
        .spawn(
            theme
                .panel()
                .radius(0.0)
                .bordered()
                .size(UIValue::Percent(46.0), UIValue::Percent(100.0))
                .column()
                .gap(8.0)
                .padding(12.0),
        )
        .entity();
    cmd.add_child(body, foundations);
    let foundation_title = cmd
        .spawn(
            theme
                .label("FOUNDATIONS")
                .font_size(15.0)
                .height(UIValue::Px(32.0))
                .fixed(),
        )
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
                theme
                    .canvas()
                    .fill(color)
                    .height(UIValue::Px(theme.row_height))
                    .fixed()
                    .padding(UIRect::axes(4.0, 8.0)),
                theme.text(name).font_size(13.0),
            ))
            .entity();
        cmd.add_child(foundations, swatch);
    }

    let type_sample = cmd
        .spawn(
            theme
                .label("Display 24\nBody 14 — warm, compact, readable\nMono 12  ABCDEFGHIJKLMNOPQRSTUVWXYZ\nUnicode  Café · 東京 · مرحبًا · 🂡")
                .font_size(14.0)
                .grow()
                .min_height(UIValue::Px(90.0)),
        )
        .entity();
    cmd.add_child(foundations, type_sample);

    let layout = cmd
        .spawn(
            theme
                .panel()
                .radius(0.0)
                .bordered()
                .size(UIValue::Auto, UIValue::Percent(100.0))
                .column()
                .gap(8.0)
                .padding(12.0),
        )
        .entity();
    cmd.add_child(body, layout);
    let layout_title = cmd
        .spawn(
            theme
                .label("LAYOUT")
                .font_size(15.0)
                .height(UIValue::Px(32.0))
                .fixed(),
        )
        .entity();
    cmd.add_child(layout, layout_title);

    let centered = cmd
        .spawn(
            theme
                .card()
                .radius(0.0)
                .height(UIValue::Px(96.0))
                .min_width(UIValue::Px(260.0))
                .max_width(UIValue::Px(640.0))
                .fixed()
                .row()
                .gap(theme.spacing_sm)
                .justify(AlignContent::Center)
                .padding(theme.spacing_md),
        )
        .entity();
    cmd.add_child(layout, centered);
    for (name, width) in [("MIN", 56.0), ("FLEXIBLE", 110.0), ("MAX", 72.0)] {
        let item = cmd
            .spawn((
                theme
                    .canvas()
                    .fill(theme.accent)
                    .border(theme.focus, 1.0)
                    .size(UIValue::Px(width), UIValue::Px(theme.control_height))
                    .padding(UIRect::axes(6.0, 8.0)),
                theme.text(name).font_size(11.0),
            ))
            .entity();
        cmd.add_child(centered, item);
    }

    let panels_title = cmd
        .spawn(
            theme
                .label("ANCHORED PANELS")
                .font_size(15.0)
                .height(UIValue::Px(32.0))
                .fixed(),
        )
        .entity();
    cmd.add_child(layout, panels_title);

    let panel_row = cmd
        .spawn(
            theme
                .row()
                .height(UIValue::Px(theme.control_height))
                .fixed(),
        )
        .entity();
    cmd.add_child(layout, panel_row);

    let menu_trigger = cmd
        .spawn(
            theme
                .button("MENU  \u{25be}")
                .width(UIValue::Px(120.0))
                .font_size(12.0),
        )
        .entity();
    cmd.add_child(panel_row, menu_trigger);

    let dropdown = cmd
        .spawn(
            theme
                .dropdown(menu_trigger)
                .width(UIValue::Px(180.0))
                .padding(theme.spacing_xs),
        )
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

    let rename_field = cmd
        .spawn(theme.text_field("Rename…").fill(theme.surface))
        .entity();
    cmd.add_child(dropdown, rename_field);

    cmd.spawn((
        theme
            .dropdown(materials_row)
            .width(UIValue::Px(150.0))
            .padding(theme.spacing_xs)
            .anchor_side(UIAnchorSide::Right)
            .anchor_gap(2.0),
        theme.text("Standard\nUnlit\nToon").font_size(12.0),
    ));

    cmd.spawn((
        theme
            .context_menu()
            .width(UIValue::Px(170.0))
            .padding(theme.spacing_xs),
        ContextMenu,
        theme.text("").font_size(12.0),
    ));

    let nested = cmd
        .spawn((
            theme
                .card()
                .radius(0.0)
                .grow()
                .min_height(UIValue::Px(64.0))
                .column()
                .gap(theme.spacing_sm)
                .padding(theme.spacing_md),
            theme
                .text("Nested flex / percent sizing\nResize the window to exercise min/max constraints. This intentionally long label demonstrates content bounds.")
                .font_size(13.0),
        ))
        .entity();
    cmd.add_child(layout, nested);

    let controls = cmd
        .spawn(
            theme
                .row()
                .height(UIValue::Px(40.0))
                .fixed()
                .align_items(AlignItems::Stretch),
        )
        .entity();
    cmd.add_child(layout, controls);
    let button = cmd
        .spawn(
            theme
                .button("BUTTON")
                .width(UIValue::Px(112.0))
                .font_size(12.0),
        )
        .entity();
    cmd.add_child(controls, button);
    let checkbox = cmd
        .spawn(
            theme
                .checkbox("CHECK", false)
                .width(UIValue::Px(90.0))
                .shrink(1.0)
                .padding(UIRect::axes(7.0, 9.0))
                .font_size(12.0),
        )
        .entity();
    cmd.add_child(controls, checkbox);
    let slider = cmd
        .spawn(theme.slider(0.62, 0.0, 1.0).width(UIValue::Px(140.0)))
        .entity();
    cmd.add_child(controls, slider);
    let input = cmd
        .spawn(
            theme
                .text_field("Unicode input…")
                .fill(theme.surface)
                .width(UIValue::Px(180.0))
                .shrink(1.0)
                .padding(UIRect::axes(7.0, 9.0)),
        )
        .entity();
    cmd.add_child(controls, input);

    let virtual_list = cmd
        .spawn((
            theme
                .card()
                .radius(0.0)
                .height(UIValue::Px(96.0))
                .min_height(UIValue::Px(56.0))
                .column()
                .gap(0.0)
                .padding(theme.spacing_sm)
                .node(|node| node.with_overflow_y(Overflow::Hidden)),
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
                theme
                    .label("")
                    .font_size(12.0)
                    .height(UIValue::Px(28.0))
                    .fixed(),
                Interactable,
                VirtualRow { slot },
            ))
            .entity();
        cmd.add_child(virtual_list, row);
    }

    let split_first = cmd
        .spawn((
            theme.canvas().fill(theme.surface),
            theme.text("SPLIT A").font_size(12.0),
        ))
        .entity();
    let split_second = cmd
        .spawn((
            theme.canvas().fill(theme.surface_raised),
            theme.text("SPLIT B").font_size(12.0),
        ))
        .entity();
    let split = cmd
        .spawn((
            UINode::default()
                .with_height(UIValue::Px(36.0))
                .with_min_height(UIValue::Px(24.0))
                .with_flex_direction(FlexDirection::Row)
                .with_gap(Vec2::splat(4.0)),
            UISplitPane::new(UISplitAxis::Horizontal, split_first, split_second),
        ))
        .entity();
    cmd.add_child(layout, split);
    cmd.add_child(split, split_first);
    let split_handle = cmd
        .spawn((
            theme
                .canvas()
                .fill(theme.accent)
                .size(UIValue::Px(5.0), UIValue::Percent(100.0))
                .fixed(),
            Interactable,
            UISplitHandle { pane: split },
        ))
        .entity();
    cmd.add_child(split, split_handle);
    cmd.add_child(split, split_second);

    let flip_row = cmd
        .spawn(
            theme
                .row()
                .height(UIValue::Px(theme.control_height))
                .fixed()
                .justify(AlignContent::End),
        )
        .entity();
    cmd.add_child(layout, flip_row);
    let flip_trigger = cmd
        .spawn(
            theme
                .button("VIEW  \u{25be}")
                .width(UIValue::Px(120.0))
                .font_size(12.0),
        )
        .entity();
    cmd.add_child(flip_row, flip_trigger);

    let flip_dropdown = cmd
        .spawn(
            theme
                .dropdown(flip_trigger)
                .width(UIValue::Px(220.0))
                .padding(theme.spacing_xs),
        )
        .entity();
    for name in ["Reset View", "Frame Selected", "Toggle Grid"] {
        menu_row(&mut cmd, &theme, flip_dropdown, name);
    }

    let diagnostics = cmd
        .spawn((
            theme
                .card()
                .radius(0.0)
                .height(UIValue::Px(28.0))
                .fixed()
                .padding(UIRect::axes(5.0, 8.0)),
            theme.text("").mono().font_size(11.0),
            Diagnostics,
        ))
        .entity();
    cmd.add_child(root, diagnostics);
}

fn drive_panels(
    mut clicks: EventReader<UIClick>,
    context_menus: Query<(&ContextMenu, &mut UIAnchoredPanel, &mut UIText)>,
    rows: Query<&VirtualRow>,
    lists: Query<&UIVirtualList>,
) {
    for click in clicks.read() {
        if click.button != MouseButton::Right {
            continue;
        }
        let Some(row) = rows.get_entity(click.entity) else {
            continue;
        };
        let index = lists
            .iter()
            .next()
            .and_then(|list| list.visible_range().nth(row.slot));
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
}

fn update_diagnostics(
    window: Res<Window>,
    diagnostics: Res<UILayoutDiagnostics>,
    text: Query<&mut UIText, concerto::ecs::With<Diagnostics>>,
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
            input.hovered(),
            **focus,
            input.captured(MouseButton::Left),
            lists.iter().next().map(|list| list.visible_range()),
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
                .map(|(panel, _)| panel.resolved_side()),
        );
    }
}

fn update_virtual_rows(lists: Query<&UIVirtualList>, rows: Query<(&VirtualRow, &mut UIText)>) {
    let Some(list) = lists.iter().next() else {
        return;
    };
    for (slot, mut text) in rows.iter() {
        text.text = list
            .visible_range()
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
