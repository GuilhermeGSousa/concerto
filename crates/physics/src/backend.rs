//! The interface between the engine-facing physics API and a concrete
//! physics engine.
//!
//! Everything above this trait — the ECS components and their lifecycles, the
//! body→entity cache, [`RayHit`](crate::ray::RayHit) assembly, ground-slope
//! classification — is shared code written once. A backend only translates
//! the shared descriptor types into its own world and answers raw queries
//! about it. Exactly one backend is compiled into a build; see the
//! [`ActiveBackend`](crate::ActiveBackend) alias.

use std::error::Error;
use std::fmt::{Debug, Display};
use std::hash::Hash;

use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::Vec3;

use crate::aabb::Aabb;
use crate::collider::{Collider, ColliderOffset};
use crate::rigid_body::RigidBody;

#[cfg(all(
    not(target_arch = "wasm32"),
    not(feature = "force-rapier"),
    feature = "jolt"
))]
pub mod jolt;
#[cfg(any(target_arch = "wasm32", feature = "force-rapier"))]
pub mod rapier;

/// The closest hit of a raycast, in backend terms. The facade resolves the
/// entity and computes the world-space hit point.
pub struct RawRayHit<Handle> {
    /// The body that was hit.
    pub body: Handle,
    /// Hit distance as a fraction `[0, 1]` of the ray's direction vector.
    pub fraction: f32,
    /// World-space surface normal at the hit point.
    pub normal: Vec3,
}

/// A ground contact found by [`PhysicsBackend::probe_ground`]: the contact
/// with the most upward-facing normal within reach of the probed body's
/// shape. Slope classification against a walkable angle happens in the
/// facade.
pub struct RawGroundHit<Handle> {
    /// The ground body.
    pub body: Handle,
    /// World-space contact point on the ground.
    pub point: Vec3,
    /// World-space contact normal (pointing away from the ground).
    pub normal: Vec3,
    /// The ground body's velocity at the contact point (moving platforms).
    pub velocity: Vec3,
}

/// Raw simulation operations a physics backend must provide.
///
/// Mutating operations take `&mut self` even where the underlying engine
/// could do with less, so the exclusivity the ECS scheduler grants a
/// `ResMut<PhysicsState>` is mirrored in the signatures.
pub trait PhysicsBackend: Send + Sync + Sized {
    /// The backend's native body handle, wrapped by the shared
    /// [`BodyId`](crate::body::BodyId) component and opaque above this trait.
    type BodyHandle: Copy + PartialEq + Eq + Hash + Debug + Send + Sync;

    /// The backend's native collision shape, wrapped by the shared
    /// [`PhysicsShape`](crate::shape::PhysicsShape). Shared behind an `Arc`
    /// by every [`Collider`] using it, hence the thread bounds.
    type ShapeHandle: Send + Sync;

    /// Per-step scratch held by
    /// [`PhysicsPipeline`](crate::physics_pipeline::PhysicsPipeline),
    /// separate from the world so stepping can borrow both.
    type Stepper: Send + Sync;

    fn new() -> Self;

    fn new_stepper() -> Self::Stepper;

    /// Advances the world by `delta_time` seconds (one fixed timestep).
    fn step(&mut self, stepper: &mut Self::Stepper, delta_time: f32);

    /// Creates a body from the shared descriptors and adds it to the
    /// simulation: static when `rigid_body` is `None`, otherwise dynamic or
    /// kinematic per its [`MotionType`](crate::rigid_body::MotionType), with
    /// its density and allowed degrees of freedom. `offset` displaces the
    /// shape's geometry relative to the body origin; it must not leak into
    /// [`body_transform`](Self::body_transform).
    fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
    ) -> Self::BodyHandle;

    /// Removes `body` from the simulation and destroys it, waking any bodies
    /// that were touching it. The handle is invalid afterwards.
    fn destroy_body(&mut self, body: Self::BodyHandle);

    /// A body's current world-space pose (translation and rotation).
    fn body_transform(&self, body: Self::BodyHandle) -> Transform;

    fn linear_velocity(&self, body: Self::BodyHandle) -> Vec3;

    /// Also wakes the body, so a sleeping body picks the velocity up.
    fn set_linear_velocity(&mut self, body: Self::BodyHandle, velocity: Vec3);

    fn add_impulse(&mut self, body: Self::BodyHandle, impulse: Vec3);

    fn add_impulse_at(&mut self, body: Self::BodyHandle, impulse: Vec3, position: Vec3);

    fn add_force(&mut self, body: Self::BodyHandle, force: Vec3);

    fn add_force_at(&mut self, body: Self::BodyHandle, force: Vec3, position: Vec3);

    /// Casts a ray from `origin` along `direction` — whose length is the
    /// maximum cast distance — and returns the closest hit, if any.
    fn cast_ray(&self, origin: Vec3, direction: Vec3) -> Option<RawRayHit<Self::BodyHandle>>;

    /// Collides `body`'s shape against the world (ignoring `body` itself)
    /// and reports the contact with the most upward-facing normal within
    /// `max_separation` of the shape, or `None` when airborne.
    fn probe_ground(
        &self,
        body: Self::BodyHandle,
        max_separation: f32,
    ) -> Option<RawGroundHit<Self::BodyHandle>>;

    /// The shape's axis-aligned bounds (min, max) in its own local space,
    /// before any body pose or scale. The engine no longer stores a
    /// collider's dimensions, so this is how they are recovered.
    fn shape_local_aabb(shape: &Self::ShapeHandle) -> Aabb;

    fn create_sphere_shape(radius: f32) -> Self::ShapeHandle;
    fn create_cuboid_shape(width: f32, height: f32, length: f32) -> Self::ShapeHandle;
    fn create_capsule_shape(half_height: f32, radius: f32) -> Self::ShapeHandle;
    fn create_shape_from_mesh(mesh: &Mesh) -> Result<Self::ShapeHandle, MeshShapeCreationError>;
}

#[derive(Debug)]
pub struct MeshShapeCreationError;

impl Display for MeshShapeCreationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Failed to create mesh shape")
    }
}
impl Error for MeshShapeCreationError {}
