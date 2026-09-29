use concerto_app::{Plugin, schedule_groups::Update};

use crate::camera::{CameraSettings, move_camera_pivot, update_entity_follow};

pub mod camera;
pub mod movement;
pub mod player;

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(CameraSettings::default());

        app.add_system(Update, move_camera_pivot)
            .add_system(Update, update_entity_follow);
    }
}
