use concerto_app::{
    Plugin,
    schedule_groups::{FixedUpdate, Update},
};
use concerto_ecs::IntoSystemConfig;
use concerto_navigation::steer_nav_agents;
use concerto_physics::interpolation::interpolate_body_transforms;
use concerto_window::input::actions::ActionMap;

use crate::actions::bind_gameplay_actions;
use crate::camera::{
    CameraBoom, CameraSettings, move_camera_pivot, update_camera_booms, update_entity_follow,
};
use crate::first_person::{
    place_first_person_views, read_first_person_input, step_first_person_walkers,
};
use crate::navigation::follow_nav_steering;
use crate::third_person::{ThirdPersonCamera, update_third_person_cameras};

pub mod actions;
pub mod camera;
pub mod first_person;
pub mod movement;
pub mod navigation;
pub mod player;
pub mod third_person;

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        bind_gameplay_actions(
            app.get_resource_mut::<ActionMap>()
                .expect("WindowPlugin must be registered before GameplayPlugin"),
        );
        app.insert_resource(CameraSettings::default());
        app.register_type::<CameraBoom>();
        app.register_type::<ThirdPersonCamera>();

        app.add_system(Update, update_third_person_cameras)
            .add_system(Update, move_camera_pivot.after(update_third_person_cameras))
            .add_system(
                Update,
                update_entity_follow.after(interpolate_body_transforms),
            )
            .add_system(
                Update,
                update_camera_booms
                    .after(move_camera_pivot)
                    .after(update_entity_follow),
            )
            .add_system(Update, follow_nav_steering.after(steer_nav_agents))
            .add_system(Update, read_first_person_input)
            .add_system(FixedUpdate, step_first_person_walkers)
            .add_system(
                Update,
                place_first_person_views
                    .after(interpolate_body_transforms)
                    .after(read_first_person_input),
            );
    }
}
