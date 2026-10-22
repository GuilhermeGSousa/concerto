use concerto_color::Color;
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    query::Query,
    signal::{EntitySignal, On, Signal},
};
use concerto_window::input::MouseButton;

use crate::{interaction::UIClick, material::UIMaterial};

/// A toggleable boolean widget.
#[derive(Component)]
pub struct UICheckbox {
    pub checked: bool,
    /// Colour when `checked == true`.
    pub checked_color: Color,
    /// Colour when `checked == false`.
    pub unchecked_color: Color,
}

impl UICheckbox {
    pub fn new(checked: bool) -> Self {
        Self {
            checked,
            checked_color: Color::rgba(0.20, 0.50, 0.90, 1.0),
            unchecked_color: Color::rgba(0.12, 0.12, 0.12, 1.0),
        }
    }
}

/// Sent to a [`UICheckbox`] when it is toggled.
pub struct UICheckboxChanged {
    pub checked: bool,
}

impl Signal for UICheckboxChanged {}
impl EntitySignal for UICheckboxChanged {}

/// Click listener that toggles the [`UICheckbox`] it sits on.
pub fn toggle_checkbox(on: On<UIClick>, checkboxes: Query<&mut UICheckbox>, mut cmd: CommandQueue) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(mut checkbox) = checkboxes.get_entity(on.entity()) else {
        return;
    };
    checkbox.checked = !checkbox.checked;
    cmd.entity(on.entity()).trigger(UICheckboxChanged {
        checked: checkbox.checked,
    });
}

/// Drives `UIMaterial::color` from `UICheckbox::checked` each frame.
pub(crate) fn sync_checkbox_material(checkboxes: Query<(&UICheckbox, &mut UIMaterial)>) {
    for (checkbox, mut material) in checkboxes.iter() {
        material.color = if checkbox.checked {
            checkbox.checked_color
        } else {
            checkbox.unchecked_color
        }
        .to_linear();
    }
}
