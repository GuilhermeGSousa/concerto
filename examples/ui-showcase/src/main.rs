//! The UI showcase: every `ui` primitive on one screen.
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

fn spawn_showcase(
    _: NonSendMarker,
    mut cmd: CommandQueue,
    theme: Res<UITheme>,
    window: Res<Window>,
) {
    window.window_handle.set_title("Wonderland UI Showcase");

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
