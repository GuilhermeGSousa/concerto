use concerto_app::{Plugin, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;
use concerto_physics::interpolation::interpolate_body_transforms;

use crate::camera::{
    CameraBoom, CameraSettings, move_camera_pivot, update_camera_booms, update_entity_follow,
};

pub mod camera;
pub mod movement;
pub mod player;

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(CameraSettings::default());
        app.register_type::<CameraBoom>();

        app.add_system(Update, move_camera_pivot)
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
