use concerto_ecs::component::Component;

use crate::pose::RootMotion;

/// Marks the root bone of a skeleton and carries the root motion its player produced this frame.
///
/// The [`AnimationPlugin`](crate::plugin::AnimationPlugin) inserts it on the root bone of each
/// new skeleton, and [`update_animation_players`](crate::update_animation_players) writes it
/// every frame.
#[derive(Component, Default)]
pub struct AnimationRootBone {
    /// How the root moved this frame; the bone's `Transform` shows the pose net of it.
    pub root_motion: RootMotion,
}
