use concerto_ecs::component::Component;
use glam::Vec3;

#[derive(Component, Default)]
pub struct AnimationRootBone {
    /// The root bone's animated local translation for the current frame (root motion).
    ///
    /// The root bone itself is kept at its bind pose rather than being animated; consume this
    /// to drive the character's world movement (e.g. by diffing it across frames).
    pub displacement: Vec3,
}
