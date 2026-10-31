use concerto_ecs::component::Component;

use crate::pose::RootMotion;

#[derive(Component, Default)]
pub struct AnimationRootBone {
    /// How the root moved this frame; the root bone itself stays at its bind pose.
    pub root_motion: RootMotion,
}
