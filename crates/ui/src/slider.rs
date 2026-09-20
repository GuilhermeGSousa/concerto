use color::Color;
use ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    events::{Event, event_writer::EventWriter},
    query::{
        Query,
        filter::{Added, With},
    },
    resource::{Res, Resource},
};
use window::input::{Input, InputState, MouseButton};

use crate::{
    interaction::HoveredNode,
    material::UIMaterial,
    node::{UILayout, UINode},
    transform::UIValue,
};

/// A draggable range slider widget.
///
/// Attach this alongside [`UINode`], [`UIMaterial`] (for the track background),
/// and [`Interactable`](crate::interaction::Interactable).
///
/// On the first frame the `setup_slider_visuals` system spawns a fill child
/// entity that visually shows the current value.  The `update_slider_drag`
/// system updates `value` while the left button is held over the slider, and
/// `sync_slider_fill` keeps the fill child's width in sync each frame.
///
/// Listen for [`UISliderChanged`] events to react to value changes.
#[derive(Component)]
pub struct UISlider {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub(crate) dragging: bool,
}

impl UISlider {
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        Self {
            value,
            min,
            max,
            dragging: false,
        }
    }

    fn normalized(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }
}

/// Marker placed on the fill child entity spawned by `setup_slider_visuals`.
#[derive(Component)]
pub struct UISliderFill;

/// Fired whenever a [`UISlider`]'s value changes during a drag.
#[derive(Event)]
pub struct UISliderChanged {
    pub entity: Entity,
    pub value: f32,
}

/// Dummy resource marker so the event can be registered.
#[derive(Resource)]
pub struct SliderResource;

/// Spawns a fill child entity for each newly added [`UISlider`].
///
/// The fill entity has [`UISliderFill`] and a [`UIMaterial`] in the accent
/// colour.  Its width (as a Taffy percent) is kept in sync by
/// `sync_slider_fill`.
pub(crate) fn setup_slider_visuals(
    new_sliders: Query<(Entity, &UISlider), Added<UISlider>>,
    mut cmd: CommandQueue,
) {
    for (entity, slider) in new_sliders.iter() {
        let fill = cmd
            .spawn((
                UISliderFill,
                UINode {
                    width: UIValue::Percent(slider.normalized() * 100.0),
                    height: UIValue::Percent(100.0),
                    ..Default::default()
                },
                UIMaterial::flat(Color::rgba(0.25, 0.55, 0.95, 1.0)),
            ))
            .entity();
        cmd.add_child(entity, fill);
    }
}

/// Updates [`UISlider::value`] while the user drags the slider.
///
/// Drag begins on `MouseButton::Left` `Pressed` over the slider and continues
/// while the button is held, even if the cursor leaves the node bounds.
pub(crate) fn update_slider_drag(
    sliders: Query<(Entity, &mut UISlider, &UILayout)>,
    input: Res<Input>,
    window: Res<window::plugin::Window>,
    hovered: Res<HoveredNode>,
    mut writer: EventWriter<UISliderChanged>,
) {
    let cursor = window.logical_pointer_position(&input);
    let left = input.get_mouse_button_state(MouseButton::Left);

    for (entity, mut slider, computed) in sliders.iter() {
        // Start drag when pressed over this slider.
        if left == InputState::Pressed && **hovered == Some(entity) {
            slider.dragging = true;
        }
        // Stop drag on release regardless of cursor position.
        if left == InputState::Released || left == InputState::Up {
            slider.dragging = false;
        }

        if slider.dragging && (left == InputState::Pressed || left == InputState::Down) {
            let norm = ((cursor.x - computed.rect.min.x) / computed.rect.size.x).clamp(0.0, 1.0);
            let new_value = slider.min + norm * (slider.max - slider.min);
            if (new_value - slider.value).abs() > f32::EPSILON {
                slider.value = new_value;
                writer.write(UISliderChanged {
                    entity,
                    value: new_value,
                });
            }
        }
    }
}

/// Keeps the fill child's width in sync with the slider's current value.
///
/// Queries fill entities via their [`ChildOf`] parent reference, so no entity
/// ID needs to be stored inside [`UISlider`].
pub(crate) fn sync_slider_fill(
    fills: Query<(&mut UINode, &ChildOf), With<UISliderFill>>,
    sliders: Query<&UISlider>,
) {
    for (mut node, child_of) in fills.iter() {
        if let Some(slider) = sliders.get_entity(**child_of) {
            node.width = UIValue::Percent(slider.normalized() * 100.0);
        }
    }
}
