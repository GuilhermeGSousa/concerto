use concerto_animation::plugin::AnimationAppExt;
use concerto_app::{App, plugins::Plugin};

use crate::{
    aim::AimNode, foot_lock::FootLockNode, foot_placement::FootPlacementNode, look_at::LookAtNode,
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
    }
}
