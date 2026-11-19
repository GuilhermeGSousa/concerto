//! The editor camera shows the scene's main camera's sky and environment.
use concerto_app::{App, Plugin, edit_time::AtEditTime, schedule_groups::Update};
use concerto_director::MainCamera;
use concerto_ecs::{Component, Entity, Query, With, Without, command::CommandQueue};
use concerto_render::components::camera_environment::CameraEnvironment;
use concerto_skybox::Sky;

use crate::tools::camera::{EditorCamera, EditorHelper};

/// Copies the scene's `MainCamera`'s `Sky` and `CameraEnvironment` onto the editor camera, so the viewport lights and draws the scene as it will play.
pub struct ViewportEnvironmentPlugin;

impl Plugin for ViewportEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Update, mirror_scene_environment.at_edit_time());
    }
}

pub(crate) fn mirror_scene_environment(
    editor: Query<(Entity, Option<&Sky>, Option<&CameraEnvironment>), With<EditorCamera>>,
    main_cameras: Query<
        (Entity, Option<&Sky>, Option<&CameraEnvironment>),
        (With<MainCamera>, Without<EditorHelper>),
    >,
    mut cmd: CommandQueue,
) {
    let source = main_cameras
        .iter()
        .min_by_key(|(entity, ..)| entity.index());
    let sky_source = source.and_then(|(_, sky, _)| sky);
    let environment_source = source.and_then(|(_, _, environment)| environment);
    for (camera, sky, environment) in editor.iter() {
        mirror(&mut cmd, camera, sky, sky_source);
        mirror(&mut cmd, camera, environment, environment_source);
    }
}

fn mirror<T: Component + Clone + PartialEq>(
    cmd: &mut CommandQueue,
    camera: Entity,
    current: Option<&T>,
    source: Option<&T>,
) {
    match (source, current) {
        (Some(source), current) if current != Some(source) => cmd.insert(source.clone(), camera),
        (None, Some(_)) => cmd.remove::<T>(camera),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        camera::{EditorCamera, EditorHelper},
        edits::{ComponentEdit, EditComponent, SetValue},
    };
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_color::Color;
    use concerto_director::MainCamera;
    use concerto_ecs::{Component, Entity};
    use concerto_reflect::PropertyPath;
    use concerto_render::{
        ExclusiveCamera,
        components::{camera::Camera, camera_environment::CameraEnvironment},
    };
    use concerto_skybox::Sky;
    use serde_json::json;
    use std::any::TypeId;

    fn held<T: Component + Clone>(app: &mut App, entity: Entity) -> Option<T> {
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<T>(entity)
            .cloned()
    }

    #[test]
    fn the_viewport_mirrors_the_scene_cameras_sky_and_environment() {
        let mut app = App::for_editing();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin);
        crate::tools::build_core(&mut app);
        app.register_type::<Sky>()
            .register_type::<CameraEnvironment>()
            .register_type::<MainCamera>()
            .register_plugin(ViewportEnvironmentPlugin);
        app.finish_plugin_build();
        let world = app.main_mut().world_mut();
        let editor = world.spawn((Camera::default(), EditorCamera, EditorHelper));
        world.insert_resource(ExclusiveCamera(editor));
        let environment = CameraEnvironment {
            fog: None,
            ambient: Color::rgba(0.2, 0.3, 0.4, 1.0),
        };
        let second_sky = Sky::default().with_night_color(Color::rgba(0.5, 0.0, 0.0, 1.0));
        world.spawn((Camera::default(), second_sky.clone()));
        let first = world.spawn((MainCamera, Sky::default(), environment));
        let second = world.spawn((MainCamera, second_sky.clone()));

        for _ in 0..3 {
            app.update();
        }
        assert_eq!(held::<Sky>(&mut app, editor), Some(Sky::default()));
        assert_eq!(held(&mut app, editor), Some(environment));

        let night = Color::rgba(0.1, 0.2, 0.3, 1.0);
        app.main_mut().world_mut().trigger(SetValue {
            entity: first,
            component: TypeId::of::<Sky>(),
            path: PropertyPath::new(["night_color"]),
            value: json!([0.1, 0.2, 0.3, 1.0]),
        });
        app.update();
        assert_eq!(
            held::<Sky>(&mut app, editor).map(|sky| sky.night_color),
            Some(night)
        );

        let remove = |app: &mut App, entity, component| {
            app.main_mut()
                .world_mut()
                .trigger(EditComponent(ComponentEdit::Remove { entity, component }));
            app.update();
        };
        remove(&mut app, first, TypeId::of::<CameraEnvironment>());
        assert_eq!(held::<CameraEnvironment>(&mut app, editor), None);
        remove(&mut app, first, TypeId::of::<MainCamera>());
        assert_eq!(held(&mut app, editor), Some(second_sky));
        remove(&mut app, second, TypeId::of::<MainCamera>());
        assert_eq!(held::<Sky>(&mut app, editor), None);
        assert_eq!(held::<CameraEnvironment>(&mut app, editor), None);
    }
}
