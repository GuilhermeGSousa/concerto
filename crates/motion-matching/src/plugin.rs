use concerto_animation::{animate_targets, plugin::AnimationAppExt};
use concerto_app::{
    plugins::Plugin,
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::{
    assets::handle::AssetHandle,
    transform::systems::{propagate_global_transforms, update_simple_entities},
};
use concerto_physics::step_simulation;

use crate::{
    database::MotionDatabase,
    definition::{
        MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, TimeRange,
        TrajectorySettings,
    },
    displacement::{CharacterDisplacement, CharacterLink},
    features::{BonePositions, BoneVelocities, TrajectoryDirections, TrajectoryPositions},
    node::{MotionMatchingNode, MotionMatchingTrajectory},
    simulation::{CharacterSimulation, DesiredMotion, TrajectorySample},
    systems::{apply_character_displacement, drive_body, publish_trajectory},
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
            .register_type::<CharacterDisplacement>()
            .register_type::<CharacterLink>()
            .register_type::<AssetHandle<MotionDatabase>>()
            .register_type::<MotionMatchingTrajectory>()
            .register_animation_node::<MotionMatchingNode>()
            .add_system(FixedUpdate, drive_body)
            .add_system(LateFixedUpdate, publish_trajectory.after(step_simulation))
            .add_system(
                LateUpdate,
                apply_character_displacement
                    .after(animate_targets)
                    .before(update_simple_entities)
                    .before(propagate_global_transforms),
            );
    }
}

#[cfg(test)]
mod tests;
