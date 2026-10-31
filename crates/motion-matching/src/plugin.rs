use concerto_animation::{animate_targets, update_animation_players};
use concerto_app::{
    edit_time::AtPreviewTime,
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate},
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
    runtime::run_motion_matching,
    simulation::{CharacterSimulation, DesiredMotion, TrajectorySample},
    systems::{drive_body, publish_trajectory},
};

pub(crate) fn register_runtime(app: &mut concerto_app::App) {
    app.add_system(
        LateUpdate,
        run_motion_matching
            .after(update_animation_players)
            .before(animate_targets)
            .at_preview_time(),
    );
}

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
            .add_system(LateFixedUpdate, publish_trajectory.after(step_simulation));
        register_runtime(app);
    }
}
