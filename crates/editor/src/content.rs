//! The project's asset catalogue, as a docked panel.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{
    Component, Query, Res, ResMut, Resource, command::CommandQueue,
    events::event_reader::EventReader,
};
use concerto_ui::{
    focus::UIFocusable,
    interaction::{Interactable, UIClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UINode, UIRect},
    scroll::{UIScrollArea, UIVirtualList},
    text::UIText,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::marks::{self, Mark, TRANSPARENT, selection_tint};
use crate::project::{AssetEntry, EditorCommand, EditorCommands, ProjectState};
use concerto_foundation::assets::AssetId;

pub const PANEL_ID: &str = "concerto.curiosities";

const ROW_HEIGHT: f32 = 28.0;
const ROWS: usize = 24;

const KINDS: [(&str, Option<&str>); 5] = [
    ("all", None),
    ("scene", Some("Scene")),
    ("mesh", Some("Mesh")),
    ("tex", Some("Texture")),
    ("mat", Some("Material")),
];

#[derive(Resource, Default)]
pub struct ContentState {
    /// Catalogue selection is independent of the open editor's entity selection.
    pub selected: Option<AssetId>,
    filter: String,
    kind: usize,
    visible: std::ops::Range<usize>,
}

impl ContentState {
    fn index(&self, slot: usize) -> usize {
        self.visible.start + slot
    }
}

#[derive(Component, Clone, Copy)]
enum Action {
    Asset(usize),
    Kind(usize),
}

#[derive(Component)]
enum Label {
    Count,
    Asset(usize),
}

#[derive(Component)]
struct MarkSlot(usize);

#[derive(Component)]
struct Tag(usize);

#[derive(Component)]
struct Filter;

#[derive(Component)]
struct ContentView;

pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ContentState::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Curiosities",
            region: Region::Rail,
        });
        app.add_system(Startup, build_panel)
            .add_system(LateUpdate, update_filter)
            .add_system(LateUpdate, sync_scroll)
            .add_system(LateUpdate, handle_actions)
            .add_system(LateUpdate, refresh_panel)
            .add_system(LateUpdate, render_marks)
            .add_system(LateUpdate, render_tags);
    }
}

fn visible_assets<'a>(project: &'a ProjectState, state: &ContentState) -> Vec<&'a AssetEntry> {
    let kind = KINDS[state.kind].1;
    project
        .project
        .iter()
        .flat_map(|project| project.filtered_assets(&state.filter, None, kind))
        .collect()
}

fn text(theme: &UITheme, value: &str) -> UIText {
    UIText {
        text: value.into(),
        font_size: theme.font_size_md,
        line_height: theme.line_height(theme.font_size_md),
        ..Default::default()
    }
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    let Some(body) = registry.body(PANEL_ID) else {
        return;
    };

    cmd.entity(body).add_child_with(
        UINode {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            padding: UIRect::all(10.0),
            ..Default::default()
        }
        .clipped(),
        |mut panel| {
            panel = panel.add_child_with(
                (
                    UINode {
                        height: UIValue::Px(30.0),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Row,
                        align_items: Some(taffy::AlignItems::Center),
                        padding: UIRect::axes(0.0, 8.0),
                        gap: glam::Vec2::new(6.0, 0.0),
                        ..Default::default()
                    },
                    UIMaterial {
                        corner_radius: theme.radius_sm,
                        ..UIMaterial::flat(theme.surface_raised)
                    },
                ),
                |search| {
                    search
                        .add_child((
                            UINode::default(),
                            UIText {
                                color: theme.text_muted,
                                wrap: false,
                                ..text(&theme, "⌕")
                            },
                        ))
                        .add_child((
                            UINode {
                                flex_grow: 1.0,
                                flex_shrink: 1.0,
                                min_width: UIValue::Px(0.0),
                                ..Default::default()
                            },
                            UIText {
                                wrap: false,
                                ellipsis: true,
                                ..text(&theme, "")
                            },
                            UITextInput::new("Find imported assets…"),
                            Interactable,
                            UIFocusable,
                            Filter,
                        ));
                },
            );

            panel = panel.add_child_with(
                UINode {
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    gap: glam::Vec2::new(4.0, 0.0),
                    margin: UIRect::axes(7.0, 0.0),
                    ..Default::default()
                },
                |mut tags| {
                    for (index, (name, _)) in KINDS.iter().enumerate() {
                        tags = tags.add_child((
                            UINode {
                                flex_shrink: 0.0,
                                padding: UIRect::axes(2.0, 7.0),
                                ..Default::default()
                            },
                            UIMaterial {
                                corner_radius: theme.radius_sm,
                                ..UIMaterial::with_border(TRANSPARENT, theme.border, 1.0)
                            },
                            UIText {
                                color: theme.text_muted,
                                font_size: 10.0,
                                line_height: theme.line_height(10.0),
                                wrap: false,
                                ..text(&theme, name)
                            },
                            Interactable,
                            Action::Kind(index),
                            Tag(index),
                        ));
                    }
                },
            );

            panel = panel.add_child_with(
                (
                    UINode {
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        flex_direction: FlexDirection::Column,
                        ..Default::default()
                    }
                    .clipped(),
                    Interactable,
                    ContentView,
                    UIVirtualList {
                        overscan: 1,
                        ..UIVirtualList::new(0, ROW_HEIGHT)
                    },
                ),
                |mut view| {
                    let mut pool = view.spawn_child_queue(UINode {
                        flex_direction: FlexDirection::Column,
                        flex_shrink: 0.0,
                        ..Default::default()
                    });
                    let pool_entity = pool.entity();

                    for slot in 0..ROWS {
                        pool = pool.add_child_with(
                            (
                                UINode {
                                    height: UIValue::Px(ROW_HEIGHT),
                                    flex_shrink: 0.0,
                                    flex_direction: FlexDirection::Row,
                                    align_items: Some(taffy::AlignItems::Center),
                                    padding: UIRect::axes(4.0, 6.0),
                                    gap: glam::Vec2::new(8.0, 0.0),
                                    ..Default::default()
                                },
                                UIMaterial {
                                    corner_radius: theme.radius_sm,
                                    ..UIMaterial::flat(TRANSPARENT)
                                },
                                Interactable,
                                UIInteractionStyle {
                                    normal: TRANSPARENT,
                                    hovered: theme.surface_hovered,
                                    pressed: selection_tint(&theme),
                                    disabled: TRANSPARENT,
                                },
                                Action::Asset(slot),
                            ),
                            |row| {
                                let (mark_node, mark_material) = marks::node();
                                row.add_child((mark_node, mark_material, MarkSlot(slot)))
                                    .add_child((
                                        UINode {
                                            flex_grow: 1.0,
                                            flex_shrink: 1.0,
                                            ..Default::default()
                                        },
                                        UIText {
                                            ellipsis: true,
                                            wrap: false,
                                            ..text(&theme, "")
                                        },
                                        Label::Asset(slot),
                                    ));
                            },
                        );
                    }

                    view.insert(UIScrollArea {
                        content: Some(pool_entity),
                        ..Default::default()
                    });
                },
            );

            panel.add_child((
                UINode {
                    height: UIValue::Px(26.0),
                    flex_shrink: 0.0,
                    padding: UIRect::axes(4.0, 10.0),
                    ..Default::default()
                },
                text(&theme, ""),
                Label::Count,
            ));
        },
    );
}

fn sync_scroll(
    views: Query<(&ContentView, &mut UIScrollArea, &mut UIVirtualList)>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
) {
    let Some((_, mut area, mut list)) = views.iter().next() else {
        return;
    };
    let count = visible_assets(&project, &state).len();
    list.item_count = count;
    area.content_extent = count as f32 * ROW_HEIGHT;
    state.visible = list.visible_range.clone();
}

fn update_filter(
    mut changes: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    views: Query<(&ContentView, &mut UIScrollArea)>,
    mut state: ResMut<ContentState>,
) {
    let mut reset = false;
    for change in changes.read() {
        if filters.get_entity(change.entity).is_some() {
            state.filter = change.value.clone();
            reset = true;
        }
    }
    if reset {
        if let Some((_, mut area)) = views.iter().next() {
            area.offset = 0.0;
        }
    }
}

fn handle_actions(
    mut clicks: EventReader<UIClick>,
    actions: Query<&Action>,
    project: Res<ProjectState>,
    mut state: ResMut<ContentState>,
    views: Query<(&ContentView, &mut UIScrollArea)>,
    mut commands: ResMut<EditorCommands>,
) {
    let mut reset = false;
    for click in clicks.read() {
        if click.button != MouseButton::Left {
            continue;
        }
        let Some(action) = actions.get_entity(click.entity) else {
            continue;
        };
        if project.busy() {
            continue;
        }
        match *action {
            Action::Kind(index) => {
                state.kind = index;
                state.visible = 0..0;
                reset = true;
            }
            Action::Asset(slot) => {
                let assets = visible_assets(&project, &state);
                if let Some(asset) = assets.get(state.index(slot)) {
                    let id = asset.id;
                    state.selected = Some(id);
                    commands.0.push_back(EditorCommand::OpenAsset(id));
                }
            }
        }
    }
    if reset {
        if let Some((_, mut area)) = views.iter().next() {
            area.offset = 0.0;
        }
    }
}

fn refresh_panel(
    project: Res<ProjectState>,
    state: Res<ContentState>,
    labels: Query<(&Label, &mut UIText)>,
) {
    let assets = visible_assets(&project, &state);

    for (label, mut component) in labels.iter() {
        let value = match label {
            Label::Count => {
                if assets.is_empty() {
                    String::new()
                } else {
                    format!("{} assets", assets.len())
                }
            }
            Label::Asset(slot) => assets
                .get(state.index(*slot))
                .map(|asset| asset.display_name.clone())
                .unwrap_or_default(),
        };
        if component.text != value {
            component.text = value;
        }
    }
}

fn asset_mark(kind: &str) -> Mark {
    match kind {
        "Scene" => Mark::Group,
        "Mesh" => Mark::Mesh,
        "Texture" => Mark::Texture,
        "Material" => Mark::Material,
        _ => Mark::Plain,
    }
}

fn render_marks(
    project: Res<ProjectState>,
    state: Res<ContentState>,
    theme: Res<UITheme>,
    slots: Query<(&MarkSlot, &mut UINode, &mut UIMaterial)>,
    styles: Query<(&Action, &mut UIInteractionStyle)>,
) {
    let assets = visible_assets(&project, &state);
    let selected = |slot: usize| {
        assets
            .get(state.index(slot))
            .is_some_and(|asset| state.selected == Some(asset.id))
    };
    for (slot, mut node, mut material) in slots.iter() {
        let mark = assets
            .get(state.index(slot.0))
            .map_or(Mark::None, |asset| asset_mark(&asset.kind));
        marks::apply(mark, &theme, selected(slot.0), &mut node, &mut material);
    }
    for (action, mut style) in styles.iter() {
        let Action::Asset(slot) = *action else {
            continue;
        };
        let normal = if selected(slot) {
            selection_tint(&theme)
        } else {
            TRANSPARENT
        };
        if style.normal != normal {
            style.normal = normal;
        }
    }
}

fn render_tags(
    state: Res<ContentState>,
    theme: Res<UITheme>,
    tags: Query<(&Tag, &mut UIMaterial, &mut UIText)>,
) {
    for (tag, mut material, mut text) in tags.iter() {
        let active = tag.0 == state.kind;
        let color = if active {
            selection_tint(&theme)
        } else {
            TRANSPARENT
        }
        .to_linear();
        if material.color != color {
            material.color = color;
        }
        let border = if active { theme.accent } else { theme.border }.to_linear();
        if material.border_color != border {
            material.border_color = border;
        }
        let ink = if active { theme.text } else { theme.text_muted };
        if text.color != ink {
            text.color = ink;
        }
    }
}
