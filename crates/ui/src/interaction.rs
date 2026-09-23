#![allow(clippy::too_many_arguments)]

use color::Color;
use derive_more::{Deref, DerefMut};
use ecs::events::event_writer::EventWriter;
use ecs::{
    component::Component,
    entity::Entity,
    events::Event,
    query::{Query, filter::Without},
    resource::{Res, ResMut, Resource},
};
use glam::Vec2;
use window::input::{Input, InputState, MouseButton};

use crate::{material::UIMaterial, node::UILayout};

/// The UI entity currently under the cursor, if any.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct HoveredNode(Option<Entity>);

/// Shared pointer routing state. Captured widgets continue receiving drag and
/// release events after the pointer leaves their bounds.
#[derive(Resource, Default)]
pub struct UIInputState {
    pub hovered: Option<Entity>,
    left: ButtonCapture,
    right: ButtonCapture,
    middle: ButtonCapture,
}

impl UIInputState {
    fn capture_mut(&mut self, button: MouseButton) -> Option<&mut ButtonCapture> {
        match button {
            MouseButton::Left => Some(&mut self.left),
            MouseButton::Right => Some(&mut self.right),
            MouseButton::Middle => Some(&mut self.middle),
            // Back, Forward and Other route nothing: no widget asks for them,
            // and a capture slot per possible button would be a map.
            _ => None,
        }
    }

    fn capture(&self, button: MouseButton) -> Option<&ButtonCapture> {
        match button {
            MouseButton::Left => Some(&self.left),
            MouseButton::Right => Some(&self.right),
            MouseButton::Middle => Some(&self.middle),
            _ => None,
        }
    }

    /// The node this button pressed on, if it is still down.
    pub fn pressed(&self, button: MouseButton) -> Option<Entity> {
        self.capture(button).and_then(|capture| capture.pressed)
    }

    /// The node this button is routing events to, wherever the pointer is.
    pub fn captured(&self, button: MouseButton) -> Option<Entity> {
        self.capture(button).and_then(|capture| capture.captured)
    }

    /// Test-only escape hatch: forces a button's capture without going
    /// through `advance_capture`.
    ///
    /// Downstream crates' tests (e.g. `editor::workspace`) need to put a
    /// widget "mid-capture" to exercise focus/capture handling around
    /// unrelated state resets, without wiring up a fake press-and-hold
    /// through `Input`. `captured` used to be a public field they could
    /// assign directly; this is the narrow equivalent now that capture
    /// lives per button behind private `ButtonCapture` slots. `#[doc(hidden)]`
    /// because it is a test seam, not part of the intended production API —
    /// `pub(crate)` isn't enough since it must be reachable from other
    /// crates' test modules.
    #[doc(hidden)]
    pub fn set_captured(&mut self, button: MouseButton, entity: Option<Entity>) {
        if let Some(capture) = self.capture_mut(button) {
            capture.captured = entity;
        }
    }
}

/// Press-and-capture state for a single mouse button.
///
/// Per button rather than shared: a right-click that opens a menu must not
/// cancel a left-button drag that is still in progress.
#[derive(Default)]
struct ButtonCapture {
    pressed: Option<Entity>,
    captured: Option<Entity>,
    press_origin: Option<Vec2>,
    last_cursor: Option<Vec2>,
    dragged: bool,
}

/// What one button's transition produced this frame.
struct CaptureOutcome {
    down: Option<Entity>,
    up: Option<Entity>,
    click: Option<Entity>,
    drag: Option<Entity>,
    delta: Vec2,
}

impl CaptureOutcome {
    fn none() -> Self {
        Self {
            down: None,
            up: None,
            click: None,
            drag: None,
            delta: Vec2::ZERO,
        }
    }
}

/// How far the pointer may travel between press and release and still count
/// as a click rather than a drag, in logical pixels.
const DRAG_THRESHOLD: f32 = 4.0;

/// Advances one button's capture by a frame and reports what it produced.
///
/// Split out of the system because the system needs `Res<Window>` to find the
/// cursor and therefore cannot be run in a test, while every rule worth
/// getting right lives here.
fn advance_capture(
    capture: &mut ButtonCapture,
    state: InputState,
    hit: Option<Entity>,
    cursor: Vec2,
) -> CaptureOutcome {
    let mut outcome = CaptureOutcome::none();
    match state {
        InputState::Pressed => {
            capture.pressed = hit;
            capture.captured = hit;
            capture.press_origin = hit.map(|_| cursor);
            capture.last_cursor = Some(cursor);
            capture.dragged = false;
            outcome.down = hit;
        }
        InputState::Down => {
            if let Some(entity) = capture.captured {
                capture.dragged |= capture
                    .press_origin
                    .is_some_and(|origin| origin.distance(cursor) >= DRAG_THRESHOLD);
                outcome.delta = cursor - capture.last_cursor.unwrap_or(cursor);
                outcome.drag = Some(entity);
            }
            capture.last_cursor = Some(cursor);
        }
        InputState::Released => {
            if let Some(entity) = capture.captured {
                outcome.up = Some(entity);
                if capture.pressed == hit && !capture.dragged {
                    outcome.click = Some(entity);
                }
            }
            capture.pressed = None;
            capture.captured = None;
            capture.press_origin = None;
            capture.last_cursor = Some(cursor);
            capture.dragged = false;
        }
        InputState::Up => {
            capture.last_cursor = Some(cursor);
        }
    }
    outcome
}

/// Opts a node into hit testing and click events.
///
/// Add this to any UI entity that should receive [`UIClick`] events or
/// contribute to [`HoveredNode`].  Deliberately separate from
/// [`UIInteractionStyle`] so that interactability and visual feedback are
/// independent: a node can be clickable without changing colour, and a node
/// can show hover colours without being a click target.
#[derive(Component)]
pub struct Interactable;

/// Marks a node as non-interactive.  When present, `apply_interaction_styles`
/// uses `UIInteractionStyle::disabled` regardless of cursor position.
#[derive(Component)]
pub struct UIDisabled;

/// Per-node colour palette for the four interaction states.
///
/// Attach this alongside [`UIMaterial`] to get automatic hover/press colour
/// changes.  The system `apply_interaction_styles` writes the correct colour
/// into `UIMaterial::color` each frame based on the current cursor position and
/// left-button state.
///
/// # Example
/// ```rust,ignore
/// (
///     UINode::default(),
///     UIMaterial::flat(Color::rgba(0.2, 0.2, 0.2, 1.0)),
///     UIInteractionStyle {
///         normal:   Color::rgba(0.20, 0.20, 0.20, 1.0),
///         hovered:  Color::rgba(0.28, 0.28, 0.28, 1.0),
///         pressed:  Color::rgba(0.14, 0.14, 0.14, 1.0),
///         disabled: Color::rgba(0.10, 0.10, 0.10, 0.5),
///     },
/// )
/// ```
#[derive(Component, Clone)]
pub struct UIInteractionStyle {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
    pub disabled: Color,
}

/// Fired when a button is released over the same node it pressed.
#[derive(Event)]
pub struct UIClick {
    pub entity: Entity,
    pub position: Vec2,
    pub button: MouseButton,
}

#[derive(Event)]
pub struct UIPointerDown {
    pub entity: Entity,
    pub position: Vec2,
    pub button: MouseButton,
}

#[derive(Event)]
pub struct UIPointerUp {
    pub entity: Entity,
    pub position: Vec2,
    pub button: MouseButton,
}

#[derive(Event)]
pub struct UIDrag {
    pub entity: Entity,
    pub position: Vec2,
    pub delta: Vec2,
    pub button: MouseButton,
}

#[derive(Event)]
pub struct UIPointerEnter {
    pub entity: Entity,
    pub position: Vec2,
}

#[derive(Event)]
pub struct UIPointerLeave {
    pub entity: Entity,
    pub position: Vec2,
}

/// Walks all [`UILayout`]s each frame, determines which one (if any) is
/// under the cursor, updates [`HoveredNode`], and fires [`UIClick`] events on
/// left-button interaction events.
///
/// Runs in `LateUpdate`, after `compute_ui_nodes` has populated
/// [`UILayout`] for the current frame.
pub(crate) fn update_ui_interaction(
    computed_nodes: Query<(Entity, &UILayout, &Interactable), Without<UIDisabled>>,
    input: Res<Input>,
    window: Res<window::plugin::Window>,
    mut hovered: ResMut<HoveredNode>,
    mut state: ResMut<UIInputState>,
    mut click_writer: EventWriter<UIClick>,
    mut down_writer: EventWriter<UIPointerDown>,
    mut up_writer: EventWriter<UIPointerUp>,
    mut drag_writer: EventWriter<UIDrag>,
    mut enter_writer: EventWriter<UIPointerEnter>,
    mut leave_writer: EventWriter<UIPointerLeave>,
) {
    let cursor = window.logical_pointer_position(&input);

    // Pick the node highest in the Z-order that contains the cursor.
    let mut best: Option<(Entity, i64)> = None;
    for (entity, node, _) in computed_nodes.iter() {
        if node.rect.contains(cursor)
            && node.clip_rect.contains(cursor)
            && best.is_none_or(|(_, z)| node.paint_order > z)
        {
            best = Some((entity, node.paint_order));
        }
    }

    let hit = best.map(|(entity, _)| entity);
    if state.hovered != hit {
        if let Some(entity) = state.hovered {
            leave_writer.write(UIPointerLeave {
                entity,
                position: cursor,
            });
        }
        if let Some(entity) = hit {
            enter_writer.write(UIPointerEnter {
                entity,
                position: cursor,
            });
        }
    }
    **hovered = hit;
    state.hovered = hit;

    // Left first, so a widget that reacts to both buttons sees the primary
    // one in the order a reader expects.
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        let input_state = input.get_mouse_button_state(button);
        let Some(capture) = state.capture_mut(button) else {
            continue;
        };
        let outcome = advance_capture(capture, input_state, hit, cursor);
        if let Some(entity) = outcome.down {
            down_writer.write(UIPointerDown {
                entity,
                position: cursor,
                button,
            });
        }
        if let Some(entity) = outcome.up {
            up_writer.write(UIPointerUp {
                entity,
                position: cursor,
                button,
            });
        }
        if let Some(entity) = outcome.click {
            click_writer.write(UIClick {
                entity,
                position: cursor,
                button,
            });
        }
        if let Some(entity) = outcome.drag {
            drag_writer.write(UIDrag {
                entity,
                position: cursor,
                delta: outcome.delta,
                button,
            });
        }
    }
}

/// Drives [`UIMaterial::color`] from [`UIInteractionStyle`] each frame.
///
/// For each entity that has both components:
/// - If it has [`UIDisabled`], use `style.disabled`.
/// - Else if it is the currently hovered node and the left button is held,
///   use `style.pressed`.
/// - Else if it is hovered, use `style.hovered`.
/// - Otherwise use `style.normal`.
pub(crate) fn apply_interaction_styles(
    hovered: Res<HoveredNode>,
    input: Res<Input>,
    disabled_nodes: Query<(Entity, &UIDisabled)>,
    styled: Query<(Entity, &UIInteractionStyle, &mut UIMaterial)>,
) {
    let left_held = matches!(
        input.get_mouse_button_state(MouseButton::Left),
        InputState::Pressed | InputState::Down
    );

    for (entity, style, mut material) in styled.iter() {
        let color = if disabled_nodes.get_entity(entity).is_some() {
            style.disabled
        } else if **hovered == Some(entity) {
            if left_held {
                style.pressed
            } else {
                style.hovered
            }
        } else {
            style.normal
        };
        material.color = color.to_linear();
    }
}

#[cfg(test)]
mod tests {
    use crate::node::{UIBox, UILayout};
    use glam::Vec2;

    use super::{ButtonCapture, Interactable, UIInputState, advance_capture};
    use ecs::{World, entity::Entity};
    use window::input::{InputState, MouseButton};

    /// Entities have no public raw constructor — generations are `NonZero` —
    /// so a throwaway world mints the distinct ids these tests compare.
    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::default();
        (0..count).map(|_| world.spawn(Interactable)).collect()
    }

    #[test]
    fn a_press_and_release_on_the_same_node_is_a_click() {
        let mut capture = ButtonCapture::default();
        let node = Some(entities(1)[0]);

        let down = advance_capture(&mut capture, InputState::Pressed, node, Vec2::ZERO);
        assert_eq!(down.down, node);
        assert_eq!(down.click, None);

        let up = advance_capture(&mut capture, InputState::Released, node, Vec2::ZERO);
        assert_eq!(up.up, node);
        assert_eq!(up.click, node, "press and release on one node is a click");
    }

    #[test]
    fn releasing_over_a_different_node_is_not_a_click() {
        let ids = entities(2);
        let mut capture = ButtonCapture::default();
        advance_capture(&mut capture, InputState::Pressed, Some(ids[0]), Vec2::ZERO);

        let up = advance_capture(&mut capture, InputState::Released, Some(ids[1]), Vec2::ZERO);
        assert_eq!(up.up, Some(ids[0]), "the captured node still gets the up");
        assert_eq!(up.click, None, "but sliding off it cancels the click");
    }

    #[test]
    fn a_drag_past_the_threshold_cancels_the_click() {
        let mut capture = ButtonCapture::default();
        let node = Some(entities(1)[0]);
        advance_capture(&mut capture, InputState::Pressed, node, Vec2::ZERO);

        let held = advance_capture(&mut capture, InputState::Down, node, Vec2::new(40.0, 0.0));
        assert_eq!(held.drag, node);
        assert_eq!(
            held.delta,
            Vec2::new(40.0, 0.0),
            "the first drag reports movement since the press"
        );

        let up = advance_capture(
            &mut capture,
            InputState::Released,
            node,
            Vec2::new(40.0, 0.0),
        );
        assert_eq!(up.click, None, "a drag is not a click");
    }

    #[test]
    fn a_captured_node_keeps_receiving_drags_after_the_pointer_leaves_it() {
        let handle = entities(1)[0];
        let mut capture = ButtonCapture::default();
        advance_capture(&mut capture, InputState::Pressed, Some(handle), Vec2::ZERO);

        let held = advance_capture(&mut capture, InputState::Down, None, Vec2::new(5.0, 9.0));
        assert_eq!(
            held.drag,
            Some(handle),
            "a split handle dragged off its own bounds must keep the pointer"
        );
    }

    #[test]
    fn a_press_on_empty_space_captures_nothing() {
        let mut capture = ButtonCapture::default();
        let down = advance_capture(&mut capture, InputState::Pressed, None, Vec2::ZERO);
        assert_eq!(down.down, None);

        let up = advance_capture(&mut capture, InputState::Released, None, Vec2::ZERO);
        assert_eq!(up.up, None);
        assert_eq!(up.click, None);
    }

    #[test]
    fn each_button_captures_independently() {
        // A right-click opening a context menu must not cancel a left-button
        // drag that is still in progress.
        let ids = entities(2);
        let mut state = UIInputState::default();
        let handle = Some(ids[0]);
        let row = Some(ids[1]);

        advance_capture(&mut state.left, InputState::Pressed, handle, Vec2::ZERO);
        let right = advance_capture(&mut state.right, InputState::Pressed, row, Vec2::ZERO);

        assert_eq!(right.down, row);
        assert_eq!(
            state.captured(MouseButton::Left),
            handle,
            "the left button must still own the node it grabbed"
        );

        let left = advance_capture(
            &mut state.left,
            InputState::Down,
            handle,
            Vec2::new(9.0, 0.0),
        );
        assert_eq!(left.drag, handle, "and must keep receiving its drags");
    }

    #[test]
    fn clip_rect_excludes_visually_clipped_area() {
        let layout = UILayout {
            rect: UIBox {
                min: Vec2::ZERO,
                size: Vec2::splat(100.0),
            },
            content_rect: UIBox::default(),
            clip_rect: UIBox {
                min: Vec2::ZERO,
                size: Vec2::splat(50.0),
            },
            paint_order: 0,
        };
        assert!(layout.rect.contains(Vec2::new(75.0, 25.0)));
        assert!(!layout.clip_rect.contains(Vec2::new(75.0, 25.0)));
    }
}
