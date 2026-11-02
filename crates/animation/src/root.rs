use concerto_ecs::component::Component;

use crate::pose::RootMotion;

#[derive(Component, Default)]
pub struct AnimationRootBone {
    /// How the root moved this frame; the bone's `Transform` shows the pose net of it.
    pub root_motion: RootMotion,
}
