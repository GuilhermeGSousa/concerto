use concerto_animation::plugin::AnimationAppExt;
use concerto_app::{App, plugins::Plugin};

use crate::{foot_placement::FootPlacementNode, look_at::LookAtNode};

/// Registers the procedural animation nodes: foot placement, which reads physics, and look-at.
pub struct ProceduralAnimationPlugin;

impl Plugin for ProceduralAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.register_animation_node::<FootPlacementNode>();
        app.register_animation_node::<LookAtNode>();
    }
}
