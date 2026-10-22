//! The UI showcase: every `ui` primitive on one screen.
#![allow(clippy::too_many_arguments)]

use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    ecs::{
        CommandQueue, Component, Entity, Query, Res, ResMut,
        command::EntityCommandQueue,
        signal::{On, listener::IntoListener},
    },
    ui::{
        UIRenderDiagnostics,
        anchor::{
            UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack, toggle_owned_panels,
        },
        elements::prelude::*,
        focus::FocusedWidget,
        interaction::{Interactable, UIInputState},
        items::{UIItem, UIItemContextMenu},
        list::UIList,
        node::{
            AlignContent, AlignItems, FlexDirection, UILayout, UILayoutDiagnostics, UINode, UIRect,
        },
        scroll::{UISplitAxis, UISplitHandle, UISplitPane, drag_split_handle},
        text::UIText,
        theme::UITheme,
        transform::UIValue,
        tree::{UITree, UITreeElement},
    },
    window::{
        input::MouseButton,
        plugin::{Window, WindowCommand},
    },
};
use glam::Vec2;

#[derive(Component)]
struct Diagnostics;

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

fn spawn_showcase(mut cmd: CommandQueue, theme: Res<UITheme>, mut window: ResMut<Window>) {
    window.settings.title = "Concerto UI Showcase".to_string();
    window.send(WindowCommand::RequestSize(Vec2::new(1100.0, 700.0)));
    window.settings.min_size = Some(Vec2::new(900.0, 700.0));

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
                .size(UIValue::Percent(24.0), UIValue::Percent(100.0))
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
                .font_size(12.0)
                .on_click(toggle_owned_panels),
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
    cmd.entity(materials_row)
        .insert(toggle_owned_panels.into_listener());

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

    let list = cmd
        .spawn((
            theme
                .card()
                .radius(0.0)
                .height(UIValue::Px(96.0))
                .min_height(UIValue::Px(56.0))
                .column()
                .gap(0.0)
                .padding(theme.spacing_sm),
            UIList {
                row_height: Some(28.0),
                ..Default::default()
            },
        ))
        .entity();
    cmd.add_child(layout, list);
    for index in 0..3_000 {
        let row = cmd
            .spawn((
                theme
                    .label(format!("◇  Asset row {index:04}"))
                    .font_size(12.0),
                open_context_menu.into_listener(),
            ))
            .entity();
        cmd.add_child(list, row);
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
            drag_split_handle.into_listener(),
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
                .font_size(12.0)
                .on_click(toggle_owned_panels),
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

    cmd.entity(body)
        .spawn_child(
            theme
                .panel()
                .radius(0.0)
                .bordered()
                .size(UIValue::Px(230.0), UIValue::Percent(100.0))
                .fixed()
                .column()
                .gap(8.0)
                .padding(12.0),
        )
        .add_child_with(
            (
                theme.canvas().grow().column().gap(0.0),
                UITree {
                    row_height: Some(24.0),
                    ..Default::default()
                },
            ),
            |mut tree| tree_items(&mut tree, &theme, SHOWCASE_TREE),
        );
}

struct TreeSpec {
    name: &'static str,
    expanded: bool,
    children: &'static [TreeSpec],
}

const fn leaf(name: &'static str) -> TreeSpec {
    TreeSpec {
        name,
        expanded: false,
        children: &[],
    }
}

const fn branch(name: &'static str, expanded: bool, children: &'static [TreeSpec]) -> TreeSpec {
    TreeSpec {
        name,
        expanded,
        children,
    }
}

const SHOWCASE_TREE: &[TreeSpec] = &[
    branch(
        "World",
        true,
        &[
            branch(
                "Terrain",
                true,
                &[
                    leaf("Heightmap"),
                    branch(
                        "Foliage",
                        true,
                        &[leaf("Oak"), leaf("Birch"), leaf("Fern"), leaf("Moss")],
                    ),
                    branch("Rocks", false, &[leaf("Boulder"), leaf("Pebble")]),
                ],
            ),
            branch(
                "Lighting",
                true,
                &[
                    leaf("Sun"),
                    branch(
                        "Lamps",
                        false,
                        &[leaf("Lamp A"), leaf("Lamp B"), leaf("Lamp C")],
                    ),
                    leaf("Ambient"),
                ],
            ),
            branch("Water", false, &[leaf("Lake"), leaf("River")]),
        ],
    ),
    branch(
        "Characters",
        true,
        &[
            branch(
                "Hero",
                true,
                &[
                    leaf("Skeleton"),
                    branch(
                        "Outfit",
                        true,
                        &[leaf("Boots"), leaf("Cloak"), leaf("Belt")],
                    ),
                    leaf("Voice"),
                ],
            ),
            branch(
                "Villagers",
                false,
                &[leaf("Baker"), leaf("Smith"), leaf("Elder")],
            ),
            leaf("Companion"),
        ],
    ),
    branch(
        "UI",
        false,
        &[leaf("Health bar"), leaf("Minimap"), leaf("Dialogue box")],
    ),
    leaf("Audio"),
    branch("Scripts", true, &[leaf("Quest log"), leaf("Day cycle")]),
];

fn tree_items(parent: &mut EntityCommandQueue, theme: &UITheme, specs: &[TreeSpec]) {
    for spec in specs {
        parent.add_child_with(
            (
                UINode::default(),
                UITreeElement {
                    expanded: spec.expanded,
                },
            ),
            |mut item| {
                item.add_child(theme.label(spec.name).font_size(12.0));
                tree_items(&mut item, theme, spec.children);
            },
        );
    }
}

fn open_context_menu(
    on: On<UIItemContextMenu>,
    context_menus: Query<(&ContextMenu, &mut UIAnchoredPanel, &mut UIText)>,
    items: Query<&UIItem>,
) {
    let index = items.get_entity(on.entity()).map(|item| item.index());
    for (_, mut panel, mut text) in context_menus.iter() {
        panel.target = UIAnchorTarget::Point {
            position: on.signal().position,
        };
        panel.owner = Some(on.entity());
        panel.open = true;
        text.text = match index {
            Some(index) => format!("Row {index:04}\nRename\nDuplicate\nDelete"),
            None => String::from("Rename\nDuplicate\nDelete"),
        };
    }
}

fn update_diagnostics(
    window: Res<Window>,
    diagnostics: Res<UILayoutDiagnostics>,
    text: Query<&mut UIText, concerto::ecs::With<Diagnostics>>,
    input: Res<UIInputState>,
    focus: Res<FocusedWidget>,
    render_diagnostics: Res<UIRenderDiagnostics>,
    stack: Res<UIPanelStack>,
    panels: Query<(&UIAnchoredPanel, &UILayout)>,
) {
    let physical = window.physical_size();
    let logical = window.logical_size();
    for mut text in text.iter() {
        text.text = format!(
            "logical {:.0}×{:.0}  physical {}×{}  DPI {:.2}  layout {}  tree {}  quads {}  text {}  bindings {}  hovered {:?}  focused {:?}  captured {:?}  panels {}  top {:?}  side {:?}",
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

fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins::default())
        .add_system(Startup, spawn_showcase)
        .add_system(Update, update_diagnostics);
}

fn main() {
    env_logger::init();
    concerto::run(game);
}
