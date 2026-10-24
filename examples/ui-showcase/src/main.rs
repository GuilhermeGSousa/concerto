//! The UI showcase: every `ui` primitive on one screen.
#![allow(clippy::too_many_arguments)]

use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    ecs::{
        CommandQueue, Component, Query, Res, ResMut,
        command::EntityCommandQueue,
        signal::{On, listener::IntoListener},
    },
    ui::{
        UIRenderDiagnostics,
        anchor::{
            UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, UIPanelStack, toggle_owned_panels,
        },
        elements::{Label, Pressable, Text, prelude::*},
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

fn menu_row(theme: &UITheme, name: &str) -> (Pressable, Text) {
    (
        theme
            .pressable()
            .solid()
            .radius(0.0)
            .height(UIValue::Px(theme.row_height))
            .padding(UIRect::axes(theme.spacing_xs, theme.spacing_sm)),
        theme.text(name).font_size(12.0),
    )
}

fn section_title(theme: &UITheme, name: &str) -> Label {
    theme
        .label(name)
        .font_size(15.0)
        .height(UIValue::Px(32.0))
        .fixed()
}

fn spawn_showcase(mut cmd: CommandQueue, theme: Res<UITheme>, mut window: ResMut<Window>) {
    window.settings.title = "Concerto UI Showcase".to_string();
    window.send(WindowCommand::RequestSize(Vec2::new(1100.0, 700.0)));
    window.settings.min_size = Some(Vec2::new(900.0, 700.0));
    let theme = &*theme;

    let mut root = cmd.spawn(
        theme
            .canvas()
            .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .column()
            .gap(theme.spacing_md)
            .padding(theme.spacing_lg),
    );
    root.add_child(
        theme
            .label("CONCERTO  /  UI SHOWCASE\nLooking Glass foundations and layout")
            .font_size(18.0)
            .height(UIValue::Px(54.0))
            .fixed(),
    );

    let mut body = root.spawn_child(
        theme
            .row()
            .grow()
            .min_height(UIValue::Px(320.0))
            .gap(theme.spacing_md)
            .align_items(AlignItems::Stretch),
    );
    body.add_child_with(
        theme
            .panel()
            .radius(0.0)
            .bordered()
            .size(UIValue::Percent(24.0), UIValue::Percent(100.0))
            .column()
            .gap(8.0)
            .padding(12.0),
        |mut foundations| {
            foundations.add_child(section_title(theme, "FOUNDATIONS"));
            for (name, color) in [
                ("Canvas", theme.canvas),
                ("Surface", theme.surface_raised),
                ("Iris accent", theme.accent),
                ("Focus", theme.focus),
                ("Warning", theme.warning),
                ("Error", theme.error),
            ] {
                foundations.add_child((
                    theme
                        .canvas()
                        .fill(color)
                        .height(UIValue::Px(theme.row_height))
                        .fixed()
                        .padding(UIRect::axes(4.0, 8.0)),
                    theme.text(name).font_size(13.0),
                ));
            }
            foundations.add_child(
                theme
                    .label("Display 24\nBody 14 — warm, compact, readable\nMono 12  ABCDEFGHIJKLMNOPQRSTUVWXYZ\nUnicode  Café · 東京 · مرحبًا · 🂡")
                    .font_size(14.0)
                    .grow()
                    .min_height(UIValue::Px(90.0)),
            );
        },
    );

    let mut layout = body.spawn_child(
        theme
            .panel()
            .radius(0.0)
            .bordered()
            .size(UIValue::Auto, UIValue::Percent(100.0))
            .column()
            .gap(8.0)
            .padding(12.0),
    );
    layout
        .add_child(section_title(theme, "LAYOUT"))
        .add_child_with(
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
            |mut centered| {
                for (name, width) in [("MIN", 56.0), ("FLEXIBLE", 110.0), ("MAX", 72.0)] {
                    centered.add_child((
                        theme
                            .canvas()
                            .fill(theme.accent)
                            .border(theme.focus, 1.0)
                            .size(UIValue::Px(width), UIValue::Px(theme.control_height))
                            .padding(UIRect::axes(6.0, 8.0)),
                        theme.text(name).font_size(11.0),
                    ));
                }
            },
        )
        .add_child(section_title(theme, "ANCHORED PANELS"));

    let menu_trigger = layout
        .spawn_child(
            theme
                .row()
                .height(UIValue::Px(theme.control_height))
                .fixed(),
        )
        .spawn_child(
            theme
                .button("MENU  \u{25be}")
                .width(UIValue::Px(120.0))
                .font_size(12.0)
                .on_click(toggle_owned_panels),
        )
        .entity();

    layout
        .add_child((
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
        .add_child_with(
            theme
                .row()
                .height(UIValue::Px(40.0))
                .fixed()
                .align_items(AlignItems::Stretch),
            |mut controls| {
                controls
                    .add_child(
                        theme
                            .button("BUTTON")
                            .width(UIValue::Px(112.0))
                            .font_size(12.0),
                    )
                    .add_child(
                        theme
                            .checkbox("CHECK", false)
                            .width(UIValue::Px(90.0))
                            .shrink(1.0)
                            .padding(UIRect::axes(7.0, 9.0))
                            .font_size(12.0),
                    )
                    .add_child(theme.slider(0.62, 0.0, 1.0).width(UIValue::Px(140.0)))
                    .add_child(
                        theme
                            .text_field("Unicode input…")
                            .fill(theme.surface)
                            .width(UIValue::Px(180.0))
                            .shrink(1.0)
                            .padding(UIRect::axes(7.0, 9.0)),
                    );
            },
        )
        .add_child_with(
            (
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
            ),
            |mut list| {
                for index in 0..3_000 {
                    list.add_child((
                        theme
                            .label(format!("◇  Asset row {index:04}"))
                            .font_size(12.0),
                        open_context_menu.into_listener(),
                    ));
                }
            },
        );

    let mut split = layout.spawn_child(
        UINode::default()
            .with_height(UIValue::Px(36.0))
            .with_min_height(UIValue::Px(24.0))
            .with_flex_direction(FlexDirection::Row)
            .with_gap(Vec2::splat(4.0)),
    );
    let pane = split.entity();
    let first = split
        .spawn_child((
            theme.canvas().fill(theme.surface),
            theme.text("SPLIT A").font_size(12.0),
        ))
        .entity();
    split.add_child((
        theme
            .canvas()
            .fill(theme.accent)
            .size(UIValue::Px(5.0), UIValue::Percent(100.0))
            .fixed(),
        Interactable,
        UISplitHandle { pane },
        drag_split_handle.into_listener(),
    ));
    let second = split
        .spawn_child((
            theme.canvas().fill(theme.surface_raised),
            theme.text("SPLIT B").font_size(12.0),
        ))
        .entity();
    split.insert(UISplitPane::new(UISplitAxis::Horizontal, first, second));

    let view_trigger = layout
        .spawn_child(
            theme
                .row()
                .height(UIValue::Px(theme.control_height))
                .fixed()
                .justify(AlignContent::End),
        )
        .spawn_child(
            theme
                .button("VIEW  \u{25be}")
                .width(UIValue::Px(120.0))
                .font_size(12.0)
                .on_click(toggle_owned_panels),
        )
        .entity();

    body.spawn_child(
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
        |mut tree| tree_items(&mut tree, theme, SHOWCASE_TREE),
    );

    root.add_child((
        theme
            .card()
            .radius(0.0)
            .height(UIValue::Px(28.0))
            .fixed()
            .padding(UIRect::axes(5.0, 8.0)),
        theme.text("").mono().font_size(11.0),
        Diagnostics,
    ));

    let mut menu = cmd.spawn(
        theme
            .dropdown(menu_trigger)
            .width(UIValue::Px(180.0))
            .padding(theme.spacing_xs),
    );
    menu.add_child(menu_row(theme, "Mesh Renderer"))
        .add_child(menu_row(theme, "Point Light"))
        .add_child(menu_row(theme, "Rigid Body"));
    let mut materials = menu.spawn_child(menu_row(theme, "Materials  \u{25b8}"));
    materials.insert(toggle_owned_panels.into_listener());
    let materials = materials.entity();
    menu.add_child(theme.text_field("Rename…").fill(theme.surface));

    cmd.spawn((
        theme
            .dropdown(materials)
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

    let mut view_menu = cmd.spawn(
        theme
            .dropdown(view_trigger)
            .width(UIValue::Px(220.0))
            .padding(theme.spacing_xs),
    );
    for name in ["Reset View", "Frame Selected", "Toggle Grid"] {
        view_menu.add_child(menu_row(theme, name));
    }
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
