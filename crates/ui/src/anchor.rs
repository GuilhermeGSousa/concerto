//! Anchored panels: floating UI positioned against something else, painted
//! above everything, and closed when you press outside it.
#![allow(clippy::too_many_arguments)]
use std::collections::{HashMap, HashSet};

use ecs::{
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    events::{event_reader::EventReader, event_writer::EventWriter},
    query::{Query, filter::Without},
    resource::{Res, ResMut, Resource},
    system::input::SystemLocal,
};
use glam::Vec2;
use log::warn;
use window::define_action;
use window::input::{Input, InputState, MouseButton, actions::ActionFired};
use window::plugin::Window;

use crate::{
    focus::{FocusedWidget, UIFocusLost},
    node::{UIBox, UILayout, UINode},
    text_input::{UITextInput, UITextInputCancelled},
};

/// The layer anchored panels paint on, above the editor's window chrome.
pub const UI_PANEL_LAYER: i32 = 200;

/// What a panel anchors to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UIAnchorTarget {
    /// Another UI entity; the panel tracks its rect as it moves or resizes.
    Node { entity: Entity },
    /// A fixed screen point in logical pixels — a context menu at the cursor.
    Point { position: Vec2 },
}

impl UIAnchorTarget {
    /// The box `place` should anchor against, given the rects laid out this pass.
    pub fn anchor_box(&self, rects: &HashMap<Entity, UIBox>) -> Option<UIBox> {
        match self {
            Self::Point { position } => Some(UIBox {
                min: *position,
                size: Vec2::ZERO,
            }),
            Self::Node { entity } => rects.get(entity).copied(),
        }
    }
}

/// The side of the anchor the panel prefers to sit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UIAnchorSide {
    Below,
    Above,
    Right,
    Left,
}

impl UIAnchorSide {
    fn opposite(self) -> Self {
        match self {
            Self::Below => Self::Above,
            Self::Above => Self::Below,
            Self::Right => Self::Left,
            Self::Left => Self::Right,
        }
    }

    fn is_vertical(self) -> bool {
        matches!(self, Self::Below | Self::Above)
    }
}

/// How the panel lines up along the anchor's other axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UIAnchorAlign {
    Start,
    Center,
    End,
}

/// A floating panel, anchored to another node or to a point. Must be a UI root and carry [`UINode`].
#[derive(Component)]
pub struct UIAnchoredPanel {
    /// Where the panel sits.
    pub target: UIAnchorTarget,
    /// The row, button or item this panel was opened for.
    pub owner: Option<Entity>,
    pub side: UIAnchorSide,
    pub align: UIAnchorAlign,
    /// Gap between the anchor edge and the panel, in logical pixels.
    pub gap: f32,
    /// Authoritative open state. The caller sets this; the crate projects it
    /// onto `UINode::visible`.
    pub open: bool,
    /// The side `place` actually used the last time this panel was laid out, after flip.
    pub resolved_side: UIAnchorSide,
}

impl Default for UIAnchoredPanel {
    fn default() -> Self {
        Self {
            target: UIAnchorTarget::Point {
                position: Vec2::ZERO,
            },
            owner: None,
            side: UIAnchorSide::Below,
            align: UIAnchorAlign::Start,
            gap: 4.0,
            open: false,
            resolved_side: UIAnchorSide::Below,
        }
    }
}

impl UIAnchoredPanel {
    /// The entity whose rect a press is exempt from dismissal inside.
    pub fn press_exempt_entity(&self) -> Option<Entity> {
        match (self.owner, self.target) {
            (Some(owner), _) => Some(owner),
            (None, UIAnchorTarget::Node { entity }) => Some(entity),
            (None, UIAnchorTarget::Point { .. }) => None,
        }
    }
}

/// Where a panel ended up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub origin: Vec2,
    /// The side actually used, after flipping.
    pub side: UIAnchorSide,
}

/// Places `panel` against `anchor` inside `window`, in logical pixels.
pub fn place(anchor: UIBox, panel: Vec2, window: Vec2, spec: &UIAnchoredPanel) -> Placement {
    let side = if fits(anchor, panel, window, spec.side, spec.gap) {
        spec.side
    } else if fits(anchor, panel, window, spec.side.opposite(), spec.gap) {
        spec.side.opposite()
    } else {
        spec.side
    };

    let mut origin = main_axis_origin(anchor, panel, side, spec.gap);
    align_cross_axis(&mut origin, anchor, panel, side, spec.align);
    shift_cross_axis(&mut origin, panel, window, side);
    clamp(&mut origin, panel, window);

    Placement { origin, side }
}

fn fits(anchor: UIBox, panel: Vec2, window: Vec2, side: UIAnchorSide, gap: f32) -> bool {
    let max = anchor.max();
    match side {
        UIAnchorSide::Below => max.y + gap + panel.y <= window.y,
        UIAnchorSide::Above => anchor.min.y - gap - panel.y >= 0.0,
        UIAnchorSide::Right => max.x + gap + panel.x <= window.x,
        UIAnchorSide::Left => anchor.min.x - gap - panel.x >= 0.0,
    }
}

fn main_axis_origin(anchor: UIBox, panel: Vec2, side: UIAnchorSide, gap: f32) -> Vec2 {
    let max = anchor.max();
    match side {
        UIAnchorSide::Below => Vec2::new(0.0, max.y + gap),
        UIAnchorSide::Above => Vec2::new(0.0, anchor.min.y - gap - panel.y),
        UIAnchorSide::Right => Vec2::new(max.x + gap, 0.0),
        UIAnchorSide::Left => Vec2::new(anchor.min.x - gap - panel.x, 0.0),
    }
}

fn align_cross_axis(
    origin: &mut Vec2,
    anchor: UIBox,
    panel: Vec2,
    side: UIAnchorSide,
    align: UIAnchorAlign,
) {
    let (start, extent, panel_extent) = if side.is_vertical() {
        (anchor.min.x, anchor.size.x, panel.x)
    } else {
        (anchor.min.y, anchor.size.y, panel.y)
    };
    let value = match align {
        UIAnchorAlign::Start => start,
        UIAnchorAlign::Center => start + (extent - panel_extent) / 2.0,
        UIAnchorAlign::End => start + extent - panel_extent,
    };
    if side.is_vertical() {
        origin.x = value;
    } else {
        origin.y = value;
    }
}

fn shift_cross_axis(origin: &mut Vec2, panel: Vec2, window: Vec2, side: UIAnchorSide) {
    if side.is_vertical() {
        origin.x = origin.x.min(window.x - panel.x).max(0.0);
    } else {
        origin.y = origin.y.min(window.y - panel.y).max(0.0);
    }
}

fn clamp(origin: &mut Vec2, panel: Vec2, window: Vec2) {
    origin.x = origin.x.min(window.x - panel.x).max(0.0);
    origin.y = origin.y.min(window.y - panel.y).max(0.0);
}

/// One open panel's geometry, for deciding what a press dismisses.
pub struct PanelRects {
    pub panel: Entity,
    pub rect: UIBox,
    /// The owner's rect, when it has one and it has been laid out.
    pub owner_rect: Option<UIBox>,
}

/// The panels a press at `cursor` should close, given the open stack in outermost-first order.
pub fn panels_to_close(stack: &[PanelRects], cursor: Vec2) -> &[PanelRects] {
    let deepest = stack.iter().rposition(|panel| {
        panel.rect.contains(cursor) || panel.owner_rect.is_some_and(|owner| owner.contains(cursor))
    });
    match deepest {
        Some(index) => &stack[index + 1..],
        None => stack,
    }
}

/// The open panels, outermost first.
#[derive(Resource, Default)]
pub struct UIPanelStack {
    open: Vec<Entity>,
}

impl UIPanelStack {
    pub fn open(&self) -> &[Entity] {
        &self.open
    }
}

struct PanelInfo {
    entity: Entity,
    owner: Option<Entity>,
    target: UIAnchorTarget,
    open: bool,
}

/// Rebuilds the stack, closes stale chains, and projects `open` onto the node's visibility and layer.
pub fn track_panel_stack(
    panels: Query<(Entity, &mut UIAnchoredPanel, &mut UINode)>,
    nodes: Query<&UINode>,
    parents: Query<&ChildOf>,
    parented_panels: Query<(Entity, &UIAnchoredPanel, &ChildOf)>,
    nodeless_panels: Query<(Entity, &UIAnchoredPanel), Without<UINode>>,
    mut warned: SystemLocal<MisusedPanels>,
    mut stack: ResMut<UIPanelStack>,
    mut focused: ResMut<FocusedWidget>,
    mut focus_lost: EventWriter<UIFocusLost>,
) {
    for (entity, _, _) in parented_panels.iter() {
        if warned.insert(entity) {
            warn!(
                "UIAnchoredPanel on {entity:?} has a ChildOf: a panel must be a UI root, or it \
                 lays out inline in its parent's flow and is clipped by it"
            );
        }
    }
    for (entity, _) in nodeless_panels.iter() {
        if warned.insert(entity) {
            warn!(
                "UIAnchoredPanel on {entity:?} has no UINode: the panel is inert — nothing places, \
                 shows or dismisses it"
            );
        }
    }

    let mut infos: Vec<PanelInfo> = panels
        .iter()
        .map(|(entity, panel, _)| PanelInfo {
            entity,
            owner: panel.owner,
            target: panel.target,
            open: panel.open,
        })
        .collect();
    let panel_entities: Vec<Entity> = infos.iter().map(|info| info.entity).collect();

    for info in infos.iter_mut() {
        if !info.open {
            continue;
        }
        let owner_alive = match info.owner {
            Some(owner) => on_screen(owner, &nodes, &parents, &panel_entities),
            None => true,
        };
        let target_alive = match info.target {
            UIAnchorTarget::Node { entity } => on_screen(entity, &nodes, &parents, &panel_entities),
            UIAnchorTarget::Point { .. } => true,
        };
        if !owner_alive || !target_alive {
            info.open = false;
        }
    }

    loop {
        let mut orphaned = false;
        for index in 0..infos.len() {
            if !infos[index].open {
                continue;
            }
            let Some(owner) = infos[index].owner else {
                continue;
            };
            let Some(parent) = enclosing_panel(owner, &parents, &panel_entities) else {
                continue;
            };
            if !is_open(parent, &infos) {
                infos[index].open = false;
                orphaned = true;
            }
        }
        if !orphaned {
            break;
        }
    }

    let mut ordered: Vec<(usize, Entity)> = infos
        .iter()
        .filter(|info| info.open)
        .map(|info| {
            let depth = info
                .owner
                .map(|owner| depth_of(owner, &parents, &panel_entities, &infos))
                .unwrap_or(0);
            (depth, info.entity)
        })
        .collect();
    ordered.sort_by_key(|(depth, entity)| (*depth, entity.index(), entity.generation()));

    stack.open = ordered.into_iter().map(|(_, entity)| entity).collect();

    for (entity, mut panel, mut node) in panels.iter() {
        let is_panel_open = stack.open.contains(&entity);
        if panel.open && !is_panel_open {
            panel.open = false;
        }
        if node.visible != is_panel_open {
            node.visible = is_panel_open;
        }
        if is_panel_open {
            let index = stack
                .open
                .iter()
                .position(|open| *open == entity)
                .unwrap_or(0);
            let layer = UI_PANEL_LAYER + index as i32;
            if node.z_index != layer {
                node.z_index = layer;
            }
        }
    }

    if let Some(entity) = **focused
        && let Some(panel) = enclosing_panel(entity, &parents, &panel_entities)
        && !stack.open.contains(&panel)
    {
        **focused = None;
        focus_lost.write(UIFocusLost(entity));
    }
}

fn on_screen(
    entity: Entity,
    nodes: &Query<&UINode>,
    parents: &Query<&ChildOf>,
    panel_entities: &[Entity],
) -> bool {
    let mut current = Some(entity);
    while let Some(candidate) = current {
        if panel_entities.contains(&candidate) {
            return true;
        }
        if !nodes.get_entity(candidate).is_some_and(|node| node.visible) {
            return false;
        }
        current = parents
            .get_entity(candidate)
            .map(|child_of| child_of.parent());
    }
    true
}

/// The panels already reported for being built wrong, so a misuse is logged
/// once rather than every frame for as long as the panel exists.
#[derive(Default)]
pub struct MisusedPanels(HashSet<Entity>);

impl MisusedPanels {
    fn insert(&mut self, entity: Entity) -> bool {
        self.0.insert(entity)
    }
}

impl ecs::world::FromWorld for MisusedPanels {
    fn from_world(_: &ecs::world::World) -> Self {
        Self::default()
    }
}

fn is_open(entity: Entity, infos: &[PanelInfo]) -> bool {
    infos
        .iter()
        .find(|info| info.entity == entity)
        .is_some_and(|info| info.open)
}

fn enclosing_panel(
    entity: Entity,
    parents: &Query<&ChildOf>,
    panel_entities: &[Entity],
) -> Option<Entity> {
    let mut current = Some(entity);
    while let Some(candidate) = current {
        if panel_entities.contains(&candidate) {
            return Some(candidate);
        }
        current = parents
            .get_entity(candidate)
            .map(|child_of| child_of.parent());
    }
    None
}

fn depth_of(
    owner: Entity,
    parents: &Query<&ChildOf>,
    panel_entities: &[Entity],
    infos: &[PanelInfo],
) -> usize {
    let mut depth = 0;
    let mut current = Some(owner);
    while let Some(candidate) = current {
        if panel_entities.contains(&candidate) && is_open(candidate, infos) {
            depth += 1;
        }
        current = parents
            .get_entity(candidate)
            .map(|child_of| child_of.parent());
    }
    depth
}

define_action!(
    /// Close the topmost open panel.
    UIDismissPanel
);

struct EscapeDismissal {
    dismissed: bool,
    field_focused: bool,
    field_cancelled_this_frame: bool,
    stack_open: bool,
}

fn should_dismiss_on_escape(inputs: EscapeDismissal) -> bool {
    inputs.dismissed
        && !inputs.field_focused
        && !inputs.field_cancelled_this_frame
        && inputs.stack_open
}

pub(crate) fn dismiss_panels(
    stack: Res<UIPanelStack>,
    panels: Query<(&mut UIAnchoredPanel, Option<&UILayout>)>,
    layouts: Query<&UILayout>,
    input: Res<Input>,
    window: Res<Window>,
    focused: Res<FocusedWidget>,
    text_inputs: Query<&UITextInput>,
    mut actions: EventReader<ActionFired>,
    mut cancelled_this_frame: EventReader<UITextInputCancelled>,
) {
    let pressed = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::Back,
        MouseButton::Forward,
    ]
    .into_iter()
    .any(|button| input.get_mouse_button_state(button) == InputState::Pressed);

    if pressed {
        let cursor = window.logical_pointer_position(&input);
        let rects = stack
            .open()
            .iter()
            .filter_map(|entity| {
                let (panel, layout) = panels.get_entity(*entity)?;
                let layout = layout?;
                Some(PanelRects {
                    panel: *entity,
                    rect: layout.rect,
                    owner_rect: panel
                        .press_exempt_entity()
                        .and_then(|owner| layouts.get_entity(owner))
                        .map(|owner_layout| owner_layout.rect),
                })
            })
            .collect::<Vec<_>>();
        for closing in panels_to_close(&rects, cursor) {
            if let Some((mut panel, _)) = panels.get_entity(closing.panel) {
                panel.open = false;
            }
        }
    }

    let field_focused = (**focused).is_some_and(|entity| text_inputs.get_entity(entity).is_some());
    let dismissed = actions.read().any(|fired| fired.is(UIDismissPanel));
    let field_cancelled_this_frame = cancelled_this_frame.read().next().is_some();
    let should_dismiss = should_dismiss_on_escape(EscapeDismissal {
        dismissed,
        field_focused,
        field_cancelled_this_frame,
        stack_open: !stack.open().is_empty(),
    });
    if should_dismiss
        && let Some(top) = stack.open().last()
        && let Some((mut panel, _)) = panels.get_entity(*top)
    {
        panel.open = false;
    }
}

#[cfg(test)]
mod dismiss_tests {
    use super::{EscapeDismissal, should_dismiss_on_escape};

    #[test]
    fn does_nothing_without_a_press() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: false,
            field_focused: false,
            field_cancelled_this_frame: false,
            stack_open: true,
        }));
    }

    #[test]
    fn a_focused_text_field_keeps_first_claim_on_escape() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: true,
            field_cancelled_this_frame: false,
            stack_open: true,
        }));
    }

    #[test]
    fn nothing_to_close_is_a_no_op() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: false,
            field_cancelled_this_frame: false,
            stack_open: false,
        }));
    }

    #[test]
    fn a_focused_field_with_nothing_open_is_still_a_no_op() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: true,
            field_cancelled_this_frame: false,
            stack_open: false,
        }));
    }

    #[test]
    fn closes_the_top_panel_otherwise() {
        assert!(should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: false,
            field_cancelled_this_frame: false,
            stack_open: true,
        }));
    }

    #[test]
    fn frame_n_a_same_frame_cancel_does_not_also_close_the_panel() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: false,
            field_cancelled_this_frame: true,
            stack_open: true,
        }));
    }

    #[test]
    fn frame_n_plus_1_the_following_escape_closes_the_panel() {
        assert!(should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            field_focused: false,
            field_cancelled_this_frame: false,
            stack_open: true,
        }));
    }
}
