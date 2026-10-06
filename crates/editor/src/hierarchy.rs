//! The world tree: live entities, walked through `Children`, rooted at the scenes that were spawned into the world.
use crate::actions::{
    CollapseRow, DeleteEntity, DuplicateEntity, ExpandRow, RenameEntity, SelectFirst, SelectLast,
    SelectNext, SelectPrevious, TreeContext,
};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::entity_ops::EntityEdit;
use crate::fonts::{glyph, icon};
use crate::inspector::FocusNameField;
use crate::marks::{self, Mark};
use crate::scene::{SceneRoot, SceneState};
use crate::selection::Selection;
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    events::event_reader::EventReader,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::{
    anchor::{UIAnchorTarget, UIAnchoredPanel},
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{Interactable, UIClick, UIDisabled},
    material::UIMaterial,
    node::{UILayout, UINode, UIRect},
    scroll::{UIScrollArea, UIVirtualList, scroll_to_rect},
    text::UIText,
    text_input::UITextInputChanged,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use concerto_window::input::actions::{ActionFired, ActionMap};
use std::collections::HashSet;
use taffy::FlexDirection;

pub const PANEL_ID: &str = "concerto.warren";

const ROW_HEIGHT: f32 = 32.0;
const ROWS: usize = 32;
const INDENT: f32 = 12.0;
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
    visible: std::ops::Range<usize>,
    reveal: Option<usize>,
    pending_reveal: Option<Entity>,
}

impl HierarchyState {
    /// Expands the ancestors of `entity` and scrolls its row into view on the next rebuild.
    pub fn reveal_entity(&mut self, entity: Entity) {
        self.pending_reveal = Some(entity);
    }

    #[cfg(test)]
    pub(crate) fn pending_reveal(&self) -> Option<Entity> {
        self.pending_reveal
    }

    fn row(&self, slot: usize) -> Option<Row> {
        self.rows.get(self.visible.start + slot).copied()
    }

    fn index_of(&self, entity: Entity) -> Option<usize> {
        self.rows.iter().position(|row| row.entity == entity)
    }
}

#[derive(Component)]
struct TreeRegion;
#[derive(Component)]
struct TreeView;
#[derive(Component)]
struct Filter;

#[derive(Component)]
enum Label {
    Row(usize),
    Toggle(usize),
    Count(usize),
    Title,
    Position,
}

#[derive(Component)]
struct MarkSlot(usize);

#[derive(Component)]
struct AddEntityButton;

/// The shared row menu and the entity it was opened for.
#[derive(Component)]
struct RowMenu {
    target: Option<Entity>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum RowMenuItem {
    AddChild,
    Duplicate,
    Rename,
    Delete,
}

const MENU_FILL: Color = Color::srgba(0.090, 0.102, 0.157, 0.96);
const MENU_WIDTH: f32 = 168.0;

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
            .add_system(LateUpdate, sync_tree_scroll)
            .add_system(LateUpdate, sync_tree_context)
            .add_system(LateUpdate, keyboard_tree)
            .add_system(LateUpdate, entity_shortcuts)
            .add_system(LateUpdate, sync_disabled)
            .add_system(LateUpdate, sync_entity_controls)
            .add_system(LateUpdate, render_tree)
            .add_system(LateUpdate, render_marks);
    }
}

fn line(height: f32) -> UINode {
    UINode::default()
        .with_height(UIValue::Px(height))
        .with_flex_shrink(0.0)
        .with_padding(UIRect::axes(6.0, 8.0))
}

fn icon_column(width: f32) -> UINode {
    UINode::default()
        .with_size(UIValue::Px(width), UIValue::Px(ROW_HEIGHT))
        .with_flex_shrink(0.0)
        .with_padding(UIRect::axes(6.0, 0.0))
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &theme);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) {
    cmd.entity(parent).add_child_with(
        (
            theme
                .panel()
                .radius(0.0)
                .width(UIValue::Percent(100.0))
                .grow()
                .column()
                .padding(8.0)
                .clipped(),
            Interactable,
            TreeRegion,
        ),
        |mut tree| {
            tree = tree
                .add_child_with(theme.row().height(UIValue::Px(42.0)).fixed(), |header| {
                    header
                        .add_child((
                            line(42.0).with_flex_grow(1.0).with_flex_shrink(1.0),
                            theme.text("WORLD"),
                            Label::Title,
                        ))
                        .add_child((
                            theme
                                .pressable()
                                .size(UIValue::Px(24.0), UIValue::Px(24.0))
                                .padding(UIRect::axes(4.0, 5.0))
                                .on_click(add_entity),
                            icon(theme, glyph::PLUS, theme.font_size_md).muted(),
                            AddEntityButton,
                        ));
                })
                .add_child((
                    theme
                        .text_field("Search entities…")
                        .height(UIValue::Px(38.0))
                        .padding(UIRect::axes(6.0, 8.0)),
                    Filter,
                ));

            tree = tree.add_child_with(
                (
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_direction(FlexDirection::Column)
                        .clipped(),
                    Interactable,
                    TreeRegion,
                    TreeView,
                    UIVirtualList::new(0, ROW_HEIGHT).with_overscan(1),
                ),
                |mut view| {
                    let mut pool = view.spawn_child_queue(
                        UINode::default()
                            .with_flex_direction(FlexDirection::Column)
                            .with_flex_shrink(0.0),
                    );
                    let pool_entity = pool.entity();

                    for slot in 0..ROWS {
                        pool = pool.add_child_with(
                            (
                                theme
                                    .canvas()
                                    .fill(Color::TRANSPARENT)
                                    .radius_sm()
                                    .height(UIValue::Px(ROW_HEIGHT))
                                    .fixed()
                                    .row(),
                                RowSlot(slot),
                            ),
                            |row| {
                                let (mark_node, mark_material) = marks::node();
                                row.add_child((
                                    icon_column(20.0),
                                    theme
                                        .text("")
                                        .muted()
                                        .font_size(9.0)
                                        .line_height(theme.line_height(theme.font_size_md))
                                        .no_wrap(),
                                    Interactable,
                                    TreeRegion,
                                    Label::Toggle(slot),
                                    toggle_row(slot),
                                ))
                                .add_child((mark_node, mark_material, MarkSlot(slot)))
                                .add_child((
                                    line(ROW_HEIGHT).with_flex_grow(1.0).with_flex_shrink(1.0),
                                    theme.text("").single_line(),
                                    Interactable,
                                    TreeRegion,
                                    Label::Row(slot),
                                    select_row(slot),
                                ))
                                .add_child((
                                    theme
                                        .label("")
                                        .muted()
                                        .mono()
                                        .font_size(10.0)
                                        .no_wrap()
                                        .fixed()
                                        .align_self(taffy::AlignItems::Center)
                                        .margin(UIRect::axes(0.0, 8.0)),
                                    Label::Count(slot),
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

            tree.add_child((line(46.0), theme.text(""), Label::Position));
        },
    );

    let mut menu = cmd.spawn((
        theme
            .context_menu()
            .fill(MENU_FILL)
            .width(UIValue::Px(MENU_WIDTH))
            .padding(theme.spacing_xs),
        RowMenu { target: None },
    ));
    for (label, item) in [
        ("Add child", RowMenuItem::AddChild),
        ("Duplicate", RowMenuItem::Duplicate),
        ("Rename", RowMenuItem::Rename),
        ("Delete", RowMenuItem::Delete),
    ] {
        let color = if item == RowMenuItem::Delete {
            theme.error
        } else {
            theme.text
        };
        menu = menu.add_child_with(
            (
                theme
                    .pressable()
                    .row()
                    .padding(UIRect::axes(6.0, 8.0))
                    .on_click(press_row_menu_item),
                item,
            ),
            |entry| {
                entry.add_child(theme.label(label).font_size(12.5).color(color).no_wrap());
            },
        );
    }
}

fn add_entity(
    on: On<UIClick>,
    selection: Res<Selection>,
    roots: Query<(Entity, &SceneRoot)>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let root = roots.iter().next().map(|(entity, _)| entity);
    if let Some(parent) = selection.entity().or(root) {
        cmd.trigger(EntityEdit::Create { parent });
    }
}

fn press_row_menu_item(
    on: On<UIClick>,
    items: Query<&RowMenuItem>,
    menus: Query<(&RowMenu, &mut UIAnchoredPanel)>,
    mut focus_name: ResMut<FocusNameField>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    for (menu, mut panel) in menus.iter() {
        if let Some(target) = menu.target.filter(|_| panel.open) {
            match *item {
                RowMenuItem::AddChild => cmd.trigger(EntityEdit::Create { parent: target }),
                RowMenuItem::Duplicate => cmd.trigger(EntityEdit::Duplicate(target)),
                RowMenuItem::Rename => focus_name.0 = true,
                RowMenuItem::Delete => cmd.trigger(EntityEdit::Delete(target)),
            }
        }
        panel.open = false;
    }
}

fn entity_shortcuts(
    mut fired: EventReader<ActionFired>,
    selection: Res<Selection>,
    mut focus_name: ResMut<FocusNameField>,
    mut cmd: CommandQueue,
) {
    for action in fired.read() {
        let Some(entity) = selection.entity() else {
            continue;
        };
        if action.is(DeleteEntity) {
            cmd.trigger(EntityEdit::Delete(entity));
        } else if action.is(DuplicateEntity) {
            cmd.trigger(EntityEdit::Duplicate(entity));
        } else if action.is(RenameEntity) {
            focus_name.0 = true;
        }
    }
}

fn sync_entity_controls(
    roots: Query<&SceneRoot>,
    menus: Query<&RowMenu>,
    buttons: Query<(Entity, &AddEntityButton, Option<&UIDisabled>)>,
    items: Query<(Entity, &RowMenuItem, Option<&UIDisabled>)>,
    mut cmd: CommandQueue,
) {
    let mut set_disabled = |entity: Entity, wanted: bool, current: bool| {
        if wanted && !current {
            cmd.insert(UIDisabled, entity);
        } else if !wanted && current {
            cmd.remove::<UIDisabled>(entity);
        }
    };
    let no_scene = roots.iter().next().is_none();
    for (entity, _, disabled) in buttons.iter() {
        set_disabled(entity, no_scene, disabled.is_some());
    }
    let on_root = menus
        .iter()
        .next()
        .and_then(|menu| menu.target)
        .is_some_and(|target| roots.get_entity(target).is_some());
    for (entity, item, disabled) in items.iter() {
        set_disabled(
            entity,
            on_root && *item != RowMenuItem::AddChild,
            disabled.is_some(),
        );
    }
}

fn rebuild_rows(
    roots: Query<(Entity, &SceneRoot)>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    mut state: ResMut<HierarchyState>,
) {
    let revealed = state.pending_reveal.take();
    let mut ancestor = revealed.and_then(|entity| parents.get_entity(entity));
    while let Some(parent) = ancestor {
        state.expanded.insert(parent.parent());
        ancestor = parents.get_entity(parent.parent());
    }

    let mut root_entities: Vec<Entity> = roots.iter().map(|(entity, _)| entity).collect();
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
    if let Some(index) = revealed.and_then(|entity| state.index_of(entity)) {
        state.reveal = Some(index);
    }
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
    state.visible = list.visible_range();
}

fn select_row(slot: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           state: Res<HierarchyState>,
           mut selection: ResMut<Selection>,
           menus: Query<(&mut RowMenu, &mut UIAnchoredPanel)>| {
        let Some(row) = state.row(slot) else {
            return;
        };
        match on.signal().button {
            MouseButton::Left => selection.select_entity(row.entity),
            MouseButton::Right => {
                selection.select_entity(row.entity);
                for (mut menu, mut panel) in menus.iter() {
                    menu.target = Some(row.entity);
                    panel.target = UIAnchorTarget::from_point(on.signal().position);
                    panel.owner = Some(on.entity());
                    panel.open = true;
                }
            }
            _ => {}
        }
    })
    .into_listener()
}

fn toggle_row(slot: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut state: ResMut<HierarchyState>| {
        if on.signal().button == MouseButton::Left
            && let Some(row) = state.row(slot)
            && row.has_children
            && !state.expanded.remove(&row.entity)
        {
            state.expanded.insert(row.entity);
        }
    })
    .into_listener()
}

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
    labels: Query<(&Label, &mut UIText)>,
    nodes: Query<(&Label, &mut UINode)>,
) {
    for (label, mut node) in nodes.iter() {
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

fn entity_label(entity: Entity, names: &Query<&Name>, roots: &Query<&SceneRoot>) -> String {
    if let Some(root) = roots.get_entity(entity) {
        return root.address.clone();
    }
    names
        .get_entity(entity)
        .filter(|name| !name.as_str().is_empty())
        .map(|name| name.as_str().to_string())
        .unwrap_or_else(|| format!("Entity {}", entity.index()))
}

fn entity_mark(
    entity: Entity,
    has_children: bool,
    roots: &Query<&SceneRoot>,
    cameras: &Query<&concerto_render::components::camera::Camera>,
    lights: &Query<&concerto_render::components::light::Light>,
    meshes: &Query<&concerto_mesh::MeshComponent>,
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

#[allow(clippy::too_many_arguments)]
fn render_marks(
    state: Res<HierarchyState>,
    selection: Res<Selection>,
    theme: Res<UITheme>,
    roots: Query<&SceneRoot>,
    cameras: Query<&concerto_render::components::camera::Camera>,
    lights: Query<&concerto_render::components::light::Light>,
    meshes: Query<&concerto_mesh::MeshComponent>,
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
            theme.selection()
        } else {
            Color::TRANSPARENT
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_editor::{EditorDocument, EditorOwned};
    use concerto_ecs::{
        IntoSystem, System, World, events::event_channel::EventChannel, query::filter::With,
    };
    use concerto_foundation::{assets::AssetId, transform::Transform};
    use concerto_window::input::actions::ActionLabel;
    use glam::Vec2;

    struct Fixture {
        world: World,
        root: Entity,
        node: Entity,
    }

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn build(mut cmd: CommandQueue, theme: Res<UITheme>) {
        let parent = cmd.spawn(UINode::default()).entity();
        spawn_panel(&mut cmd, parent, &theme);
    }

    fn fixture() -> Fixture {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(HierarchyState::default());
        world.insert_resource(Selection::default());
        world.insert_resource(FocusNameField::default());
        world.insert_resource(EventChannel::<ActionFired>::default());
        world.add_listener(crate::entity_ops::apply_entity_edit);
        run(&mut world, build);
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            project_generation: 0,
            request_generation: 0,
            order: 0,
            status: String::new(),
            revision: 0,
            saved_revision: 0,
        });
        let root = world.spawn((
            Transform::IDENTITY,
            SceneRoot {
                asset_id: AssetId::new(),
                address: "level.gasset".into(),
            },
            EditorOwned(document),
        ));
        let node = world.spawn((Name::new("node"), Transform::IDENTITY));
        world.entity_mut(root).add_child(node);
        let mut fixture = Fixture { world, root, node };
        fixture.refresh();
        fixture
    }

    impl Fixture {
        fn refresh(&mut self) {
            run(&mut self.world, rebuild_rows);
            let state = self.world.get_resource_mut::<HierarchyState>().unwrap();
            state.visible = 0..state.rows.len();
            run(&mut self.world, sync_entity_controls);
        }
        fn expand(&mut self, entity: Entity) {
            self.world
                .get_resource_mut::<HierarchyState>()
                .unwrap()
                .expanded
                .insert(entity);
            self.refresh();
        }
        fn click(&mut self, entity: Entity, button: MouseButton) {
            self.world.trigger_on(
                entity,
                UIClick {
                    position: Vec2::new(40.0, 60.0),
                    button,
                },
            );
            self.refresh();
        }
        fn row_label(&mut self, entity: Entity) -> Entity {
            let slot = self
                .world
                .get_resource::<HierarchyState>()
                .unwrap()
                .index_of(entity)
                .expect("the entity has a visible row");
            self.world
                .query::<(Entity, &Label), ()>()
                .iter(&mut self.world)
                .find(|(_, label)| matches!(label, Label::Row(at) if *at == slot))
                .unwrap()
                .0
        }
        fn item(&mut self, wanted: RowMenuItem) -> Entity {
            self.world
                .query::<(Entity, &RowMenuItem), ()>()
                .iter(&mut self.world)
                .find(|(_, item)| **item == wanted)
                .unwrap()
                .0
        }
        fn menu(&mut self) -> (Option<Entity>, bool, UIAnchorTarget) {
            let (menu, panel) = self
                .world
                .query::<(&RowMenu, &UIAnchoredPanel), ()>()
                .iter(&mut self.world)
                .next()
                .unwrap();
            (menu.target, panel.open, panel.target)
        }
        fn selected(&self) -> Option<Entity> {
            self.world.get_resource::<Selection>().unwrap().entity()
        }
        fn children(&self, entity: Entity) -> Vec<Entity> {
            self.world
                .get_component_for_entity::<Children>(entity)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default()
        }
        fn disabled(&self, entity: Entity) -> bool {
            self.world
                .get_component_for_entity::<UIDisabled>(entity)
                .is_some()
        }
        fn fire(&mut self, action: impl ActionLabel) {
            let channel = self
                .world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap();
            channel.update();
            channel.update();
            channel.push_event(ActionFired {
                action: action.intern(),
            });
            run(&mut self.world, entity_shortcuts);
            self.refresh();
        }
        fn rename_requested(&mut self) -> bool {
            std::mem::take(&mut self.world.get_resource_mut::<FocusNameField>().unwrap().0)
        }
    }

    #[test]
    fn right_clicking_a_row_selects_it_and_opens_the_menu_at_the_pointer() {
        let mut f = fixture();
        f.expand(f.root);
        let label = f.row_label(f.node);
        f.click(label, MouseButton::Right);
        assert_eq!(f.selected(), Some(f.node));
        let (target, open, anchor) = f.menu();
        assert_eq!(target, Some(f.node));
        assert!(open);
        assert!(matches!(
            anchor,
            UIAnchorTarget::Point { position } if position == Vec2::new(40.0, 60.0)
        ));
        for item in [
            RowMenuItem::AddChild,
            RowMenuItem::Duplicate,
            RowMenuItem::Rename,
            RowMenuItem::Delete,
        ] {
            let entity = f.item(item);
            assert!(!f.disabled(entity), "{item:?}");
        }
    }

    #[test]
    fn menu_items_apply_their_edit_to_the_row_and_close_the_menu() {
        let mut f = fixture();
        f.expand(f.root);

        let label = f.row_label(f.node);
        f.click(label, MouseButton::Right);
        let add = f.item(RowMenuItem::AddChild);
        f.click(add, MouseButton::Left);
        assert_eq!(f.children(f.node).len(), 1);
        assert!(!f.menu().1);

        let label = f.row_label(f.node);
        f.click(label, MouseButton::Right);
        let duplicate = f.item(RowMenuItem::Duplicate);
        f.click(duplicate, MouseButton::Left);
        assert_eq!(f.children(f.root).len(), 2);
        let copy = f.selected().unwrap();
        assert_ne!(copy, f.node);

        let label = f.row_label(copy);
        f.click(label, MouseButton::Right);
        let rename = f.item(RowMenuItem::Rename);
        f.click(rename, MouseButton::Left);
        assert!(f.rename_requested());
        assert!(f.world.entity_is_valid(copy));

        let label = f.row_label(copy);
        f.click(label, MouseButton::Right);
        let delete = f.item(RowMenuItem::Delete);
        f.click(delete, MouseButton::Left);
        assert!(!f.world.entity_is_valid(copy));
        assert_eq!(f.children(f.root), [f.node]);
    }

    #[test]
    fn a_closed_menu_does_nothing_when_an_item_is_clicked() {
        let mut f = fixture();
        let delete = f.item(RowMenuItem::Delete);
        f.click(delete, MouseButton::Left);
        assert!(f.world.entity_is_valid(f.node));
    }

    #[test]
    fn the_scene_root_only_offers_add_child() {
        let mut f = fixture();
        let label = f.row_label(f.root);
        f.click(label, MouseButton::Right);
        for (item, disabled) in [
            (RowMenuItem::AddChild, false),
            (RowMenuItem::Duplicate, true),
            (RowMenuItem::Rename, true),
            (RowMenuItem::Delete, true),
        ] {
            let entity = f.item(item);
            assert_eq!(f.disabled(entity), disabled, "{item:?}");
        }
    }

    #[test]
    fn the_add_button_targets_the_selection_or_the_root_and_needs_a_scene() {
        let mut f = fixture();
        let button = f
            .world
            .query::<Entity, With<AddEntityButton>>()
            .iter(&mut f.world)
            .next()
            .unwrap();
        assert!(!f.disabled(button));
        f.click(button, MouseButton::Left);
        assert_eq!(f.children(f.root).len(), 2);

        f.world
            .get_resource_mut::<Selection>()
            .unwrap()
            .select_entity(f.node);
        f.click(button, MouseButton::Left);
        assert_eq!(f.children(f.node).len(), 1);

        f.world.despawn(f.root);
        f.refresh();
        assert!(f.disabled(button));
    }

    #[test]
    fn shortcuts_act_on_the_selection_and_do_nothing_without_one() {
        let mut f = fixture();
        f.fire(DeleteEntity);
        assert!(f.world.entity_is_valid(f.node));

        f.world
            .get_resource_mut::<Selection>()
            .unwrap()
            .select_entity(f.node);
        f.fire(RenameEntity);
        assert!(f.rename_requested());

        f.fire(DuplicateEntity);
        assert_eq!(f.children(f.root).len(), 2);

        f.world
            .get_resource_mut::<Selection>()
            .unwrap()
            .select_entity(f.node);
        f.fire(DeleteEntity);
        assert!(!f.world.entity_is_valid(f.node));
        assert_eq!(f.children(f.root).len(), 1);
    }

    #[test]
    fn an_edit_that_selects_an_entity_expands_its_ancestors_and_reveals_its_row() {
        let mut f = fixture();
        assert!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .index_of(f.node)
                .is_none()
        );
        f.world.trigger(EntityEdit::Create { parent: f.node });
        let created = f.selected().unwrap();
        run(&mut f.world, rebuild_rows);
        let state = f.world.get_resource::<HierarchyState>().unwrap();
        assert!(state.expanded.contains(&f.root));
        assert!(state.expanded.contains(&f.node));
        assert_eq!(state.reveal, state.index_of(created));
        assert!(state.reveal.is_some());
    }
}
