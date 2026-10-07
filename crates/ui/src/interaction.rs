use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::Entity,
    query::{Query, filter::Without},
    resource::{Res, ResMut, Resource},
    signal::{EntitySignal, Signal},
};
use concerto_foundation::time::Instant;
use concerto_window::input::{Input, InputState, MouseButton};
use derive_more::{Deref, DerefMut};
use glam::Vec2;
use std::time::Duration;

use crate::{material::UIMaterial, node::UILayout};

/// The UI entity currently under the cursor, if any.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct HoveredNode(Option<Entity>);

/// Shared pointer routing state.
#[derive(Resource, Default)]
pub struct UIInputState {
    pub(crate) hovered: Option<Entity>,
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
    pub fn hovered(&self) -> Option<Entity> {
        self.hovered
    }

    pub fn pressed(&self, button: MouseButton) -> Option<Entity> {
        self.capture(button).and_then(|capture| capture.pressed)
    }

    /// The node this button is routing events to, wherever the pointer is.
    pub fn captured(&self, button: MouseButton) -> Option<Entity> {
        self.capture(button).and_then(|capture| capture.captured)
    }

    /// Test-only escape hatch: forces a button's capture without going through `advance_capture`.
    #[doc(hidden)]
    pub fn set_captured(&mut self, button: MouseButton, entity: Option<Entity>) {
        if let Some(capture) = self.capture_mut(button) {
            capture.captured = entity;
        }
    }
}

#[derive(Default)]
struct ButtonCapture {
    pressed: Option<Entity>,
    captured: Option<Entity>,
    press_origin: Option<Vec2>,
    last_cursor: Option<Vec2>,
    dragged: bool,
    last_click: Option<(Entity, Instant)>,
}

impl ButtonCapture {
    fn completes_double_click(&mut self, entity: Entity, now: Instant) -> bool {
        let doubled = self.last_click.is_some_and(|(previous, at)| {
            previous == entity && now.duration_since(at) < DOUBLE_CLICK
        });
        self.last_click = (!doubled).then_some((entity, now));
        doubled
    }
}

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

const DRAG_THRESHOLD: f32 = 4.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

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

/// Opts a node into hit testing and pointer signals.
#[derive(Component)]
pub struct Interactable;

/// Marks a node as non-interactive.
#[derive(Component)]
pub struct UIDisabled;

/// Per-node colour palette for the four interaction states.
#[derive(Component, Clone)]
pub struct UIInteractionStyle {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
    pub disabled: Color,
}

/// Sent to a node when a button is released over the node it pressed.
pub struct UIClick {
    pub position: Vec2,
    pub button: MouseButton,
}

/// Sent to a node after the second [`UIClick`] of a quick pair on it.
pub struct UIDoubleClick {
    pub position: Vec2,
    pub button: MouseButton,
}

/// Sent to a node when a button goes down over it.
pub struct UIPointerDown {
    pub position: Vec2,
    pub button: MouseButton,
}

/// Sent to the node a button pressed when that button is released, wherever the pointer is.
pub struct UIPointerUp {
    pub position: Vec2,
    pub button: MouseButton,
}

/// Sent every frame to the node a held button pressed, wherever the pointer is.
pub struct UIDrag {
    pub position: Vec2,
    pub delta: Vec2,
    pub button: MouseButton,
}

/// Sent to a node when the pointer moves onto it.
pub struct UIPointerEnter {
    pub position: Vec2,
}

/// Sent to a node when the pointer moves off it.
pub struct UIPointerLeave {
    pub position: Vec2,
}

macro_rules! entity_signals {
    ($($signal:ty),* $(,)?) => {
        $(
            impl Signal for $signal {}
            impl EntitySignal for $signal {}
        )*
    };
}

entity_signals!(
    UIClick,
    UIDoubleClick,
    UIPointerDown,
    UIPointerUp,
    UIDrag,
    UIPointerEnter,
    UIPointerLeave,
);

/// Walks all [`UILayout`]s each frame, determines which one (if any) is under the cursor, updates [`HoveredNode`], and sends the pointer signals to the nodes involved.
pub(crate) fn update_ui_interaction(
    computed_nodes: Query<(Entity, &UILayout, &Interactable), Without<UIDisabled>>,
    input: Res<Input>,
    window: Res<concerto_window::plugin::Window>,
    mut hovered: ResMut<HoveredNode>,
    mut state: ResMut<UIInputState>,
    mut cmd: CommandQueue,
) {
    let cursor = window.logical_pointer_position(&input);
    route_pointer(
        cursor,
        Instant::now(),
        &computed_nodes,
        &input,
        &mut hovered,
        &mut state,
        &mut cmd,
    );
}

fn route_pointer(
    cursor: Vec2,
    now: Instant,
    computed_nodes: &Query<(Entity, &UILayout, &Interactable), Without<UIDisabled>>,
    input: &Input,
    hovered: &mut HoveredNode,
    state: &mut UIInputState,
    cmd: &mut CommandQueue,
) {
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
            cmd.entity(entity)
                .trigger(UIPointerLeave { position: cursor });
        }
        if let Some(entity) = hit {
            cmd.entity(entity)
                .trigger(UIPointerEnter { position: cursor });
        }
    }
    **hovered = hit;
    state.hovered = hit;

    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        let input_state = input.get_mouse_button_state(button);
        let Some(capture) = state.capture_mut(button) else {
            continue;
        };
        let outcome = advance_capture(capture, input_state, hit, cursor);
        if let Some(entity) = outcome.down {
            cmd.entity(entity).trigger(UIPointerDown {
                position: cursor,
                button,
            });
        }
        if let Some(entity) = outcome.up {
            cmd.entity(entity).trigger(UIPointerUp {
                position: cursor,
                button,
            });
        }
        if let Some(entity) = outcome.click {
            cmd.entity(entity).trigger(UIClick {
                position: cursor,
                button,
            });
            if capture.completes_double_click(entity, now) {
                cmd.entity(entity).trigger(UIDoubleClick {
                    position: cursor,
                    button,
                });
            }
        }
        if let Some(entity) = outcome.drag {
            cmd.entity(entity).trigger(UIDrag {
                position: cursor,
                delta: outcome.delta,
                button,
            });
        }
    }
}

/// Drives [`UIMaterial::color`] from [`UIInteractionStyle`] each frame.
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

    use super::{
        ButtonCapture, DOUBLE_CLICK, HoveredNode, Interactable, UIClick, UIDisabled, UIDoubleClick,
        UIDrag, UIInputState, UIPointerDown, UIPointerEnter, UIPointerLeave, UIPointerUp,
        advance_capture, route_pointer,
    };
    use concerto_ecs::{
        IntoSystem, System, World,
        command::CommandQueue,
        entity::Entity,
        query::{Query, filter::Without},
        resource::{Res, ResMut, Resource},
        signal::{On, listener::IntoListener},
    };
    use concerto_foundation::time::Instant;
    use concerto_window::input::{Input, InputState, MouseButton};
    use winit::event::ElementState;

    #[derive(Resource, Default)]
    struct Heard(Vec<(Entity, &'static str)>);

    #[derive(Resource)]
    struct Cursor(Vec2);

    #[derive(Resource)]
    struct Clock(Instant);

    fn square(min: f32) -> UILayout {
        let rect = UIBox {
            min: Vec2::splat(min),
            size: Vec2::splat(10.0),
        };
        UILayout {
            rect,
            content_rect: rect,
            clip_rect: rect,
            paint_order: 0,
        }
    }

    fn hear<S: concerto_ecs::signal::Signal>(
        name: &'static str,
    ) -> concerto_ecs::signal::listener::Listener<S> {
        (move |on: On<S>, mut heard: ResMut<Heard>| heard.0.push((on.entity(), name)))
            .into_listener()
    }

    fn node(world: &mut World, min: f32) -> Entity {
        world.spawn((
            square(min),
            Interactable,
            hear::<UIPointerEnter>("enter"),
            hear::<UIPointerLeave>("leave"),
            hear::<UIPointerDown>("down"),
            hear::<UIPointerUp>("up"),
            hear::<UIClick>("click"),
            hear::<UIDoubleClick>("double"),
            hear::<UIDrag>("drag"),
        ))
    }

    fn pointer_world() -> World {
        let mut world = World::default();
        world.insert_resource(Heard::default());
        world.insert_resource(Cursor(Vec2::ZERO));
        world.insert_resource(Clock(Instant::now()));
        world.insert_resource(Input::new());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world
    }

    fn frame(
        world: &mut World,
        cursor: Vec2,
        button: Option<ElementState>,
    ) -> Vec<(Entity, &'static str)> {
        world.get_resource_mut::<Cursor>().unwrap().0 = cursor;
        if let Some(state) = button {
            world
                .get_resource_mut::<Input>()
                .unwrap()
                .update_mouse_button(MouseButton::Left, state);
        }
        let mut route =
            (|cursor: Res<Cursor>,
              clock: Res<Clock>,
              nodes: Query<(Entity, &UILayout, &Interactable), Without<UIDisabled>>,
              input: Res<Input>,
              mut hovered: ResMut<HoveredNode>,
              mut state: ResMut<UIInputState>,
              mut cmd: CommandQueue| {
                route_pointer(
                    cursor.0,
                    clock.0,
                    &nodes,
                    &input,
                    &mut hovered,
                    &mut state,
                    &mut cmd,
                );
            })
            .into_system();
        route.initialize(world);
        route.run_and_apply((), world);
        world.get_resource_mut::<Input>().unwrap().update();
        std::mem::take(&mut world.get_resource_mut::<Heard>().unwrap().0)
    }

    #[test]
    fn a_press_and_release_reaches_the_listeners_on_the_node_under_the_pointer() {
        let mut world = pointer_world();
        let target = node(&mut world, 0.0);
        let _other = node(&mut world, 50.0);
        let inside = Vec2::splat(5.0);

        assert_eq!(frame(&mut world, inside, None), [(target, "enter")]);
        assert_eq!(
            frame(&mut world, inside, Some(ElementState::Pressed)),
            [(target, "down")]
        );
        assert_eq!(
            frame(&mut world, inside, Some(ElementState::Released)),
            [(target, "up"), (target, "click")]
        );
    }

    #[test]
    fn a_captured_node_hears_drags_and_the_release_after_the_pointer_leaves_it() {
        let mut world = pointer_world();
        let target = node(&mut world, 0.0);
        let other = node(&mut world, 50.0);
        let inside = Vec2::splat(5.0);
        let over_other = Vec2::splat(55.0);

        frame(&mut world, inside, None);
        frame(&mut world, inside, Some(ElementState::Pressed));
        assert_eq!(
            frame(&mut world, over_other, None),
            [(target, "leave"), (other, "enter"), (target, "drag")]
        );
        assert_eq!(
            frame(&mut world, over_other, Some(ElementState::Released)),
            [(target, "up")],
            "sliding off the pressed node cancels the click"
        );
    }

    #[test]
    fn a_disabled_node_hears_nothing() {
        let mut world = pointer_world();
        let target = node(&mut world, 0.0);
        world.insert(UIDisabled, target);
        let inside = Vec2::splat(5.0);

        assert!(frame(&mut world, inside, None).is_empty());
        assert!(frame(&mut world, inside, Some(ElementState::Pressed)).is_empty());
        assert!(frame(&mut world, inside, Some(ElementState::Released)).is_empty());
    }

    fn click(world: &mut World, cursor: Vec2) -> Vec<(Entity, &'static str)> {
        frame(world, cursor, Some(ElementState::Pressed));
        frame(world, cursor, Some(ElementState::Released))
    }

    fn wait(world: &mut World, duration: std::time::Duration) {
        world.get_resource_mut::<Clock>().unwrap().0 += duration;
    }

    #[test]
    fn a_second_click_on_the_same_node_in_quick_succession_is_a_double_click() {
        let mut world = pointer_world();
        let target = node(&mut world, 0.0);
        let inside = Vec2::splat(5.0);
        frame(&mut world, inside, None);

        assert_eq!(
            click(&mut world, inside),
            [(target, "up"), (target, "click")]
        );
        wait(&mut world, DOUBLE_CLICK / 2);
        assert_eq!(
            click(&mut world, inside),
            [(target, "up"), (target, "click"), (target, "double")]
        );
        assert_eq!(
            click(&mut world, inside),
            [(target, "up"), (target, "click")],
            "a third click starts a new pair"
        );
    }

    #[test]
    fn slow_clicks_and_clicks_on_different_nodes_are_not_double_clicks() {
        let mut world = pointer_world();
        let target = node(&mut world, 0.0);
        let other = node(&mut world, 50.0);
        let inside = Vec2::splat(5.0);
        let over_other = Vec2::splat(55.0);
        frame(&mut world, inside, None);

        click(&mut world, inside);
        wait(&mut world, DOUBLE_CLICK);
        assert_eq!(
            click(&mut world, inside),
            [(target, "up"), (target, "click")]
        );

        frame(&mut world, over_other, None);
        assert_eq!(
            click(&mut world, over_other),
            [(other, "up"), (other, "click")]
        );
    }

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
