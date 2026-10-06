use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::{Entity, hierarchy::ChildOf},
    query::{
        Query,
        filter::{Added, With},
    },
    signal::{EntitySignal, On, Signal},
};
use concerto_window::input::MouseButton;
use glam::Vec2;

use crate::{
    interaction::{UIDrag, UIPointerDown},
    material::UIMaterial,
    node::{UILayout, UINode},
    transform::UIValue,
};

/// A draggable range slider widget.
#[derive(Component)]
pub struct UISlider {
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

impl UISlider {
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        Self { value, min, max }
    }

    fn normalized(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }
}

/// Marker placed on the fill child entity spawned by `setup_slider_visuals`.
#[derive(Component)]
pub(crate) struct UISliderFill;

/// Sent to a [`UISlider`] whenever its value changes during a drag.
pub struct UISliderChanged {
    pub value: f32,
}

impl Signal for UISliderChanged {}
impl EntitySignal for UISliderChanged {}

/// Spawns a fill child entity for each newly added [`UISlider`].
pub(crate) fn setup_slider_visuals(
    new_sliders: Query<(Entity, &UISlider), Added<UISlider>>,
    mut cmd: CommandQueue,
) {
    for (entity, slider) in new_sliders.iter() {
        let fill = cmd
            .spawn((
                UISliderFill,
                UINode::default().with_size(
                    UIValue::Percent(slider.normalized() * 100.0),
                    UIValue::Percent(100.0),
                ),
                UIMaterial::flat(Color::rgba(0.25, 0.55, 0.95, 1.0)),
            ))
            .entity();
        cmd.add_child(entity, fill);
    }
}

/// Pointer-down listener that moves the [`UISlider`] it sits on to the pointer.
pub fn press_slider(
    on: On<UIPointerDown>,
    sliders: Query<(&mut UISlider, &UILayout)>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    slide_to(
        on.entity(),
        signal.button,
        signal.position,
        &sliders,
        &mut cmd,
    );
}

/// Drag listener that keeps the [`UISlider`] it sits on under the pointer.
pub fn drag_slider(
    on: On<UIDrag>,
    sliders: Query<(&mut UISlider, &UILayout)>,
    mut cmd: CommandQueue,
) {
    let signal = on.signal();
    slide_to(
        on.entity(),
        signal.button,
        signal.position,
        &sliders,
        &mut cmd,
    );
}

fn slide_to(
    entity: Entity,
    button: MouseButton,
    position: Vec2,
    sliders: &Query<(&mut UISlider, &UILayout)>,
    cmd: &mut CommandQueue,
) {
    if button != MouseButton::Left {
        return;
    }
    let Some((mut slider, layout)) = sliders.get_entity(entity) else {
        return;
    };
    let norm = ((position.x - layout.rect.min.x) / layout.rect.size.x).clamp(0.0, 1.0);
    let value = slider.min + norm * (slider.max - slider.min);
    if (value - slider.value).abs() > f32::EPSILON {
        slider.value = value;
        cmd.entity(entity).trigger(UISliderChanged { value });
    }
}

/// Keeps the fill child's width in sync with the slider's current value.
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
