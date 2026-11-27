use concerto_ecs::Component;
use glam::Vec3;

/// An axis-aligned box given by its minimum and maximum corners.
///
/// [`PhysicsShape::local_aabb`](crate::shape::PhysicsShape::local_aabb) returns one in the shape's
/// local space.
#[derive(Component, Clone)]
pub struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    /// Creates a box from its minimum and maximum corners, which the caller keeps ordered.
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// The corner with the smallest coordinate on every axis.
    pub fn min(&self) -> &Vec3 {
        &self.min
    }

    /// The corner with the largest coordinate on every axis.
    pub fn max(&self) -> &Vec3 {
        &self.max
    }
}
