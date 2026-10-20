use std::ops::Range;

use concerto_color::Color;
use concerto_ecs::{
    World,
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::Children},
    events::{event_reader::EventReader, event_writer::EventWriter},
    query::{Query, change_detection::Mut, filter::With},
    resource::{Res, ResMut},
    signal::{
        EntitySignal, On, Signal,
        lifecycle::{Add, Despawn},
        listener::IntoListener,
    },
};
use concerto_window::{
    define_action,
    input::{MouseButton, actions::ActionFired},
};
use glam::Vec2;

use crate::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost, UIFocusable},
    interaction::{HoveredNode, Interactable, UIClick, UIDoubleClick},
    material::UIMaterial,
    node::{
        FlexDirection, Overflow, Position, UICulled, UIInset, UIIntrinsicHeight, UILayout, UINode,
    },
    scroll::UIScrollArea,
    theme::UITheme,
    transform::UIValue,
};

const OVERSCAN: usize = 2;

define_action!(
    /// Select the next item of the focused list or tree.
    UIListNext
);
define_action!(
    /// Select the previous item of the focused list or tree.
    UIListPrevious
);
define_action!(
    /// Select the first item of the focused list or tree.
    UIListFirst
);
define_action!(
    /// Select the last item of the focused list or tree.
    UIListLast
);
define_action!(
    /// Activate the focused item of a list or tree.
    UIListActivate
);
define_action!(
    /// Expand the focused tree item, or move to its first child when it is already expanded.
    UITreeExpand
);
define_action!(
    /// Collapse the focused tree item, or move to its parent when it is already collapsed.
    UITreeCollapse
);

/// Marks the one selected item of a list or tree; inserting it on an item takes it from the others.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UISelected;

/// Marks an item that its list or tree leaves out.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UIHidden;

/// Sent to an item when the user selects it, by a left click or a navigation action.
pub struct UIItemSelected;

/// Sent to an item when the user activates it, by a double click or [`UIListActivate`].
pub struct UIItemActivated;

/// Sent to an item when the user right-clicks it, at the pointer's position.
pub struct UIItemContextMenu {
    pub position: Vec2,
}

/// Trigger on an item to scroll its list or tree until the item's row is in view.
pub struct UIReveal;

impl Signal for UIItemSelected {}
impl EntitySignal for UIItemSelected {}
impl Signal for UIItemActivated {}
impl EntitySignal for UIItemActivated {}
impl Signal for UIItemContextMenu {}
impl EntitySignal for UIItemContextMenu {}
impl Signal for UIReveal {}
impl EntitySignal for UIReveal {}

/// Registers the listeners that keep selection unique and focus off despawned items; the world needs a `FocusedWidget`.
pub fn install_listeners(world: &mut World) {
    world.add_listener(keep_selection_unique);
    world.add_listener(keep_new_item_selection_unique);
    world.add_listener(release_despawned_focus);
}

fn keep_selection_unique(
    on: On<Add<UISelected>>,
    items: Query<&UIItem>,
    selected: Query<(Entity, &UIItem), With<UISelected>>,
    mut cmd: CommandQueue,
) {
    take_selection(on.signal().entity, &items, &selected, &mut cmd);
}

fn keep_new_item_selection_unique(
    on: On<Add<UIItem>>,
    items: Query<&UIItem>,
    selected: Query<(Entity, &UIItem), With<UISelected>>,
    mut cmd: CommandQueue,
) {
    let entity = on.signal().entity;
    if selected.contains_entity(entity) {
        take_selection(entity, &items, &selected, &mut cmd);
    }
}

fn take_selection(
    entity: Entity,
    items: &Query<&UIItem>,
    selected: &Query<(Entity, &UIItem), With<UISelected>>,
    cmd: &mut CommandQueue,
) {
    let Some(item) = items.get_entity(entity) else {
        return;
    };
    for (other, other_item) in selected.iter() {
        if other != entity && other_item.owner == item.owner {
            cmd.remove::<UISelected>(other);
        }
    }
}

fn release_despawned_focus(on: On<Despawn<UIItem>>, mut focused: ResMut<FocusedWidget>) {
    if **focused == Some(on.signal().entity) {
        **focused = None;
    }
}

fn click_item(
    on: On<UIClick>,
    items: Query<&UIItem>,
    selected: Query<&UISelected>,
    mut cmd: CommandQueue,
) {
    let entity = on.entity();
    let Some(item) = items.get_entity(entity) else {
        return;
    };
    let click = on.signal();
    match click.button {
        MouseButton::Right => cmd.entity(entity).trigger(UIItemContextMenu {
            position: click.position,
        }),
        MouseButton::Left if item.selectable => select(entity, &selected, &mut cmd),
        _ => {}
    }
}

fn double_click_item(on: On<UIDoubleClick>, items: Query<&UIItem>, mut cmd: CommandQueue) {
    let entity = on.entity();
    if on.signal().button == MouseButton::Left
        && items.get_entity(entity).is_some_and(|item| item.selectable)
    {
        cmd.entity(entity).trigger(UIItemActivated);
    }
}

fn reveal_item(
    on: On<UIReveal>,
    items: Query<&UIItem>,
    owners: Query<(&mut UIScrollArea, Option<&UILayout>)>,
) {
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    let Some((mut area, layout)) = owners.get_entity(item.owner) else {
        return;
    };
    let top = item.index as f32 * item.row_height;
    let bottom = top + item.row_height;
    let offset = match layout.map(|layout| layout.content_rect.size.y) {
        Some(viewport) if viewport > 0.0 => {
            if top < area.offset {
                top
            } else if bottom > area.offset + viewport {
                bottom - viewport
            } else {
                area.offset
            }
        }
        _ => top,
    };
    if area.offset != offset {
        area.offset = offset;
    }
}

fn select(entity: Entity, selected: &Query<&UISelected>, cmd: &mut CommandQueue) {
    if !selected.contains_entity(entity) {
        cmd.insert(UISelected, entity);
    }
    cmd.entity(entity).trigger(UIItemSelected);
}

pub(crate) fn navigate_items(
    mut actions: EventReader<ActionFired>,
    mut focused: ResMut<FocusedWidget>,
    items: Query<(Entity, &UIItem)>,
    selected: Query<&UISelected>,
    mut lost: EventWriter<UIFocusLost>,
    mut gained: EventWriter<UIFocusGained>,
    mut cmd: CommandQueue,
) {
    for fired in actions.read() {
        let Some(current) = **focused else {
            continue;
        };
        let Some((_, &item)) = items.get_entity(current) else {
            continue;
        };
        if !item.selectable {
            continue;
        }
        if fired.is(UIListActivate) {
            cmd.entity(current).trigger(UIItemActivated);
            continue;
        }
        let mut siblings = items
            .iter()
            .filter(|(_, other)| other.owner == item.owner)
            .map(|(entity, other)| (other.index, entity));
        let target = if fired.is(UIListNext) {
            siblings.find(|(index, _)| *index == item.index + 1)
        } else if fired.is(UIListPrevious) {
            item.index
                .checked_sub(1)
                .and_then(|previous| siblings.find(|(index, _)| *index == previous))
        } else if fired.is(UIListFirst) {
            siblings.min_by_key(|(index, _)| *index)
        } else if fired.is(UIListLast) {
            siblings.max_by_key(|(index, _)| *index)
        } else {
            None
        };
        let Some((_, target)) = target else {
            continue;
        };
        if target == current {
            continue;
        }
        **focused = Some(target);
        lost.write(UIFocusLost(current));
        gained.write(UIFocusGained(target));
        select(target, &selected, &mut cmd);
        cmd.entity(target).trigger(UIReveal);
    }
}

pub(crate) fn paint_items(
    items: Query<(Entity, &UIItem, &mut UIMaterial, Option<&UISelected>)>,
    hovered: Res<HoveredNode>,
    theme: Res<UITheme>,
) {
    for (entity, item, mut material, selected) in items.iter() {
        let colour = if !item.selectable {
            Color::TRANSPARENT
        } else if selected.is_some() {
            theme.selection()
        } else if **hovered == Some(entity) {
            theme.surface_hovered
        } else {
            Color::TRANSPARENT
        }
        .to_linear();
        if material.color != colour {
            material.color = colour;
        }
    }
}

pub(crate) fn equip_items(
    rows: &[Entity],
    selectable: bool,
    items: &Query<&UIItem>,
    materials: &Query<&UIMaterial>,
    cmd: &mut CommandQueue,
) {
    for &row in rows {
        let previous = items.get_entity(row).map(|item| item.selectable);
        if previous.is_none() {
            cmd.insert(
                (
                    Interactable,
                    click_item.into_listener(),
                    double_click_item.into_listener(),
                    reveal_item.into_listener(),
                ),
                row,
            );
            if !materials.contains_entity(row) {
                cmd.insert(UIMaterial::flat(Color::TRANSPARENT), row);
            }
        }
        match (previous, selectable) {
            (None | Some(false), true) => cmd.insert(UIFocusable, row),
            (Some(true), false) => cmd.remove::<UIFocusable>(row),
            _ => {}
        }
    }
}

/// Where the widget placed a row: written by a `UIList` or `UITree` on each item it manages.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UIItem {
    owner: Entity,
    index: usize,
    depth: usize,
    selectable: bool,
    row_height: f32,
}

impl UIItem {
    /// The `UIList` or `UITree` entity that manages this item.
    pub fn owner(&self) -> Entity {
        self.owner
    }

    /// The item's row among the visible rows, counted from the top.
    pub fn index(&self) -> usize {
        self.index
    }

    /// How many items this one is nested under; always 0 in a list.
    pub fn depth(&self) -> usize {
        self.depth
    }
}

#[derive(Component)]
pub(crate) struct UIWidgetPart;

#[derive(Component)]
pub(crate) struct UIRowSpacer;

pub(crate) fn set_up_owner(
    owner: Entity,
    areas: &Query<&UIScrollArea>,
    children: &Query<&Children>,
    spacers: &Query<&UIRowSpacer>,
    cmd: &mut CommandQueue,
) {
    if !areas.contains_entity(owner) {
        cmd.insert(UIScrollArea::default(), owner);
    }
    let has_spacer = children
        .get_entity(owner)
        .is_some_and(|children| children.iter().any(|child| spacers.contains_entity(*child)));
    if has_spacer {
        return;
    }
    let spacer = cmd
        .spawn((
            UINode::default()
                .with_width(UIValue::Px(0.0))
                .with_flex_shrink(0.0),
            UIIntrinsicHeight(0.0),
            UIWidgetPart,
            UIRowSpacer,
        ))
        .entity();
    cmd.add_child(owner, spacer);
}

pub(crate) fn keep_parts_last(children: &mut Mut<Children>, parts: &Query<&UIWidgetPart>) {
    let mut seen_part = false;
    let out_of_order = children.iter().any(|child| {
        let part = parts.contains_entity(*child);
        let misplaced = seen_part && !part;
        seen_part |= part;
        misplaced
    });
    if out_of_order {
        children.sort_by_key(|child| parts.contains_entity(child));
    }
}

pub(crate) fn place_rows(
    rows: &[Entity],
    owner: Entity,
    row_height: f32,
    selectable: bool,
    viewport: Option<f32>,
    area: &mut Mut<UIScrollArea>,
    nodes: &Query<&mut UINode>,
    items: &Query<&UIItem>,
    culled: &Query<&UICulled>,
    cmd: &mut CommandQueue,
) {
    let extent = rows.len() as f32 * row_height;
    if area.content_extent != extent {
        area.content_extent = extent;
    }
    let viewport = viewport.unwrap_or(0.0);
    let offset = area.offset.clamp(0.0, (extent - viewport).max(0.0));
    if area.offset != offset {
        area.offset = offset;
    }

    let window = unculled_rows(rows.len(), row_height, offset, viewport);
    for (index, &row) in rows.iter().enumerate() {
        let item = UIItem {
            owner,
            index,
            depth: 0,
            selectable,
            row_height,
        };
        if items.get_entity(row).copied() != Some(item) {
            cmd.insert(item, row);
        }
        let is_culled = culled.contains_entity(row);
        if !window.contains(&index) {
            if !is_culled {
                cmd.insert(UICulled, row);
            }
            continue;
        }
        if is_culled {
            cmd.remove::<UICulled>(row);
        }
        if let Some(mut node) = nodes.get_entity(row) {
            place_row(&mut node, index as f32 * row_height - offset, row_height);
        }
    }
}

pub(crate) fn fit_owner(
    owner: Entity,
    spacer: Option<Mut<UIIntrinsicHeight>>,
    extent: f32,
    nodes: &Query<&mut UINode>,
) {
    if let Some(mut node) = nodes.get_entity(owner) {
        if node.overflow_x != Overflow::Hidden {
            node.overflow_x = Overflow::Hidden;
        }
        if node.overflow_y != Overflow::Hidden {
            node.overflow_y = Overflow::Hidden;
        }
    }
    if let Some(mut spacer) = spacer
        && spacer.0 != extent
    {
        spacer.0 = extent;
    }
}

fn unculled_rows(count: usize, row_height: f32, offset: f32, viewport: f32) -> Range<usize> {
    if row_height <= 0.0 {
        return 0..count;
    }
    let first = (offset / row_height).floor() as usize;
    let end = ((offset + viewport) / row_height).ceil() as usize;
    first.saturating_sub(OVERSCAN).min(count)..(end + OVERSCAN).min(count)
}

fn place_row(node: &mut Mut<UINode>, top: f32, height: f32) {
    if node.position != Position::Absolute {
        node.position = Position::Absolute;
    }
    let inset = UIInset {
        top: UIValue::Px(top),
        left: UIValue::Px(0.0),
        right: UIValue::Px(0.0),
        ..Default::default()
    };
    if node.inset != inset {
        node.inset = inset;
    }
    let height = UIValue::Px(height);
    if node.height != height {
        node.height = height;
    }
    if node.flex_direction != FlexDirection::Row {
        node.flex_direction = FlexDirection::Row;
    }
}
