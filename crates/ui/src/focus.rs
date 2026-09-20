use derive_more::{Deref, DerefMut};
use ecs::{
    component::Component,
    entity::Entity,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    query::{Query, filter::Without},
    resource::{Res, ResMut, Resource},
};
use window::define_action;
use window::input::{
    Input, InputState, MouseButton,
    actions::{ActionFired, ActionMap},
};

use crate::{
    interaction::{HoveredNode, Interactable, UIDisabled},
    node::{UILayout, UINode},
    text_input::UITextInput,
};

define_action!(
    /// Move keyboard focus to the next focusable widget.
    UIFocusNext
);
define_action!(
    /// Move keyboard focus to the previous focusable widget.
    UIFocusPrevious
);

/// Opts a widget into the keyboard focus ring.
///
/// Deliberately separate from [`Interactable`]: a tree row or a split handle is
/// clickable but has no business being a Tab stop, and a ring that visits every
/// interactable node is one nobody can use.
#[derive(Component)]
pub struct UIFocusable;

/// The UI entity that currently holds keyboard focus, if any.
///
/// Focus is set when the user clicks an [`Interactable`](crate::interaction::Interactable) node
/// and cleared when the user clicks on empty space.  Widgets that need keyboard
/// input (e.g. [`UITextInput`](crate::text_input::UITextInput)) read this resource to decide
/// whether to consume typed characters.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct FocusedWidget(Option<ecs::entity::Entity>);

#[derive(Event)]
pub struct UIFocusGained(pub Entity);

#[derive(Event)]
pub struct UIFocusLost(pub Entity);

/// Sets [`FocusedWidget`] based on left-button clicks.
///
/// - Click on an interactable node → focus that node.
/// - Click on empty space (no node hovered) → clear focus.
pub(crate) fn update_focus(
    mut focused: ResMut<FocusedWidget>,
    hovered: Res<HoveredNode>,
    input: Res<Input>,
    focusable: Query<
        (Entity, &UILayout, &UINode, &Interactable, &UIFocusable),
        Without<UIDisabled>,
    >,
    mut actions: EventReader<ActionFired>,
    mut gained: EventWriter<UIFocusGained>,
    mut lost: EventWriter<UIFocusLost>,
) {
    let previous = **focused;
    if input.get_mouse_button_state(MouseButton::Left) == InputState::Pressed {
        **focused = **hovered;
    }
    let step = actions.read().find_map(|fired| {
        if fired.is(UIFocusNext) {
            Some(1_isize)
        } else if fired.is(UIFocusPrevious) {
            Some(-1)
        } else {
            None
        }
    });
    if let Some(step) = step {
        let mut order = focusable
            .iter()
            .filter(|(_, _, node, _, _)| node.visible)
            .map(|(entity, layout, _, _, _)| (layout.paint_order, entity))
            .collect::<Vec<_>>();
        order.sort_by_key(|(paint_order, _)| *paint_order);
        if !order.is_empty() {
            let current = order
                .iter()
                .position(|(_, entity)| Some(*entity) == **focused);
            let len = order.len() as isize;
            let next = match current {
                // Wraps in both directions, so Tab from the last widget and
                // Shift+Tab from the first both stay inside the ring.
                Some(index) => (index as isize + step).rem_euclid(len),
                None if step > 0 => 0,
                None => len - 1,
            };
            **focused = Some(order[next as usize].1);
        }
    }
    if previous != **focused {
        if let Some(entity) = previous {
            lost.write(UIFocusLost(entity));
        }
        if let Some(entity) = **focused {
            gained.write(UIFocusGained(entity));
        }
    }
}

/// Tells the action map that keystrokes belong to a text field right now, so a
/// bare letter types instead of firing a shortcut.
pub(crate) fn sync_text_capture(
    focused: Res<FocusedWidget>,
    inputs: Query<&UITextInput>,
    mut actions: ResMut<ActionMap>,
) {
    let capturing = (**focused).is_some_and(|entity| inputs.get_entity(entity).is_some());
    if actions.capturing_text() != capturing {
        actions.set_capturing_text(capturing);
    }
}
