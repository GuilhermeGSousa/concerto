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
