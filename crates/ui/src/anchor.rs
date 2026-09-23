//! Anchored panels: floating UI that is positioned against something else,
//! paints above everything, and closes when you press outside it.
//!
//! The primitive owns placement, layering and dismissal. What a panel
//! contains — rows, icons, separators, what an item does — is the caller's,
//! built from ordinary UI children.
// A system function's parameter list is its data dependencies, one per
// resource or query; splitting it up would hide those dependencies rather
// than reduce them. Same convention as node.rs and interaction.rs.
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
/// Nested panels take `UI_PANEL_LAYER + depth`.
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
    /// The box `place` should anchor against, given the rects laid out this
    /// pass.
    ///
    /// A point is a degenerate box: placement only ever reads an anchor's
    /// edges, so a cursor is simply a box with no extent. `None` means the
    /// target was not laid out — hidden, or gone — and the caller should
    /// leave the panel where it is rather than placing it against nothing.
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

/// A floating panel, anchored to another node or to a point.
///
/// The entity carrying this is a UI **root** — it has no `ChildOf` — and it
/// must also carry [`UINode`]. Being a root is what lets it escape the clip
/// rect of whatever it was opened from; a menu that cannot paint outside its
/// scroll area is not a menu. Neither requirement is enforced by the type
/// system, so both are checked at runtime and reported once per entity: a
/// panel with a `ChildOf` lays out inline in its parent's flow, clipped, and
/// one with no `UINode` is inert.
///
/// The stack is ordered — and nesting derived — from the **owner** chain,
/// while placement anchors to `target`. A panel whose `target` points inside
/// another panel but whose `owner` does not reads as top-level, so it may be
/// placed before the panel its anchor lives in has a rect, and is then left
/// unplaced for that pass.
#[derive(Component)]
pub struct UIAnchoredPanel {
    /// Where the panel sits.
    pub target: UIAnchorTarget,
    /// What the panel is about: the row, button or item it was opened for.
    ///
    /// Separate from `target` because a context menu is placed at the cursor
    /// but belongs to the row under it, and one field cannot say both. This
    /// is what nesting, the press exemption and auto-close all key off, and
    /// how a caller identifies the subject its items act on.
    pub owner: Option<Entity>,
    pub side: UIAnchorSide,
    pub align: UIAnchorAlign,
    /// Gap between the anchor edge and the panel, in logical pixels.
    pub gap: f32,
    /// Authoritative open state. The caller sets this; the crate projects it
    /// onto `UINode::visible`.
    pub open: bool,
    /// The side `place` actually used the last time this panel was laid out,
    /// after flip. Crate-written output: the layout sweep overwrites this
    /// every pass, so a caller reads it (to draw a pointer arrow, or to show
    /// the flip as text) but never sets it — a caller's own value would just
    /// be discarded on the next pass.
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
    ///
    /// `owner` when the caller set one. Otherwise a `Node` target stands in
    /// for it, because the obvious way to write a dropdown —
    /// `target: Node { entity: trigger }` and `..Default::default()`, whose
    /// `owner` is `None` — would otherwise have no exemption at all: a second
    /// press on the trigger would read as an outside press, close the panel,
    /// and the caller's own toggle would reopen it a frame later, so the menu
    /// could never be closed by its own button.
    ///
    /// A `Point` target with no owner is a standalone overlay and stays
    /// exempt from nothing: it has no second rect to fall back on.
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
    /// The side actually used, after flipping. Callers that draw a pointer
    /// arrow need to know which way the panel ended up facing.
    pub side: UIAnchorSide,
}

/// Places `panel` against `anchor` inside `window`, in logical pixels.
///
/// Flip, then shift, then clamp. Flipping first keeps the panel attached to
/// the thing it belongs to; shifting only slides it along the anchor's own
/// edge, which never covers the anchor; clamping is the last resort for a
/// panel too big to fit at all, and pins its top-left rather than its middle
/// so the first item is the one you can see.
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

/// Whether the panel's main-axis extent stays inside the window on `side`.
fn fits(anchor: UIBox, panel: Vec2, window: Vec2, side: UIAnchorSide, gap: f32) -> bool {
    let max = anchor.max();
    match side {
        UIAnchorSide::Below => max.y + gap + panel.y <= window.y,
        UIAnchorSide::Above => anchor.min.y - gap - panel.y >= 0.0,
        UIAnchorSide::Right => max.x + gap + panel.x <= window.x,
        UIAnchorSide::Left => anchor.min.x - gap - panel.x >= 0.0,
    }
}

/// The panel's position on the axis it is offset along.
fn main_axis_origin(anchor: UIBox, panel: Vec2, side: UIAnchorSide, gap: f32) -> Vec2 {
    let max = anchor.max();
    match side {
        UIAnchorSide::Below => Vec2::new(0.0, max.y + gap),
        UIAnchorSide::Above => Vec2::new(0.0, anchor.min.y - gap - panel.y),
        UIAnchorSide::Right => Vec2::new(max.x + gap, 0.0),
        UIAnchorSide::Left => Vec2::new(anchor.min.x - gap - panel.x, 0.0),
    }
}

/// Fills in the axis the panel is aligned along.
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

/// Slides the panel back inside the window along the axis it is aligned on.
fn shift_cross_axis(origin: &mut Vec2, panel: Vec2, window: Vec2, side: UIAnchorSide) {
    if side.is_vertical() {
        origin.x = origin.x.min(window.x - panel.x).max(0.0);
    } else {
        origin.y = origin.y.min(window.y - panel.y).max(0.0);
    }
}

/// Last resort for a panel that does not fit however it is placed.
fn clamp(origin: &mut Vec2, panel: Vec2, window: Vec2) {
    origin.x = origin.x.min(window.x - panel.x).max(0.0);
    origin.y = origin.y.min(window.y - panel.y).max(0.0);
}

/// One open panel's geometry, for deciding what a press dismisses.
pub struct PanelRects {
    pub panel: Entity,
    pub rect: UIBox,
    /// The owner's rect, when it has one and it has been laid out. A press
    /// here counts as a press inside the panel.
    pub owner_rect: Option<UIBox>,
}

/// The panels a press at `cursor` should close, given the open stack in
/// outermost-first order.
///
/// Returns a suffix: closing a panel always closes everything opened from it.
/// A press inside a panel — or on the owner that opened it — closes nothing at
/// that level and everything below.
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
///
/// Depth comes from the owner chain, not from a field: a panel whose owner
/// lives inside another open panel is that panel's child. A submenu therefore
/// needs no wiring beyond naming the row that opened it.
#[derive(Resource, Default)]
pub struct UIPanelStack {
    open: Vec<Entity>,
}

impl UIPanelStack {
    pub fn open(&self) -> &[Entity] {
        &self.open
    }
}

/// One panel's identity, snapshotted once so the rest of the system can walk
/// it repeatedly without re-borrowing `panels` — whose items hold a mutable
/// `UINode` reference — mid-iteration.
struct PanelInfo {
    entity: Entity,
    owner: Option<Entity>,
    target: UIAnchorTarget,
    open: bool,
}

/// Rebuilds the stack, closes stale chains, and projects `open` onto the
/// node's visibility and layer.
///
/// Runs before the layout pass, which reads the stack to decide the order
/// anchored roots are placed in.
///
/// A panel closed here by staleness or orphaning gets `UIAnchoredPanel::open`
/// latched to `false`, not just its visibility: otherwise a hidden owner that
/// reappears un-hides a panel the user never reopened, a toggle callback sees
/// a stale `true` and needs two clicks, and `UIPanelStack` — which only lists
/// open panels — has nothing left to tell `dismiss_panels` to clear.
pub fn track_panel_stack(
    panels: Query<(Entity, &mut UIAnchoredPanel, &mut UINode)>,
    nodes: Query<&UINode>,
    parents: Query<&ChildOf>,
    // Both misuses a panel can be built with, and neither fails loudly on its
    // own: a panel with a parent lays out inline in that parent's flow,
    // clipped; a panel with no `UINode` never reaches `panels` at all.
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

    // An owner or target that is gone or hidden means the thing the panel is
    // about is gone. Absence of UILayout deliberately does not count: that is
    // the normal state of an entity spawned this frame, not staleness.
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

    // A panel nests under whichever panel encloses its owner, whether or not
    // that enclosing panel is currently open — a closed parent must still be
    // found so its children close too. This repeats to a fixed point because
    // closing a middle panel can orphan a grandchild only discovered once its
    // immediate parent has already been marked closed.
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

    // Depth is how many open panels enclose an owner. Outermost-first keeps
    // Task 7's placement order sane: a submenu must be placed after the panel
    // it is anchored to.
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
    // Entity index as the tie-break, so two panels at the same depth keep a
    // stable order between frames rather than following archetype order.
    ordered.sort_by_key(|(depth, entity)| (*depth, entity.index(), entity.generation()));

    stack.open = ordered.into_iter().map(|(_, entity)| entity).collect();

    // Layer is stack index, not owner-depth: it only needs to be strictly
    // monotonic in placement order for painting to stack correctly, and two
    // top-level siblings landing on adjacent layers is harmless.
    for (entity, mut panel, mut node) in panels.iter() {
        let is_panel_open = stack.open.contains(&entity);
        // Latch the closure back onto the authoritative field: staleness and
        // orphaning are decisions this system makes, not transient render
        // state, so they must survive to the next frame and to whatever else
        // reads `open` (a toggle callback, `dismiss_panels`).
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

    // Focus cannot be left sitting in a panel that is no longer on screen.
    // `update_focus` only reassigns on a left press, so until the next one a
    // hidden text field keeps `ActionMap` in text capture — swallowing every
    // bare-letter shortcut — and keeps `dismiss_panels` deferring Escape to a
    // field nobody can see. The owner walk above already gives the answer:
    // whichever panel encloses the focused entity must still be open.
    if let Some(entity) = **focused
        && let Some(panel) = enclosing_panel(entity, &parents, &panel_entities)
        && !stack.open.contains(&panel)
    {
        **focused = None;
        focus_lost.write(UIFocusLost(entity));
    }
}

/// Whether `entity` is somewhere a user can actually see: it still exists, it
/// is visible, and so is every ancestor up to the panel or root it lives in.
///
/// Ancestor-aware on purpose. The layout pass refuses to record an anchor rect
/// for anything under a hidden ancestor (`write_absolute_positions` in
/// `node.rs`), so a panel judged fresh by its trigger's own `visible` flag
/// while that trigger sits inside a collapsed section would find no rect to
/// anchor to, keep its last `UILayout`, and hang over the screen at a stale
/// position with the thing it belongs to nowhere in sight.
///
/// The walk stops at a panel, whose `visible` this system writes itself from
/// `open` further down: a submenu's row is inside its parent menu, which is
/// still `visible: false` at this point on the frame it opens. Whether that
/// parent is going away is the orphan pass's question, not this one's.
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
        // A missing `UINode` anywhere in the chain is as good as hidden: the
        // layout pass walks `UINode` roots and their children, so a subtree
        // hanging off something it does not recognise is never laid out.
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
    /// Whether this is the first report for `entity`.
    fn insert(&mut self, entity: Entity) -> bool {
        self.0.insert(entity)
    }
}

impl ecs::world::FromWorld for MisusedPanels {
    fn from_world(_: &ecs::world::World) -> Self {
        Self::default()
    }
}

/// Whether `entity`'s panel is currently open, per the frame-local snapshot.
fn is_open(entity: Entity, infos: &[PanelInfo]) -> bool {
    infos
        .iter()
        .find(|info| info.entity == entity)
        .is_some_and(|info| info.open)
}

/// The panel `entity` sits inside, if any — the nearest ancestor (including
/// itself) that is itself a panel entity, open or closed. Whether that panel
/// is currently open is the caller's decision to make; this only answers
/// which panel it is.
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

/// How many open panels enclose `owner`.
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

/// Whether a fired [`UIDismissPanel`] should close the topmost panel right
/// now.
///
/// Pulled out of `dismiss_panels` because it is the one genuinely ambiguous
/// call this system makes: a focused text field reads Escape first, to
/// cancel editing, so the panel only gets to react once the field has
/// declined and there is actually something open to close.
///
/// The inputs `should_dismiss_on_escape` decides over — named so a call site
/// cannot transpose two `bool`s that would still compile and silently invert
/// the decision.
struct EscapeDismissal {
    /// `UIDismissPanel` fired this frame.
    dismissed: bool,
    /// A `UITextInput` holds focus, and therefore first claim on Escape.
    field_focused: bool,
    /// A `UITextInputCancelled` was written this frame: a field just spent
    /// *this* Escape cancelling, in the same frame it also gave up focus.
    /// Without this, clearing focus on cancel (so a *second* Escape can
    /// reach the panel) makes the *first* Escape look field-free by the
    /// time `dismiss_panels` reads `FocusedWidget` — `update_text_inputs`
    /// runs first and has already cleared it — so the same keypress would
    /// cancel the field and close the panel together. Treating a same-frame
    /// cancel as equivalent to a focused field closes that gap: frame N
    /// cancels and is suppressed, frame N+1 finds no focus and no cancel
    /// event, and closes the panel.
    field_cancelled_this_frame: bool,
    /// There is at least one open panel to close.
    stack_open: bool,
}

fn should_dismiss_on_escape(inputs: EscapeDismissal) -> bool {
    inputs.dismissed
        && !inputs.field_focused
        && !inputs.field_cancelled_this_frame
        && inputs.stack_open
}

/// Closes panels on an outside press or on Escape.
///
/// Runs after `update_text_inputs`: a focused text field reads Escape
/// directly to cancel editing, so the field gets the first Escape and the
/// panel gets the second.
pub(crate) fn dismiss_panels(
    stack: Res<UIPanelStack>,
    // `UILayout` is `Option`: a panel spawned this frame has no layout yet,
    // and Escape has no geometric need for one. Requiring it outright would
    // make such a panel silently swallow Escape — the predicate says
    // dismiss, `get_entity` returns `None`, and nothing closes.
    panels: Query<(&mut UIAnchoredPanel, Option<&UILayout>)>,
    layouts: Query<&UILayout>,
    input: Res<Input>,
    window: Res<Window>,
    focused: Res<FocusedWidget>,
    text_inputs: Query<&UITextInput>,
    mut actions: EventReader<ActionFired>,
    // Each system reading an event type keeps its own cursor, so reading
    // this here does not consume it out from under the editor's
    // `cancel_numeric_fields`, which also reads it to revert the field.
    mut cancelled_this_frame: EventReader<UITextInputCancelled>,
) {
    // Any button, not just the left one: a thumb-button press outside an
    // open menu should close it just as a left-click does. `MouseButton`
    // also has an `Other(u16)` variant for vendor-specific buttons that
    // `Input` does not expose a way to enumerate; those are out of scope.
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
        // Snapshot rects first, then apply closures in a second pass: two
        // sequential lookups into `panels` per entity (one here, one below)
        // never overlap, but folding both into a single borrow-holding loop
        // would.
        //
        // An open panel with no `UILayout` yet is dropped from this list
        // entirely, not treated as "everywhere" or "nowhere" — so with
        // stack `[A, B, C]` and B unlaid-out, a press resolves against A and
        // C only: it can still close C while leaving B (and therefore A)
        // untouched. Rare after panels lay out even while closed, but the
        // suffix this produces depends on it.
        let rects = stack
            .open()
            .iter()
            .filter_map(|entity| {
                let (panel, layout) = panels.get_entity(*entity)?;
                let layout = layout?;
                Some(PanelRects {
                    panel: *entity,
                    rect: layout.rect,
                    // Feeds the owner exemption in `panels_to_close`: without
                    // it, a second click on an open menu's trigger reads as
                    // an outside press, closes the panel, and the caller's
                    // own toggle immediately reopens it. A panel that named
                    // no owner falls back to its `Node` target, which for a
                    // dropdown is the same trigger.
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

    // A focused text field owns Escape; it cancels editing with it, so the
    // panel only reacts once the field has had first claim.
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

    // `should_dismiss_on_escape` is a plain four-input AND, so these are not
    // an exhaustive truth table over all 16 combinations — they're the
    // cases that matter: a plain press, each of the two ways a field can
    // claim the keypress, an empty stack, and (below) the specific
    // same-frame-vs-next-frame sequence this predicate exists to get right.
    // The named fields mean a transposed pair of `bool`s (the failure mode
    // this struct exists to rule out) fails to compile rather than passing
    // silently.
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

    // The two-frame sequence `dismiss_panels` relies on: `update_text_inputs`
    // clears focus the same frame a field cancels, which would otherwise
    // make that Escape look field-free by the time this predicate runs. The
    // cancel-this-frame flag is what tells frame N apart from frame N+1.
    #[test]
    fn frame_n_a_same_frame_cancel_does_not_also_close_the_panel() {
        assert!(!should_dismiss_on_escape(EscapeDismissal {
            dismissed: true,
            // Already cleared by update_text_inputs, which runs first.
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
            // No cancel event this frame: the field already gave up focus
            // and editing last frame.
            field_cancelled_this_frame: false,
            stack_open: true,
        }));
    }
}
