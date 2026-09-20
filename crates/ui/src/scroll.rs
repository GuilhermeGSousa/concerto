//! Scroll, virtual-list, and fixed split-pane behavior.
use ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    events::event_reader::EventReader,
    query::{Query, filter::Added},
    resource::Res,
};
use window::winit_events::WindowEvent;
use winit::event::{MouseScrollDelta, WindowEvent as WinitWindowEvent};

use crate::{
    interaction::{HoveredNode, Interactable, UIDrag},
    material::UIMaterial,
    node::{Overflow, Position, UIInset, UILayout, UINode},
    theme::UITheme,
    transform::UIValue,
};

#[derive(Component)]
pub struct UIScrollArea {
    pub offset: f32,
    pub content_extent: f32,
    pub content: Option<Entity>,
}

impl Default for UIScrollArea {
    fn default() -> Self {
        Self {
            offset: 0.0,
            content_extent: 0.0,
            content: None,
        }
    }
}

#[derive(Component)]
pub struct UIVirtualList {
    pub item_count: usize,
    pub row_height: f32,
    pub overscan: usize,
    pub visible_range: std::ops::Range<usize>,
}

impl UIVirtualList {
    pub fn new(item_count: usize, row_height: f32) -> Self {
        Self {
            item_count,
            row_height,
            overscan: 2,
            visible_range: 0..0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UISplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Component)]
pub struct UISplitPane {
    pub axis: UISplitAxis,
    pub ratio: f32,
    pub minimum_first: f32,
    pub minimum_second: f32,
    pub collapsed: Option<usize>,
    pub first: Option<Entity>,
    pub second: Option<Entity>,
}

impl UISplitPane {
    pub fn new(axis: UISplitAxis, first: Entity, second: Entity) -> Self {
        Self {
            axis,
            ratio: 0.5,
            minimum_first: 120.0,
            minimum_second: 120.0,
            collapsed: None,
            first: Some(first),
            second: Some(second),
        }
    }
}

#[derive(Component)]
pub struct UISplitHandle {
    pub pane: Entity,
}

pub(crate) fn update_scroll_areas(
    mut events: EventReader<WindowEvent>,
    hovered: Res<HoveredNode>,
    areas: Query<(Entity, &mut UIScrollArea, &UILayout)>,
    parents: Query<&ChildOf>,
) {
    let Some(hovered) = **hovered else { return };
    let mut delta = 0.0;
    for event in events.read() {
        if let WinitWindowEvent::MouseWheel { delta: wheel, .. } = &**event {
            delta += match wheel {
                MouseScrollDelta::LineDelta(_, y) => -*y * 28.0,
                MouseScrollDelta::PixelDelta(value) => -value.y as f32,
            };
        }
    }
    if delta == 0.0 {
        return;
    }
    let mut candidate = Some(hovered);
    while let Some(entity) = candidate {
        if let Some((_, mut area, layout)) = areas.get_entity(entity) {
            let max = (area.content_extent - layout.content_rect.size.y).max(0.0);
            let previous = area.offset;
            area.offset = (area.offset + delta).clamp(0.0, max);
            if area.offset != previous {
                break;
            }
        }
        candidate = parents.get_entity(entity).map(|parent| **parent);
    }
}

pub(crate) fn sync_scroll_content(
    areas: Query<(&UIScrollArea, Option<&UIVirtualList>)>,
    nodes: Query<&mut UINode>,
) {
    for (area, list) in areas.iter() {
        let Some(content) = area.content else {
            continue;
        };
        if let Some(mut node) = nodes.get_entity(content) {
            // The content is positioned by the scroll area, not laid out by the
            // panel around it. Absolute, so its height cannot feed back into
            // the viewport's own size: a taller list would otherwise grow the
            // viewport, a scrolled one shrink it, and the panel would collapse
            // as the two chased each other. Pinned left and right so it still
            // spans the viewport's width.
            node.position = Position::Absolute;
            node.inset.left = UIValue::Px(0.0);
            node.inset.right = UIValue::Px(0.0);
            node.inset.top = UIValue::Px(-content_offset(area, list));
        }
    }
}

/// How far the content node is shifted up, in logical pixels.
///
/// A plain scroll area moves its content by the whole offset. A virtualized one
/// only renders rows from `visible_range` onwards, so the content node starts at
/// that row and needs to move by the remaining distance only — otherwise the
/// list would scroll twice, once by recycling rows and again by the margin.
fn content_offset(area: &UIScrollArea, list: Option<&UIVirtualList>) -> f32 {
    match list {
        Some(list) => area.offset - list.visible_range.start as f32 * list.row_height,
        None => area.offset,
    }
}

pub(crate) fn update_virtual_lists(lists: Query<(&mut UIVirtualList, &UIScrollArea, &UILayout)>) {
    for (mut list, scroll, layout) in lists.iter() {
        let first = (scroll.offset / list.row_height).floor().max(0.0) as usize;
        let visible = (layout.content_rect.size.y / list.row_height)
            .ceil()
            .max(0.0) as usize;
        list.visible_range = first.saturating_sub(list.overscan)
            ..(first + visible + list.overscan).min(list.item_count);
    }
}

pub(crate) fn update_split_panes(
    mut drags: EventReader<UIDrag>,
    handles: Query<(&UISplitHandle, &UILayout)>,
    panes: Query<(&mut UISplitPane, &UILayout)>,
) {
    for drag in drags.read() {
        let Some(pane_entity) = handles
            .get_entity(drag.entity)
            .map(|(handle, _)| handle.pane)
        else {
            continue;
        };
        let Some((mut pane, layout)) = panes.get_entity(pane_entity) else {
            continue;
        };
        if pane.collapsed.is_some() {
            continue;
        }
        let available = available_extent(&pane, layout, pane_entity, &handles);
        if available <= 0.0 {
            continue;
        }
        let delta = match pane.axis {
            UISplitAxis::Horizontal => drag.delta.x,
            UISplitAxis::Vertical => drag.delta.y,
        };
        // The handle tracks the pointer 1:1: `first_extent` is measured in the
        // same logical pixels the drag delta is, so adding the delta moves the
        // boundary exactly as far as the cursor moved.
        let first = first_extent(&pane, available) + delta;
        pane.ratio = (first / available).clamp(0.0, 1.0);
    }
}

pub(crate) fn sync_split_panes(
    panes: Query<(Entity, &UISplitPane, &UILayout)>,
    handles: Query<(&UISplitHandle, &UILayout)>,
    nodes: Query<&mut UINode>,
) {
    for (entity, pane, layout) in panes.iter() {
        let (Some(first), Some(second)) = (pane.first, pane.second) else {
            continue;
        };
        let available = available_extent(pane, layout, entity, &handles);
        {
            let Some(mut first_node) = nodes.get_entity(first) else {
                continue;
            };
            apply_first_split(pane, available, &mut first_node);
        }
        let Some(mut second_node) = nodes.get_entity(second) else {
            continue;
        };
        apply_second_split(pane, &mut second_node);
    }
}

fn axis_extent(axis: UISplitAxis, size: glam::Vec2) -> f32 {
    match axis {
        UISplitAxis::Horizontal => size.x,
        UISplitAxis::Vertical => size.y,
    }
}

/// Space the two panes actually share: the container's content box minus the
/// handles that sit between them. Excluding the handle is what keeps
/// `first + handle + second` inside the container instead of overflowing it.
fn available_extent(
    pane: &UISplitPane,
    layout: &UILayout,
    pane_entity: Entity,
    handles: &Query<(&UISplitHandle, &UILayout)>,
) -> f32 {
    let reserved: f32 = handles
        .iter()
        .filter(|(handle, _)| handle.pane == pane_entity)
        .map(|(_, handle_layout)| axis_extent(pane.axis, handle_layout.rect.size))
        .sum();
    (axis_extent(pane.axis, layout.content_rect.size) - reserved).max(0.0)
}

/// Resolves the first pane's size in logical pixels, honouring collapse state
/// and both minimums. Minimums lose to `available` when the container is too
/// small to satisfy them, so the panes never overflow their container.
fn first_extent(pane: &UISplitPane, available: f32) -> f32 {
    match pane.collapsed {
        Some(0) => 0.0,
        Some(1) => available,
        _ => {
            let low = pane.minimum_first.clamp(0.0, available);
            let high = (available - pane.minimum_second).clamp(low, available);
            (pane.ratio * available).clamp(low, high)
        }
    }
}

/// Applies a split ratio to two caller-owned child nodes. `available` is the
/// container's content extent along the split axis, minus the handle.
pub fn apply_split(pane: &UISplitPane, available: f32, first: &mut UINode, second: &mut UINode) {
    apply_first_split(pane, available, first);
    apply_second_split(pane, second);
}

/// The first pane is sized explicitly; the second absorbs whatever is left.
/// Sizing only one side means the handle, the container gap and rounding all
/// land in the flexible pane rather than overflowing the container.
fn apply_first_split(pane: &UISplitPane, available: f32, first: &mut UINode) {
    first.visible = pane.collapsed != Some(0);
    first.flex_grow = 0.0;
    first.flex_shrink = 0.0;
    // A pane owns a fixed region of the container; its contents must not paint
    // over its neighbour when they no longer fit.
    first.overflow_x = Overflow::Hidden;
    first.overflow_y = Overflow::Hidden;
    let extent = UIValue::Px(first_extent(pane, available));
    match pane.axis {
        UISplitAxis::Horizontal => first.width = extent,
        UISplitAxis::Vertical => first.height = extent,
    }
}

fn apply_second_split(pane: &UISplitPane, second: &mut UINode) {
    second.visible = pane.collapsed != Some(1);
    second.flex_grow = 1.0;
    second.flex_shrink = 1.0;
    second.overflow_x = Overflow::Hidden;
    second.overflow_y = Overflow::Hidden;
    match pane.axis {
        UISplitAxis::Horizontal => second.width = UIValue::Auto,
        UISplitAxis::Vertical => second.height = UIValue::Auto,
    }
}

pub fn scroll_to_rect(area: &mut UIScrollArea, viewport_extent: f32, min: f32, max: f32) {
    if min < area.offset {
        area.offset = min;
    }
    if max > area.offset + viewport_extent {
        area.offset = max - viewport_extent;
    }
    area.offset = area
        .offset
        .clamp(0.0, (area.content_extent - viewport_extent).max(0.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_scroll_area_moves_its_content_by_the_whole_offset() {
        let area = UIScrollArea {
            offset: 140.0,
            content_extent: 900.0,
            content: None,
        };
        assert_eq!(content_offset(&area, None), 140.0);
    }

    #[test]
    fn a_virtual_list_only_moves_by_the_distance_row_recycling_does_not_cover() {
        let area = UIScrollArea {
            offset: 140.0,
            content_extent: 900.0,
            content: None,
        };
        let mut list = UIVirtualList::new(100, 32.0);
        // Row 4 is the first one rendered, so it already accounts for 128px.
        list.visible_range = 4..12;
        assert_eq!(content_offset(&area, Some(&list)), 12.0);
    }

    fn pane(ratio: f32) -> UISplitPane {
        UISplitPane {
            axis: UISplitAxis::Horizontal,
            ratio,
            minimum_first: 100.0,
            minimum_second: 100.0,
            collapsed: None,
            first: None,
            second: None,
        }
    }

    #[test]
    fn first_pane_is_sized_from_the_space_left_by_the_handle() {
        // 1000px container, 10px handle: the ratio applies to the 990px the
        // panes actually share, so first + handle + second == 1000.
        assert_eq!(first_extent(&pane(0.5), 990.0), 495.0);
    }

    #[test]
    fn minimums_never_push_the_panes_past_the_available_space() {
        let mut pane = pane(0.9);
        pane.minimum_first = 400.0;
        pane.minimum_second = 400.0;
        // 500px is too small for both minimums; the first pane still fits.
        let first = first_extent(&pane, 500.0);
        assert!(
            (0.0..=500.0).contains(&first),
            "{first} escaped the container"
        );
    }

    #[test]
    fn dragging_moves_the_boundary_by_exactly_the_pointer_delta() {
        let mut pane = pane(0.5);
        let available = 800.0;
        let before = first_extent(&pane, available);
        pane.ratio = ((before + 37.0) / available).clamp(0.0, 1.0);
        assert_eq!(first_extent(&pane, available) - before, 37.0);
    }

    #[test]
    fn collapsed_panes_take_all_or_none_of_the_space() {
        let mut pane = pane(0.5);
        pane.collapsed = Some(0);
        assert_eq!(first_extent(&pane, 640.0), 0.0);
        pane.collapsed = Some(1);
        assert_eq!(first_extent(&pane, 640.0), 640.0);
    }

    #[test]
    fn panes_clip_their_contents_so_they_cannot_paint_over_each_other() {
        let mut first = UINode::default();
        let mut second = UINode::default();
        apply_split(&pane(0.5), 600.0, &mut first, &mut second);
        assert_eq!(first.overflow_x, Overflow::Hidden);
        assert_eq!(second.overflow_y, Overflow::Hidden);
        // Only the first pane is sized; the second absorbs handle and rounding.
        assert_eq!(first.width, UIValue::Px(300.0));
        assert_eq!(second.width, UIValue::Auto);
        assert_eq!(first.flex_grow, 0.0);
        assert_eq!(second.flex_grow, 1.0);
    }

    #[test]
    fn scroll_to_rect_clamps_both_edges() {
        let mut area = UIScrollArea {
            offset: 50.0,
            content_extent: 300.0,
            content: None,
        };
        scroll_to_rect(&mut area, 100.0, 220.0, 240.0);
        assert_eq!(area.offset, 140.0);
        scroll_to_rect(&mut area, 100.0, 10.0, 20.0);
        assert_eq!(area.offset, 10.0);
    }
}

/// Width of a scrollbar, in logical pixels.
const BAR_WIDTH: f32 = 8.0;
/// The thumb never shrinks below this, however long the content is, so it stays
/// visible and grabbable.
const MIN_THUMB: f32 = 24.0;

/// A scrollbar track, pinned to the right edge of its scroll area.
#[derive(Component)]
pub struct UIScrollBar {
    pub area: Entity,
}

/// The draggable part of a scrollbar.
#[derive(Component)]
pub struct UIScrollThumb {
    pub area: Entity,
}

/// Where a scrollbar thumb sits within its track, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbGeometry {
    pub offset: f32,
    pub height: f32,
}

/// Thumb position and size for a scroll state, or `None` when the content fits
/// and no bar should be drawn.
///
/// The thumb's size is the visible fraction of the content, which is what makes
/// a scrollbar readable as "how much of this am I seeing"; it is clamped to
/// [`MIN_THUMB`] so a very long list still leaves something to grab, and the
/// travel is rescaled to that clamped size so the thumb still lands flush at
/// both ends.
pub fn thumb_geometry(
    offset: f32,
    content_extent: f32,
    viewport: f32,
    track: f32,
) -> Option<ThumbGeometry> {
    let scrollable = content_extent - viewport;
    if scrollable <= 0.0 || viewport <= 0.0 || track <= 0.0 {
        return None;
    }
    let height = (viewport / content_extent * track).clamp(MIN_THUMB.min(track), track);
    let travel = track - height;
    let progress = (offset / scrollable).clamp(0.0, 1.0);
    Some(ThumbGeometry {
        offset: progress * travel,
        height,
    })
}

/// Spawns a track and thumb for each new scroll area.
///
/// Absolutely positioned so the bar overlays the content instead of taking a
/// column out of it, which would reflow every panel that gained one.
pub(crate) fn setup_scrollbars(
    new_areas: Query<(Entity, &UIScrollArea), Added<UIScrollArea>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, _) in new_areas.iter() {
        let track = cmd
            .spawn((
                UINode {
                    width: UIValue::Px(BAR_WIDTH),
                    position: Position::Absolute,
                    visible: false,
                    ..Default::default()
                },
                UIMaterial::flat(theme.canvas),
                UIScrollBar { area: entity },
            ))
            .entity();
        cmd.add_child(entity, track);

        let thumb = cmd
            .spawn((
                UINode {
                    width: UIValue::Percent(100.0),
                    position: Position::Absolute,
                    ..Default::default()
                },
                UIMaterial::flat(theme.border),
                Interactable,
                UIScrollThumb { area: entity },
            ))
            .entity();
        cmd.add_child(track, thumb);
    }
}

/// Pins each track to the right edge of its area and hides it when there is
/// nothing to scroll.
pub(crate) fn sync_scrollbar_tracks(
    areas: Query<(&UIScrollArea, &UILayout)>,
    tracks: Query<(&UIScrollBar, &mut UINode)>,
) {
    for (bar, mut node) in tracks.iter() {
        let Some((area, layout)) = areas.get_entity(bar.area) else {
            continue;
        };
        let viewport = layout.content_rect.size.y;
        let visible =
            thumb_geometry(area.offset, area.content_extent, viewport, viewport).is_some();
        if node.visible != visible {
            node.visible = visible;
        }
        // Pinned to the right edge by an explicit left offset: the track has a
        // fixed width, so anchoring both sides would stretch it instead.
        let inset = UIInset {
            top: UIValue::Px(0.0),
            left: UIValue::Px((layout.content_rect.size.x - BAR_WIDTH).max(0.0)),
            ..Default::default()
        };
        if node.inset != inset {
            node.inset = inset;
        }
        let height = UIValue::Px(viewport);
        if node.height != height {
            node.height = height;
        }
    }
}

pub(crate) fn sync_scrollbar_thumbs(
    areas: Query<(&UIScrollArea, &UILayout)>,
    thumbs: Query<(&UIScrollThumb, &mut UINode)>,
) {
    for (thumb, mut node) in thumbs.iter() {
        let Some((area, layout)) = areas.get_entity(thumb.area) else {
            continue;
        };
        let viewport = layout.content_rect.size.y;
        let Some(geometry) = thumb_geometry(area.offset, area.content_extent, viewport, viewport)
        else {
            continue;
        };
        let height = UIValue::Px(geometry.height);
        if node.height != height {
            node.height = height;
        }
        let top = UIValue::Px(geometry.offset);
        if node.inset.top != top {
            node.inset.top = top;
        }
    }
}

/// Drags the view by dragging its thumb. The pointer moves `track` pixels while
/// the content moves `scrollable` pixels, so the delta is scaled between them.
pub(crate) fn drag_scrollbar_thumbs(
    mut drags: EventReader<UIDrag>,
    thumbs: Query<&UIScrollThumb>,
    areas: Query<(&mut UIScrollArea, &UILayout)>,
) {
    for drag in drags.read() {
        let Some(thumb) = thumbs.get_entity(drag.entity) else {
            continue;
        };
        let Some((mut area, layout)) = areas.get_entity(thumb.area) else {
            continue;
        };
        let viewport = layout.content_rect.size.y;
        let Some(geometry) = thumb_geometry(area.offset, area.content_extent, viewport, viewport)
        else {
            continue;
        };
        let travel = viewport - geometry.height;
        if travel <= 0.0 {
            continue;
        }
        let scrollable = area.content_extent - viewport;
        let max = scrollable.max(0.0);
        area.offset = (area.offset + drag.delta.y / travel * scrollable).clamp(0.0, max);
    }
}
