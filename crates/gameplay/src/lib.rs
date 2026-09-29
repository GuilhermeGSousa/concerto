use concerto_app::{
    Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::systems::propagate_global_transforms;

use crate::{
    camera::{CameraSettings, move_camera_pivot, update_entity_follow},
    observation::{ObservationSettings, PreviousViewpoint, update_observation},
};

pub mod camera;
pub mod movement;
pub mod observation;
pub mod player;

pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(CameraSettings::default())
            .insert_resource(ObservationSettings::default())
            .insert_resource(PreviousViewpoint::default());

        app.add_system(Update, move_camera_pivot)
            .add_system(Update, update_entity_follow)
            // After propagation, so observables are tested against the pose
            // the camera is about to be drawn from.
            .add_system(
                LateUpdate,
                update_observation.after(propagate_global_transforms),
            );
    }
}
