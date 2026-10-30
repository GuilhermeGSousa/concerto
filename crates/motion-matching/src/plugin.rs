use concerto_app::{
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_physics::step_simulation;

use crate::{
    database::MotionDatabase,
    definition::{
        MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, TimeRange,
        TrajectorySettings,
    },
    features::{BonePositions, BoneVelocities, TrajectoryDirections, TrajectoryPositions},
    node::{MotionMatchingNode, MotionMatchingTrajectory},
    simulation::{CharacterSimulation, DesiredMotion, TrajectorySample},
    systems::{drive_body, predict_trajectory_system, read_back_body},
};

pub struct MotionMatchingPlugin;

impl Plugin for MotionMatchingPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_asset::<MotionDatabaseDefinition>()
            .register_asset::<MotionDatabase>()
            .register_type::<TimeRange>()
            .register_type::<MotionEntry>()
            .register_type::<SimulationBoneSettings>()
            .register_type::<TrajectorySettings>()
            .register_type::<BonePositions>()
            .register_type::<BoneVelocities>()
            .register_type::<TrajectoryPositions>()
            .register_type::<TrajectoryDirections>()
            .register_type::<DesiredMotion>()
            .register_type::<TrajectorySample>()
            .register_type::<CharacterSimulation>()
            .register_type::<AssetHandle<MotionDatabase>>()
            .register_type::<MotionMatchingTrajectory>()
            .register_type::<MotionMatchingNode>()
            .add_system(FixedUpdate, drive_body)
            .add_system(LateFixedUpdate, read_back_body.after(step_simulation))
            .add_system(
                LateFixedUpdate,
                predict_trajectory_system.after(read_back_body),
            );
    }
}
