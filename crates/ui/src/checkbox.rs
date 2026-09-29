use concerto_color::Color;
use concerto_ecs::{
    component::Component,
    entity::Entity,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    query::Query,
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

/// Fired the frame a [`UICheckbox`] is toggled.
#[derive(Event)]
pub struct UICheckboxChanged {
    pub entity: Entity,
    pub checked: bool,
}

/// Toggles [`UICheckbox::checked`] when the entity receives a [`UIClick`].
pub(crate) fn toggle_checkboxes(
    mut clicks: EventReader<UIClick>,
    checkboxes: Query<&mut UICheckbox>,
    mut writer: EventWriter<UICheckboxChanged>,
) {
    for click in clicks.read() {
        if click.button != MouseButton::Left {
            continue;
        }
        if let Some(mut checkbox) = checkboxes.get_entity(click.entity) {
            checkbox.checked = !checkbox.checked;
            writer.write(UICheckboxChanged {
                entity: click.entity,
                checked: checkbox.checked,
            });
        }
    }
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
