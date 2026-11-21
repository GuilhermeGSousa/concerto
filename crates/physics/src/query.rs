use concerto_ecs::entity::Entity;
use glam::Vec3;

use crate::body::BodyId;

/// What a scene query skips.
#[derive(Clone, Copy, Debug, Default)]
pub struct QueryFilter<'a> {
    pub ignore: &'a [BodyId],
}

/// The first body a swept sphere touches, found by [`PhysicsState::cast_sphere`](crate::physics_state::PhysicsState::cast_sphere).
#[derive(Clone, Copy, Debug)]
pub struct ShapeHit {
    pub body: BodyId,
    pub entity: Option<Entity>,
    /// How far along the sweep the sphere first touches, as a fraction `[0, 1]` of `direction`.
    pub fraction: f32,
    /// World-space contact point on the hit body.
    pub point: Vec3,
    /// World-space normal of the hit surface, pointing away from it.
    pub normal: Vec3,
}
