use concerto_ecs::Entity;
use glam::Vec3;

use crate::body::BodyId;

/// The closest hit found by [`PhysicsState::cast_ray`].
///
/// [`PhysicsState::cast_ray`]: crate::physics_state::PhysicsState::cast_ray
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    /// The body that was hit.
    pub body: BodyId,
    /// The entity that corresponds to this body
    pub entity: Option<Entity>,
    /// Hit distance as a fraction `[0, 1]` of the ray's direction vector.
    pub fraction: f32,
    /// World-space position of the hit.
    pub point: Vec3,
    /// World-space surface normal at the hit point.
    pub normal: Vec3,
}
