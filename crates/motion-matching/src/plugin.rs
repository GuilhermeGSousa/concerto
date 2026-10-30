use concerto_app::{
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate},
};
use concerto_ecs::IntoSystemConfig;
use concerto_physics::step_simulation;

use crate::{
    simulation::{CharacterSimulation, DesiredMotion, TrajectorySample},
    systems::{drive_body, predict_trajectory_system, read_back_body},
};

pub struct MotionMatchingPlugin;

impl Plugin for MotionMatchingPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<DesiredMotion>()
            .register_type::<TrajectorySample>()
            .register_type::<CharacterSimulation>()
            .add_system(FixedUpdate, drive_body)
            .add_system(LateFixedUpdate, read_back_body.after(step_simulation))
            .add_system(
                LateFixedUpdate,
                predict_trajectory_system.after(read_back_body),
            );
    }
}
