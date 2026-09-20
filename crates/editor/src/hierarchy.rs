//! The world tree: live entities, walked through `Children`, rooted at the
//! scenes that were spawned into the world.
use crate::actions::{
    CollapseRow, ExpandRow, SelectFirst, SelectLast, SelectNext, SelectPrevious, TreeContext,
};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::marks::{self, selection_tint, Mark, TRANSPARENT};
use crate::scene::{SceneRoot, SceneState};
use crate::selection::Selection;
use app::{
    schedule_groups::{LateUpdate, Startup},
    App, Plugin,
};
use ecs::{
    command::CommandQueue, component::name::Name, entity::hierarchy::Children,
    events::event_reader::EventReader, Component, Entity, Query, Res, ResMut, Resource,
};
use std::collections::HashSet;
use taffy::FlexDirection;
use ui::{
    focus::{FocusedWidget, UIFocusable},
    interaction::{Interactable, UIClick, UIDisabled},
    material::UIMaterial,
    node::{UILayout, UINode, UIRect},
    scroll::{scroll_to_rect, UIScrollArea, UIVirtualList},
    text::TextComponent,
    text_input::{UITextInput, UITextInputChanged},
    theme::UITheme,
    transform::UIValue,
};
use window::input::actions::{ActionFired, ActionMap};

pub const PANEL_ID: &str = "rabbithole.warren";

/// Height of one tree row, in logical pixels. The scroll area measures the
/// tree in these units, so it must match the row nodes exactly.
const ROW_HEIGHT: f32 = 32.0;
/// Size of the recycled row pool. Rows beyond what the panel can show stay
/// blank, so this only needs to cover the tallest viewport we expect.
const ROWS: usize = 32;
/// Indentation per tree level, in logical pixels.
const INDENT: f32 = 12.0;
/// Depth past which rows stop indenting.
///
/// A skeleton is nested twenty levels deep; giving every level its own step
/// leaves no room for the name, and a row you cannot read is worse than one
/// whose depth you have to infer from its parents.
const MAX_INDENT_DEPTH: usize = 6;

/// One visible line of the tree.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub entity: Entity,
    pub depth: usize,
    pub has_children: bool,
}

#[derive(Resource, Default)]
pub struct HierarchyState {
    rows: Vec<Row>,
    expanded: HashSet<Entity>,
    filter: String,
    /// Rows currently mapped onto the recycled row pool, mirrored from
    /// [`UIVirtualList::visible_range`] once per frame.
    visible: std::ops::Range<usize>,
    /// A row the keyboard moved to that the scroll area must bring into view.
    reveal: Option<usize>,
}

impl HierarchyState {
    /// Document row backing pool slot `slot`, if any.
    fn row(&self, slot: usize) -> Option<Row> {
        self.rows.get(self.visible.start + slot).copied()
    }

    fn index_of(&self, entity: Entity) -> Option<usize> {
        self.rows.iter().position(|row| row.entity == entity)
    }
}

#[derive(Component)]
struct TreeRegion;
/// The tree's scrolling viewport.
#[derive(Component)]
struct TreeView;
#[derive(Component)]
struct Filter;

#[derive(Component)]
enum Action {
    Select(usize),
    Toggle(usize),
}

#[derive(Component)]
enum Label {
    Row(usize),
    Toggle(usize),
    /// How many children a group row has, shown at the end of the row.
    Count(usize),
    Title,
    Position,
}

/// The geometric mark at the head of a row.
#[derive(Component)]
struct MarkSlot(usize);

/// The row itself, which carries the selection tint.
#[derive(Component)]
struct RowSlot(usize);

pub struct HierarchyPlugin;

impl Plugin for HierarchyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HierarchyState::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Warren",
            region: Region::Rail,
        });
        app.add_system(Startup, build_panel);
        app.add_system(LateUpdate, rebuild_rows)
            .add_system(LateUpdate, filter_tree)
            // Mirrors this frame's virtual range before anything maps a pool
            // slot back onto a row.
            .add_system(LateUpdate, sync_tree_scroll)
            .add_system(LateUpdate, sync_tree_context)
            .add_system(LateUpdate, click_tree)
            .add_system(LateUpdate, keyboard_tree)
            .add_system(LateUpdate, sync_disabled)
            .add_system(LateUpdate, render_tree)
            .add_system(LateUpdate, render_marks);
    }
}

fn text(theme: &UITheme, value: &str) -> TextComponent {
    TextComponent {
        text: value.into(),
        font_size: theme.font_size_md,
        line_height: theme.line_height(theme.font_size_md),
        ..Default::default()
    }
}

fn line(height: f32) -> UINode {
    UINode {
        height: UIValue::Px(height),
        flex_shrink: 0.0,
        padding: UIRect::axes(6.0, 8.0),
        ..Default::default()
    }
}

/// A row's icon column: wide enough for the glyph and nothing else.
///
/// `line`'s horizontal padding would eat most of the box and clip the glyph to
/// a sliver, so an icon keeps only the vertical padding that centres it.
fn icon_column(width: f32) -> UINode {
    UINode {
        width: UIValue::Px(width),
        height: UIValue::Px(ROW_HEIGHT),
        flex_shrink: 0.0,
        padding: UIRect::axes(6.0, 0.0),
        ..Default::default()
    }
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &theme);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) {
    let tree = cmd
        .spawn((
            UINode {
                width: UIValue::Percent(100.0),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                padding: UIRect::all(8.0),
                ..Default::default()
            }
            .clipped(),
            UIMaterial::flat(theme.surface),
            Interactable,
            TreeRegion,
        ))
        .entity();
    cmd.add_child(parent, tree);

    let title = cmd
        .spawn((line(42.0), text(theme, "WORLD"), Label::Title))
        .entity();
    cmd.add_child(tree, title);

    let filter = cmd
        .spawn((
            line(38.0),
            text(theme, ""),
            UITextInput::new("Search entities…"),
            UIMaterial {
                corner_radius: theme.radius_md,
                ..UIMaterial::with_border(theme.canvas, theme.border, 1.0)
            },
            Interactable,
            UIFocusable,
            Filter,
        ))
        .entity();
    cmd.add_child(tree, filter);

    // The viewport clips and scrolls; the pool inside it is shifted by
    // `sync_scroll_content` and relabelled from the virtual range each frame.
    let view = cmd
        .spawn((
            UINode {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            }
            .clipped(),
            Interactable,
            TreeRegion,
            TreeView,
            UIVirtualList {
                overscan: 1,
                ..UIVirtualList::new(0, ROW_HEIGHT)
            },
        ))
        .entity();
    cmd.add_child(tree, view);

    let pool = cmd
        .spawn(UINode {
            flex_direction: FlexDirection::Column,
            flex_shrink: 0.0,
            ..Default::default()
        })
        .entity();
    cmd.add_child(view, pool);
    cmd.insert(
        UIScrollArea {
            content: Some(pool),
            ..Default::default()
        },
        view,
    );

    for slot in 0..ROWS {
        let row = cmd
            .spawn((
                UINode {
                    height: UIValue::Px(ROW_HEIGHT),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    ..Default::default()
                },
                UIMaterial {
                    corner_radius: theme.radius_sm,
                    ..UIMaterial::flat(TRANSPARENT)
                },
                RowSlot(slot),
            ))
            .entity();
        cmd.add_child(pool, row);

        let toggle = cmd
            .spawn((
                icon_column(20.0),
                TextComponent {
                    color: theme.text_muted,
                    font_size: 9.0,
                    line_height: theme.line_height(theme.font_size_md),
                    wrap: false,
                    ..text(theme, "")
                },
                Interactable,
                TreeRegion,
                Action::Toggle(slot),
                Label::Toggle(slot),
            ))
            .entity();
        cmd.add_child(row, toggle);

        let (mark_node, mark_material) = marks::node();
        let mark = cmd
            .spawn((mark_node, mark_material, MarkSlot(slot)))
            .entity();
        cmd.add_child(row, mark);

        let label = cmd
            .spawn((
                UINode {
                    flex_grow: 1.0,
                    // `line` pins its rows; the label is the one part of a row
                    // that must give, or a long name widens the row past the
                    // panel instead of ending in an ellipsis.
                    flex_shrink: 1.0,
                    ..line(ROW_HEIGHT)
                },
                TextComponent {
                    // One line per row: a long name ends in an ellipsis rather
                    // than wrapping into the row below it.
                    wrap: false,
                    ellipsis: true,
                    ..text(theme, "")
                },
                Interactable,
                TreeRegion,
                Action::Select(slot),
                Label::Row(slot),
            ))
            .entity();
        cmd.add_child(row, label);

        let count = cmd
            .spawn((
                UINode {
                    flex_shrink: 0.0,
                    align_self: Some(taffy::AlignItems::Center),
                    margin: UIRect::axes(0.0, 8.0),
                    ..Default::default()
                },
                TextComponent {
                    color: theme.text_muted,
                    font_family: ui::text::FontFamily::Monospace,
                    font_size: 10.0,
                    line_height: theme.line_height(10.0),
                    wrap: false,
                    ..text(theme, "")
                },
                Label::Count(slot),
            ))
            .entity();
        cmd.add_child(row, count);
    }

    let position = cmd
        .spawn((line(46.0), text(theme, ""), Label::Position))
        .entity();
    cmd.add_child(tree, position);
}

/// Walks the live world into a flat row list once per frame.
///
/// Rebuilding unconditionally keeps the tree honest: entities appear and vanish
/// through commands the panel never sees, so there is no reliable change signal
/// short of the walk itself. It is a tree traversal over a few hundred entities.
fn rebuild_rows(
    roots: Query<(Entity, &SceneRoot)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut state: ResMut<HierarchyState>,
) {
    let mut root_entities: Vec<Entity> = roots.iter().map(|(entity, _)| entity).collect();
    // Spawn order is not query order; sort so the tree does not reshuffle.
    root_entities.sort_by_key(|entity| (entity.index(), entity.generation()));

    let filter = state.filter.to_lowercase();
    let mut rows = Vec::new();
    if filter.is_empty() {
        for root in root_entities {
            push_rows(root, 0, &children, &state.expanded, &mut rows);
        }
    } else {
        for root in root_entities {
            push_filtered_rows(root, 0, &children, &names, &filter, &mut rows);
        }
    }
    state.rows = rows;
}

fn child_entities(children: &Query<&Children>, entity: Entity) -> Vec<Entity> {
    children
        .get_entity(entity)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default()
}

fn push_rows(
    entity: Entity,
    depth: usize,
    children: &Query<&Children>,
    expanded: &HashSet<Entity>,
    rows: &mut Vec<Row>,
) {
    let kids = child_entities(children, entity);
    rows.push(Row {
        entity,
        depth,
        has_children: !kids.is_empty(),
    });
    if expanded.contains(&entity) {
        for child in kids {
            push_rows(child, depth + 1, children, expanded, rows);
        }
    }
}

/// Emits `entity` when it or any descendant matches, so a match is never
/// orphaned from the path that leads to it. Filtered branches are always
/// expanded — collapsing while searching only hides what you searched for.
fn push_filtered_rows(
    entity: Entity,
    depth: usize,
    children: &Query<&Children>,
    names: &Query<&Name>,
    filter: &str,
    rows: &mut Vec<Row>,
) -> bool {
    let kids = child_entities(children, entity);
    let at = rows.len();
    rows.push(Row {
        entity,
        depth,
        has_children: !kids.is_empty(),
    });

    let mut kept = names
        .get_entity(entity)
        .is_some_and(|name| name.as_str().to_lowercase().contains(filter));
    for child in kids {
        kept |= push_filtered_rows(child, depth + 1, children, names, filter, rows);
    }
    if !kept {
        rows.truncate(at);
    }
    kept
}

fn filter_tree(
    mut events: EventReader<UITextInputChanged>,
    filters: Query<&Filter>,
    mut state: ResMut<HierarchyState>,
) {
    for event in events.read() {
        if filters.get_entity(event.entity).is_some() {
            state.filter = event.value.clone();
            state.reveal = Some(0);
        }
    }
}

/// Bridges the row list and the shared scroll area: publishes the row count the
/// scroll maths needs, mirrors the resulting range for the systems that map
/// pool slots back onto rows, and honours keyboard reveal requests.
fn sync_tree_scroll(
    views: Query<(&TreeView, &mut UIScrollArea, &mut UIVirtualList, &UILayout)>,
    mut state: ResMut<HierarchyState>,
) {
    let Some((_, mut area, mut list, layout)) = views.iter().next() else {
        return;
    };
    list.item_count = state.rows.len();
    area.content_extent = state.rows.len() as f32 * ROW_HEIGHT;

    if let Some(row) = state.reveal.take() {
        let top = row as f32 * ROW_HEIGHT;
        scroll_to_rect(&mut area, layout.content_rect.size.y, top, top + ROW_HEIGHT);
    }
    state.visible = list.visible_range.clone();
}

fn click_tree(
    mut events: EventReader<UIClick>,
    actions: Query<&Action>,
    mut state: ResMut<HierarchyState>,
    mut selection: ResMut<Selection>,
) {
    for event in events.read() {
        let Some(action) = actions.get_entity(event.entity) else {
            continue;
        };
        match *action {
            Action::Select(slot) => {
                if let Some(row) = state.row(slot) {
                    selection.select_entity(row.entity);
                }
            }
            Action::Toggle(slot) => {
                if let Some(row) = state.row(slot) {
                    if row.has_children && !state.expanded.remove(&row.entity) {
                        state.expanded.insert(row.entity);
                    }
                }
            }
        }
    }
}

/// The tree claims its keys only while focus is inside it, so arrow keys mean
/// something else everywhere they should.
fn sync_tree_context(
    focus: Res<FocusedWidget>,
    regions: Query<&TreeRegion>,
    mut actions: ResMut<ActionMap>,
) {
    let focused = (**focus).is_some_and(|entity| regions.get_entity(entity).is_some());
    if focused {
        actions.push_context(TreeContext);
    } else {
        actions.pop_context(TreeContext);
    }
}

fn keyboard_tree(
    mut actions: EventReader<ActionFired>,
    mut state: ResMut<HierarchyState>,
    mut selection: ResMut<Selection>,
) {
    if state.rows.is_empty() {
        return;
    }
    for fired in actions.read() {
        let position = selection.entity().and_then(|entity| state.index_of(entity));
        let last = state.rows.len() - 1;

        let moved = if fired.is(SelectFirst) {
            Some(0)
        } else if fired.is(SelectLast) {
            Some(last)
        } else if fired.is(SelectNext) {
            Some(position.map_or(0, |at| (at + 1).min(last)))
        } else if fired.is(SelectPrevious) {
            Some(position.unwrap_or(0).saturating_sub(1))
        } else {
            None
        };
        if let Some(next) = moved {
            selection.select_entity(state.rows[next].entity);
            state.reveal = Some(next);
            continue;
        }

        let Some(position) = position else { continue };
        let row = state.rows[position];
        if fired.is(ExpandRow) && row.has_children {
            state.expanded.insert(row.entity);
        } else if fired.is(CollapseRow) && !state.expanded.remove(&row.entity) {
            // Already collapsed: step out to the parent, the usual tree contract.
            let parent = (0..position)
                .rev()
                .map(|index| state.rows[index])
                .find(|candidate| candidate.depth < row.depth);
            if let Some(parent) = parent {
                selection.select_entity(parent.entity);
                state.reveal = state.index_of(parent.entity);
            }
        }
    }
}

fn sync_disabled(
    state: Res<HierarchyState>,
    labels: Query<(Entity, &Label, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    for (entity, label, disabled) in labels.iter() {
        let slot = match label {
            Label::Row(slot) | Label::Toggle(slot) => *slot,
            _ => continue,
        };
        let row = state.row(slot);
        let inactive = match label {
            Label::Toggle(_) => row.is_none_or(|row| !row.has_children),
            _ => row.is_none(),
        };
        if inactive && disabled.is_none() {
            cmd.insert(UIDisabled, entity);
        } else if !inactive && disabled.is_some() {
            cmd.remove::<UIDisabled>(entity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_tree(
    state: Res<HierarchyState>,
    scenes: Res<SceneState>,
    names: Query<&Name>,
    roots: Query<&SceneRoot>,
    children: Query<&Children>,
    labels: Query<(&Label, &mut TextComponent)>,
    nodes: Query<(&Label, &mut UINode)>,
) {
    for (label, mut node) in nodes.iter() {
        // Indentation is a layout property, not part of the label text, so a
        // long name truncates against the panel edge rather than the margin.
        if let Label::Toggle(slot) = label {
            let depth = state.row(*slot).map_or(0, |row| row.depth);
            node.margin.left = depth.min(MAX_INDENT_DEPTH) as f32 * INDENT;
        }
    }

    for (label, mut component) in labels.iter() {
        let row = match label {
            Label::Row(slot) | Label::Toggle(slot) | Label::Count(slot) => state.row(*slot),
            _ => None,
        };
        let value = match label {
            Label::Title => format!("WORLD · {} entities", state.rows.len()),
            Label::Position => {
                if state.rows.is_empty() {
                    if scenes.loading() {
                        "Loading…".into()
                    } else {
                        String::new()
                    }
                } else {
                    format!(
                        "{}–{} of {}",
                        state.visible.start + 1,
                        state.visible.end.min(state.rows.len()),
                        state.rows.len()
                    )
                }
            }
            Label::Row(_) => row
                .map(|row| entity_label(row.entity, &names, &roots))
                .unwrap_or_default(),
            Label::Toggle(_) => row
                .map(|row| {
                    if !row.has_children {
                        String::new()
                    } else if state.expanded.contains(&row.entity) || !state.filter.is_empty() {
                        "▾".to_string()
                    } else {
                        "▸".to_string()
                    }
                })
                .unwrap_or_default(),
            Label::Count(_) => row
                .filter(|row| row.has_children)
                .map(|row| child_entities(&children, row.entity).len().to_string())
                .unwrap_or_default(),
        };
        if component.text != value {
            component.text = value;
        }
    }
}

/// A scene root shows the asset it came from; everything else shows its name,
/// falling back to the entity index so a nameless entity is still selectable.
fn entity_label(entity: Entity, names: &Query<&Name>, roots: &Query<&SceneRoot>) -> String {
    if let Some(root) = roots.get_entity(entity) {
        return root.address.clone();
    }
    names
        .get_entity(entity)
        .map(|name| name.as_str().to_string())
        .unwrap_or_else(|| format!("Entity {}", entity.index()))
}

/// What an entity is, as far as the tree can tell from what it carries.
fn entity_mark(
    entity: Entity,
    has_children: bool,
    roots: &Query<&SceneRoot>,
    cameras: &Query<&render::components::camera::Camera>,
    lights: &Query<&render::components::light::Light>,
    meshes: &Query<&mesh::MeshComponent>,
) -> Mark {
    if roots.get_entity(entity).is_some() {
        Mark::Group
    } else if cameras.get_entity(entity).is_some() {
        Mark::Camera
    } else if lights.get_entity(entity).is_some() {
        Mark::Light
    } else if meshes.get_entity(entity).is_some() {
        Mark::Mesh
    } else if has_children {
        Mark::Group
    } else {
        Mark::Plain
    }
}

/// Draws each pooled row's mark from what its entity carries.
#[allow(clippy::too_many_arguments)]
fn render_marks(
    state: Res<HierarchyState>,
    selection: Res<Selection>,
    theme: Res<UITheme>,
    roots: Query<&SceneRoot>,
    cameras: Query<&render::components::camera::Camera>,
    lights: Query<&render::components::light::Light>,
    meshes: Query<&mesh::MeshComponent>,
    slots: Query<(&MarkSlot, &mut UINode, &mut UIMaterial)>,
    rows: Query<(&RowSlot, &mut UIMaterial)>,
) {
    let selected_row = |slot: usize| {
        state
            .row(slot)
            .is_some_and(|row| selection.entity() == Some(row.entity))
    };
    for (slot, mut material) in rows.iter() {
        let color = if selected_row(slot.0) {
            selection_tint(&theme)
        } else {
            TRANSPARENT
        }
        .to_linear();
        if material.color != color {
            material.color = color;
        }
    }
    for (slot, mut node, mut material) in slots.iter() {
        let row = state.row(slot.0);
        let mark = row.map_or(Mark::None, |row| {
            entity_mark(
                row.entity,
                row.has_children,
                &roots,
                &cameras,
                &lights,
                &meshes,
            )
        });
        marks::apply(mark, &theme, selected_row(slot.0), &mut node, &mut material);
    }
}
