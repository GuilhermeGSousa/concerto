//! The one thing every panel agrees on.
use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    ResMut,
    entity::Entity,
    resource::Resource,
    signal::{On, Signal},
};

#[derive(Resource, Default)]
pub struct Selection {
    current: Option<Entity>,
}

impl Selection {
    pub fn entity(&self) -> Option<Entity> {
        self.current
    }

    pub(crate) fn set(&mut self, entity: Option<Entity>) {
        self.current = entity;
    }
}

/// Selects one entity of the scene, or nothing.
pub struct Select(pub Option<Entity>);

impl Signal for Select {}

pub(crate) fn select(on: On<Select>, mut selection: ResMut<Selection>) {
    if selection.entity() != on.signal().0 {
        selection.set(on.signal().0);
    }
}

pub(crate) fn build(app: &mut App) {
    app.insert_resource(Selection::default())
        .add_listener(select.at_edit_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::scene::testing::tools_app;

    #[test]
    fn a_select_signal_sets_and_clears_the_selection() {
        let mut app = tools_app();
        let entity = app.main_mut().world_mut().spawn(());

        app.main_mut().world_mut().trigger(Select(Some(entity)));
        assert_eq!(
            app.get_resource::<Selection>().unwrap().entity(),
            Some(entity)
        );

        app.main_mut().world_mut().trigger(Select(None));
        assert_eq!(app.get_resource::<Selection>().unwrap().entity(), None);
    }

    #[test]
    fn selecting_what_is_already_selected_does_not_mark_the_selection_changed() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        let entity = world.spawn(());
        world.trigger(Select(Some(entity)));
        let changed = world.resource_changed_tick::<Selection>();
        world.tick();

        world.trigger(Select(Some(entity)));

        assert_eq!(world.resource_changed_tick::<Selection>(), changed);
    }
}
