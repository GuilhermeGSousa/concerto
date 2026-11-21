use concerto_animation::{param::Param, plugin::AnimationAppExt};
use concerto_app::{App, plugins::Plugin};
use glam::Vec3;

use crate::{
    aim::AimNode,
    foot_lock::FootLockNode,
    foot_placement::{FootPlacementNode, IgnoredBody},
    look_at::LookAtNode,
    procedural_motion::ProceduralMotionNode,
};

/// Registers the procedural animation nodes: foot placement, which reads physics, foot locking, aim, look-at, and procedural motion.
pub struct ProceduralAnimationPlugin;

impl Plugin for ProceduralAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.register_animation_node::<FootPlacementNode>();
        app.register_animation_node::<FootLockNode>();
        app.register_animation_node::<LookAtNode>();
        app.register_animation_node::<AimNode>();
        app.register_animation_node::<ProceduralMotionNode>();
        app.register_type::<Param<f32>>()
            .register_type::<Param<u64>>()
            .register_type::<Param<Vec3>>()
            .register_type::<Param<IgnoredBody>>();
    }
}
