use concerto_ecs::{
    component::Component,
    entity::Entity,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    query::{Query, filter::Without},
    resource::{Res, ResMut, Resource},
};
use concerto_window::define_action;
use concerto_window::input::{
    Input, InputState, MouseButton,
    actions::{ActionFired, ActionMap},
};
use derive_more::{Deref, DerefMut};

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
#[derive(Component)]
pub struct UIFocusable;

/// The UI entity that currently holds keyboard focus, if any.
#[derive(Resource, Deref, DerefMut, Default)]
pub struct FocusedWidget(Option<concerto_ecs::entity::Entity>);

#[derive(Event)]
pub struct UIFocusGained(pub Entity);

#[derive(Event)]
pub struct UIFocusLost(pub Entity);

/// Sets [`FocusedWidget`] based on left-button clicks.
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
            .filter(|(_, layout, node, _, _)| node.visible && layout.is_shown())
            .map(|(entity, layout, _, _, _)| (layout.paint_order, entity))
            .collect::<Vec<_>>();
        order.sort_by_key(|(paint_order, _)| *paint_order);
        if !order.is_empty() {
            let current = order
                .iter()
                .position(|(_, entity)| Some(*entity) == **focused);
            let len = order.len() as isize;
            let next = match current {
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

/// Tells the action map that keystrokes belong to a text field right now.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{UIBox, UICulled, blank_culled_layouts};
    use concerto_ecs::{IntoSystem, System, World, events::event_channel::EventChannel};
    use concerto_window::input::actions::ActionLabel;
    use glam::Vec2;

    fn focus_world() -> World {
        let mut world = World::new();
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(Input::new());
        world.insert_resource(EventChannel::<ActionFired>::default());
        world.insert_resource(EventChannel::<UIFocusGained>::default());
        world.insert_resource(EventChannel::<UIFocusLost>::default());
        world.add_listener(blank_culled_layouts);
        world
    }

    fn row(world: &mut World, index: i64) -> Entity {
        let rect = UIBox {
            min: Vec2::new(0.0, index as f32 * 20.0),
            size: Vec2::new(100.0, 20.0),
        };
        world.spawn((
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: index,
            },
            UINode::default(),
            Interactable,
            UIFocusable,
        ))
    }

    #[test]
    fn tab_skips_a_culled_row() {
        let mut world = focus_world();
        let first = row(&mut world, 0);
        let culled = row(&mut world, 1);
        let last = row(&mut world, 2);
        world.insert(UICulled, culled);
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(first);
        world
            .get_resource_mut::<EventChannel<ActionFired>>()
            .unwrap()
            .push_event(ActionFired {
                action: UIFocusNext.intern(),
            });

        let mut system = update_focus.into_system();
        system.initialize(&mut world);
        system.run_and_apply((), &mut world);

        assert_eq!(**world.get_resource::<FocusedWidget>().unwrap(), Some(last));
    }
}
