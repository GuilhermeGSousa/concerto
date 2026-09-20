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
    pub pressed: Option<Entity>,
    pub captured: Option<Entity>,
    press_origin: Option<Vec2>,
    last_cursor: Option<Vec2>,
    dragged: bool,
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

/// Fired when the left button is released over the same node it pressed.
#[derive(Event)]
pub struct UIClick {
    pub entity: Entity,
    pub position: Vec2,
}

#[derive(Event)]
pub struct UIPointerDown {
    pub entity: Entity,
    pub position: Vec2,
}

#[derive(Event)]
pub struct UIPointerUp {
    pub entity: Entity,
    pub position: Vec2,
}

#[derive(Event)]
pub struct UIDrag {
    pub entity: Entity,
    pub position: Vec2,
    pub delta: Vec2,
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

    match input.get_mouse_button_state(MouseButton::Left) {
        InputState::Pressed => {
            state.pressed = hit;
            state.captured = hit;
            state.press_origin = hit.map(|_| cursor);
            state.dragged = false;
            if let Some(entity) = hit {
                down_writer.write(UIPointerDown {
                    entity,
                    position: cursor,
                });
            }
        }
        InputState::Down => {
            if let Some(entity) = state.captured {
                state.dragged |= state
                    .press_origin
                    .is_some_and(|origin| origin.distance(cursor) >= 4.0);
                let delta = cursor - state.last_cursor.unwrap_or(cursor);
                drag_writer.write(UIDrag {
                    entity,
                    position: cursor,
                    delta,
                });
            }
        }
        InputState::Released => {
            if let Some(entity) = state.captured {
                up_writer.write(UIPointerUp {
                    entity,
                    position: cursor,
                });
                if state.pressed == hit && !state.dragged {
                    click_writer.write(UIClick {
                        entity,
                        position: cursor,
                    });
                }
            }
            state.pressed = None;
            state.captured = None;
            state.press_origin = None;
            state.dragged = false;
        }
        InputState::Up => {}
    }

    state.last_cursor = Some(cursor);
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
