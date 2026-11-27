//! Scroll and fixed split-pane behavior.
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    events::event_reader::EventReader,
    query::{Query, filter::Added},
    resource::Res,
    signal::{On, listener::IntoListener},
};
use concerto_window::events::{ScrollDelta, WindowEvent};
use concerto_window::input::MouseButton;

use crate::{
    interaction::{HoveredNode, Interactable, UIDrag},
    items::UIWidgetPart,
    material::UIMaterial,
    node::{Overflow, Position, UIInset, UILayout, UINode},
    theme::UITheme,
    transform::UIValue,
};

/// A vertically scrolling area: the mouse wheel over it moves its content up and down.
///
/// The content is pinned to the area's top, left and right edges and moved up by `offset`. A list
/// or tree sets `content_extent` itself; any other owner sets it.
#[derive(Component)]
pub struct UIScrollArea {
    /// How far the content is scrolled down, in logical pixels, from 0 to the content's overflow.
    pub offset: f32,
    /// The height of the content, in logical pixels.
    pub content_extent: f32,
    /// The node scrolled, laid out with absolute position; `None` scrolls nothing.
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

/// The axis along which a [`UISplitPane`] or a splitter divides its space.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UISplitAxis {
    /// The parts sit side by side, divided by a vertical line.
    Horizontal,
    /// The parts sit one above the other, divided by a horizontal line.
    Vertical,
}

/// Divides a node between two child nodes along `axis`, sizing them from `ratio`.
///
/// The pane sets the two nodes' size, grow, shrink, overflow and visibility every frame; a
/// [`UISplitHandle`] lets the user drag the boundary.
#[derive(Component)]
pub struct UISplitPane {
    /// The axis the space is divided along.
    pub axis: UISplitAxis,
    /// The first part's share of the space left after the handles, from 0 to 1.
    pub ratio: f32,
    /// The least extent of the first part, in logical pixels.
    pub minimum_first: f32,
    /// The least extent of the second part, in logical pixels.
    pub minimum_second: f32,
    /// The part hidden, 0 for the first or 1 for the second, giving the other all the space; `None`
    /// hides neither. A collapsed pane ignores drags.
    pub collapsed: Option<usize>,
    /// The first part's node; the pane does nothing while either part is `None`.
    pub first: Option<Entity>,
    /// The second part's node.
    pub second: Option<Entity>,
}

impl UISplitPane {
    /// A pane split half and half between `first` and `second`, each at least 120 logical pixels.
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

/// Makes a node the drag handle of a [`UISplitPane`], with [`drag_split_handle`] as its drag
/// listener.
///
/// Its extent along the pane's axis is taken from the space the parts share.
#[derive(Component)]
pub struct UISplitHandle {
    /// The pane the handle resizes.
    pub pane: Entity,
}

pub(crate) fn update_scroll_areas(
    mut events: EventReader<WindowEvent>,
    hovered: Res<HoveredNode>,
    mut areas: Query<(Entity, &mut UIScrollArea, &UILayout)>,
    parents: Query<&ChildOf>,
) {
    let Some(hovered) = **hovered else { return };
    let mut delta = 0.0;
    for event in events.read() {
        if let WindowEvent::MouseWheel { delta: wheel } = event {
            delta += match wheel {
                ScrollDelta::Lines(lines) => -lines.y * 28.0,
                ScrollDelta::Pixels(pixels) => -pixels.y,
            };
        }
    }
    if delta == 0.0 {
        return;
    }
    let mut candidate = Some(hovered);
    while let Some(entity) = candidate {
        if let Some((_, mut area, layout)) = areas.get_entity_mut(entity) {
            let max = (area.content_extent - layout.content_rect.size.y).max(0.0);
            let offset = (area.offset + delta).clamp(0.0, max);
            if offset != area.offset {
                area.offset = offset;
                break;
            }
        }
        candidate = parents.get_entity(entity).map(|parent| **parent);
    }
}

pub(crate) fn sync_scroll_content(areas: Query<&UIScrollArea>, mut nodes: Query<&mut UINode>) {
    for area in areas.iter() {
        let Some(content) = area.content else {
            continue;
        };
        let Some(mut node) = nodes.get_entity_mut(content) else {
            continue;
        };
        if node.position != Position::Absolute {
            node.position = Position::Absolute;
        }
        let inset = UIInset {
            top: UIValue::Px(0.0),
            left: UIValue::Px(0.0),
            right: UIValue::Px(0.0),
            ..node.inset
        };
        if node.inset != inset {
            node.inset = inset;
        }
    }
}

/// Drag listener that moves the boundary of the pane its [`UISplitHandle`] belongs to.
pub fn drag_split_handle(
    on: On<UIDrag>,
    handles: Query<(&UISplitHandle, &UILayout)>,
    mut panes: Query<(&mut UISplitPane, &UILayout)>,
) {
    let drag = on.signal();
    if drag.button != MouseButton::Left {
        return;
    }
    let Some(pane_entity) = handles
        .get_entity(on.entity())
        .map(|(handle, _)| handle.pane)
    else {
        return;
    };
    let Some((mut pane, layout)) = panes.get_entity_mut(pane_entity) else {
        return;
    };
    if pane.collapsed.is_some() {
        return;
    }
    let available = pane.available_extent(layout, pane_entity, &handles);
    if available <= 0.0 {
        return;
    }
    let delta = match pane.axis {
        UISplitAxis::Horizontal => drag.delta.x,
        UISplitAxis::Vertical => drag.delta.y,
    };
    let first = pane.first_extent(available) + delta;
    pane.ratio = (first / available).clamp(0.0, 1.0);
}

pub(crate) fn sync_split_panes(
    panes: Query<(Entity, &UISplitPane, &UILayout)>,
    handles: Query<(&UISplitHandle, &UILayout)>,
    mut nodes: Query<&mut UINode>,
) {
    for (entity, pane, layout) in panes.iter() {
        let (Some(first), Some(second)) = (pane.first, pane.second) else {
            continue;
        };
        let available = pane.available_extent(layout, entity, &handles);
        {
            let Some(mut first_node) = nodes.get_entity_mut(first) else {
                continue;
            };
            let mut next = UINode::clone(&first_node);
            pane.apply_first_split(available, &mut next);
            first_node.set_if_neq(next);
        }
        let Some(mut second_node) = nodes.get_entity_mut(second) else {
            continue;
        };
        let mut next = UINode::clone(&second_node);
        pane.apply_second_split(&mut next);
        second_node.set_if_neq(next);
    }
}

/// Applies a split ratio to two caller-owned child nodes.
pub fn apply_split(pane: &UISplitPane, available: f32, first: &mut UINode, second: &mut UINode) {
    pane.apply_first_split(available, first);
    pane.apply_second_split(second);
}

/// Scrolls `area` the least distance that shows the span from `min` to `max`, in content
/// coordinates, in a viewport `viewport_extent` high.
///
/// When the span is taller than the viewport, its end is shown. The offset stays within the
/// content.
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

impl UISplitAxis {
    fn extent(self, size: glam::Vec2) -> f32 {
        match self {
            UISplitAxis::Horizontal => size.x,
            UISplitAxis::Vertical => size.y,
        }
    }
}

impl UISplitPane {
    fn first_extent(&self, available: f32) -> f32 {
        match self.collapsed {
            Some(0) => 0.0,
            Some(1) => available,
            _ => {
                let low = self.minimum_first.clamp(0.0, available);
                let high = (available - self.minimum_second).clamp(low, available);
                (self.ratio * available).clamp(low, high)
            }
        }
    }

    fn apply_first_split(&self, available: f32, first: &mut UINode) {
        first.visible = self.collapsed != Some(0);
        first.flex_grow = 0.0;
        first.flex_shrink = 0.0;
        first.overflow_x = Overflow::Hidden;
        first.overflow_y = Overflow::Hidden;
        let extent = UIValue::Px(self.first_extent(available));
        match self.axis {
            UISplitAxis::Horizontal => first.width = extent,
            UISplitAxis::Vertical => first.height = extent,
        }
    }

    fn apply_second_split(&self, second: &mut UINode) {
        second.visible = self.collapsed != Some(1);
        second.flex_grow = 1.0;
        second.flex_shrink = 1.0;
        second.overflow_x = Overflow::Hidden;
        second.overflow_y = Overflow::Hidden;
        match self.axis {
            UISplitAxis::Horizontal => second.width = UIValue::Auto,
            UISplitAxis::Vertical => second.height = UIValue::Auto,
        }
    }

    fn available_extent(
        &self,
        layout: &UILayout,
        pane_entity: Entity,
        handles: &Query<(&UISplitHandle, &UILayout)>,
    ) -> f32 {
        let reserved: f32 = handles
            .iter()
            .filter(|(handle, _)| handle.pane == pane_entity)
            .map(|(_, handle_layout)| self.axis.extent(handle_layout.rect.size))
            .sum();
        (self.axis.extent(layout.content_rect.size) - reserved).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::tests::Harness;

    struct PlainArea {
        area: Entity,
        content: Entity,
    }

    fn plain_area(harness: &mut Harness, offset: f32) -> PlainArea {
        let root = harness.root();
        let area = harness.node(
            UINode::default()
                .with_height(UIValue::Px(100.0))
                .with_flex_shrink(0.0),
            root,
        );
        let content = harness.node(UINode::default().with_height(UIValue::Px(900.0)), area);
        harness.world.insert(
            UIScrollArea {
                offset,
                content_extent: 900.0,
                content: Some(content),
            },
            area,
        );
        harness.settle();
        PlainArea { area, content }
    }

    #[test]
    fn scrolling_a_plain_area_moves_its_content_but_not_its_scrollbar() {
        let mut harness = Harness::new();
        let PlainArea { area, content } = plain_area(&mut harness, 0.0);
        let track = harness.track(area);
        let content_top = harness.layout(content).rect.min.y;
        let track_rect = harness.layout(track).rect;
        let track_clip = harness.layout(track).clip_rect;

        harness.set_offset(area, 140.0);
        harness.frame();

        assert_eq!(harness.layout(content).rect.min.y, content_top - 140.0);
        assert_eq!(harness.layout(track).rect, track_rect);
        assert_eq!(harness.layout(track).clip_rect, track_clip);
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
    fn the_first_pane_takes_its_ratio_of_the_space_or_all_or_none_when_collapsed() {
        let mut pane = pane(0.5);
        assert_eq!(pane.first_extent(990.0), 495.0);
        pane.collapsed = Some(0);
        assert_eq!(pane.first_extent(640.0), 0.0);
        pane.collapsed = Some(1);
        assert_eq!(pane.first_extent(640.0), 640.0);
    }

    #[test]
    fn dragging_moves_the_boundary_by_exactly_the_pointer_delta() {
        let mut pane = pane(0.5);
        let available = 800.0;
        let before = pane.first_extent(available);
        pane.ratio = ((before + 37.0) / available).clamp(0.0, 1.0);
        assert_eq!(pane.first_extent(available) - before, 37.0);
    }

    #[test]
    fn panes_clip_their_contents_so_they_cannot_paint_over_each_other() {
        let mut first = UINode::default();
        let mut second = UINode::default();
        apply_split(&pane(0.5), 600.0, &mut first, &mut second);
        assert_eq!(first.overflow_x, Overflow::Hidden);
        assert_eq!(second.overflow_y, Overflow::Hidden);
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

/// The width of a scroll area's scrollbar, drawn over the right edge of its content.
pub const SCROLLBAR_WIDTH: f32 = 8.0;
const MIN_THUMB: f32 = 24.0;

/// A scrollbar track, pinned to the right edge of its scroll area.
#[derive(Component)]
pub(crate) struct UIScrollBar {
    pub(crate) area: Entity,
}

/// The draggable part of a scrollbar.
#[derive(Component)]
pub(crate) struct UIScrollThumb {
    pub(crate) area: Entity,
}

/// Where a scrollbar thumb sits within its track, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbGeometry {
    /// The thumb's distance from the top of the track.
    pub offset: f32,
    /// The thumb's height.
    pub height: f32,
}

/// Thumb position and size for a scroll state, or `None` when the content fits and no bar should be drawn.
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
pub(crate) fn setup_scrollbars(
    new_areas: Query<(Entity, &UIScrollArea), Added<UIScrollArea>>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, _) in new_areas.iter() {
        let track = cmd
            .spawn((
                UINode::default()
                    .with_width(UIValue::Px(SCROLLBAR_WIDTH))
                    .with_position(Position::Absolute)
                    .with_visible(false),
                UIMaterial::flat(theme.canvas),
                UIScrollBar { area: entity },
                UIWidgetPart,
            ))
            .entity();
        cmd.add_child(entity, track);

        let thumb = cmd
            .spawn((
                UINode::default()
                    .with_width(UIValue::Percent(100.0))
                    .with_position(Position::Absolute),
                UIMaterial::flat(theme.border),
                Interactable,
                UIScrollThumb { area: entity },
                UIWidgetPart,
                drag_scrollbar_thumb.into_listener(),
            ))
            .entity();
        cmd.add_child(track, thumb);
    }
}

/// Pins each track to the right edge of its area and hides it when there is nothing to scroll.
pub(crate) fn sync_scrollbar_tracks(
    areas: Query<(&UIScrollArea, &UILayout)>,
    mut tracks: Query<(&UIScrollBar, &mut UINode)>,
) {
    for (bar, mut node) in tracks.iter_mut() {
        let Some((area, layout)) = areas.get_entity(bar.area) else {
            continue;
        };
        let viewport = layout.content_rect.size.y;
        let visible =
            thumb_geometry(area.offset, area.content_extent, viewport, viewport).is_some();
        if node.visible != visible {
            node.visible = visible;
        }
        let inset = UIInset {
            top: UIValue::Px(0.0),
            left: UIValue::Px((layout.content_rect.size.x - SCROLLBAR_WIDTH).max(0.0)),
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
    mut thumbs: Query<(&UIScrollThumb, &mut UINode)>,
) {
    for (thumb, mut node) in thumbs.iter_mut() {
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

/// Drags the view by dragging its thumb.
fn drag_scrollbar_thumb(
    on: On<UIDrag>,
    thumbs: Query<&UIScrollThumb>,
    mut areas: Query<(&mut UIScrollArea, &UILayout)>,
) {
    let drag = on.signal();
    if drag.button != MouseButton::Left {
        return;
    }
    let Some(thumb) = thumbs.get_entity(on.entity()) else {
        return;
    };
    let Some((mut area, layout)) = areas.get_entity_mut(thumb.area) else {
        return;
    };
    let viewport = layout.content_rect.size.y;
    let Some(geometry) = thumb_geometry(area.offset, area.content_extent, viewport, viewport)
    else {
        return;
    };
    let travel = viewport - geometry.height;
    if travel <= 0.0 {
        return;
    }
    let scrollable = area.content_extent - viewport;
    let max = scrollable.max(0.0);
    let offset = (area.offset + drag.delta.y / travel * scrollable).clamp(0.0, max);
    if offset != area.offset {
        area.offset = offset;
    }
}
