use std::ops::Range;

use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::Children},
    query::{Query, change_detection::Mut},
};

use crate::{
    node::{FlexDirection, Overflow, Position, UICulled, UIInset, UIIntrinsicHeight, UINode},
    scroll::UIScrollArea,
    transform::UIValue,
};

const OVERSCAN: usize = 2;

/// Where the widget placed a row: written by a `UIList` or `UITree` on each item it manages.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UIItem {
    owner: Entity,
    index: usize,
    depth: usize,
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
