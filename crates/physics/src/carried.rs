use concerto_ecs::Component;
use glam::{Quat, Vec3};

/// Marks a body that something else carries, such as a character's capsule under its ragdoll.
///
/// Whatever drives the body leaves it alone while this is on it, and picks
/// up from this velocity and facing when it is removed.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Carried {
    /// The velocity of what carries the body, in m/s.
    pub velocity: Vec3,
    /// The way what carries the body faces, as a rotation about Y.
    pub facing: Quat,
}
