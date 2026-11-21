use concerto_ecs::{
    Component, Entity, Query, ResMut, With,
    entity::EntityHashSet,
    reflect::{ReflectComponent, ReflectSceneComponent},
    system::input::SystemLocal,
    world::{FromWorld, World},
};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::components::camera::Camera;

use crate::director::CameraDirector;

/// Marks the level's camera that the director drives from the live virtual camera.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Camera)]
pub struct MainCamera;

pub(crate) struct UndrivenWarnings(EntityHashSet);

impl FromWorld for UndrivenWarnings {
    fn from_world(_world: &World) -> Self {
        Self(EntityHashSet::default())
    }
}

pub(crate) fn adopt_main_camera(
    mut director: ResMut<CameraDirector>,
    main_cameras: Query<Entity, With<MainCamera>>,
    mut warned: SystemLocal<UndrivenWarnings>,
) {
    let driven = main_cameras.iter().min_by_key(|entity| entity.index());
    if director.main_camera() != driven {
        director.set_main_camera(driven);
    }
    let undriven: EntityHashSet = main_cameras
        .iter()
        .filter(|entity| Some(*entity) != driven)
        .collect();
    for entity in undriven.difference(&warned.0) {
        log::warn!(
            "MainCamera on {entity:?} is not driven: {driven:?}, the MainCamera with the lowest entity index, is"
        );
    }
    warned.0 = undriven;
}
