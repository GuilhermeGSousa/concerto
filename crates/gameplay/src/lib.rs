use concerto_app::{Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_physics::interpolation::interpolate_body_transforms;

use crate::camera::{
    CameraBoom, CameraSettings, move_camera_pivot, update_camera_booms, update_entity_follow,
};
use crate::third_person::{ThirdPersonCamera, update_third_person_cameras};

pub mod camera;
pub mod movement;
pub mod player;
pub mod third_person;

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
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
            );
    }
}
