//! Hand-written FFI bindings to the Jolt Physics library.
//!
//! The C API is our own thin shim (headers and sources in `csrc/`) over the
//! unmodified Jolt sources at `vendor/JoltPhysics` — a git submodule pinned
//! to the upstream v5.0.0 release (MIT licensed, see
//! `vendor/JoltPhysics/LICENSE`). Run `git submodule update --init` after
//! cloning to fetch it. Everything — Jolt and the shim — is compiled by
//! `build.rs` in a single `cc` invocation, so there is no external binding
//! crate, no bindgen, and no CMake involved.
//!
//! The declarations below must mirror the `csrc` headers exactly.

use std::marker::{PhantomData, PhantomPinned};

/// Opaque Jolt world: the physics system plus the collision-layer interfaces
/// it references (owned on the C++ side).
#[repr(C)]
pub struct JoltWorld {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque per-step scratch: temp allocator + job system thread pool.
#[repr(C)]
pub struct JoltStepper {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque body recipe for [`jolt_body_create`] (wraps
/// `JPH::BodyCreationSettings`).
#[repr(C)]
pub struct JoltBodyCreationSettings {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// Opaque collision shape (wraps a reference to a `JPH::Shape`).
///
/// Shapes are immutable and refcounted: one shape may back any number of
/// bodies. This handle owns one reference and bodies take their own, so
/// [`jolt_shape_destroy`] may be called while bodies still use the shape.
#[repr(C)]
pub struct JoltShape {
    _data: [u8; 0],
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

/// A Jolt body id (`JPH::BodyID::GetIndexAndSequenceNumber()`).
pub type JoltBodyId = u32;

/// Mirrors `JPH::EMotionType`. Static bodies live on the NON_MOVING collision
/// layer, kinematic and dynamic bodies on MOVING.
pub type JoltMotionType = u32;
pub const JOLT_MOTION_TYPE_STATIC: JoltMotionType = 0;
pub const JOLT_MOTION_TYPE_KINEMATIC: JoltMotionType = 1;
pub const JOLT_MOTION_TYPE_DYNAMIC: JoltMotionType = 2;

/// Bitmask of the degrees of freedom a dynamic body may use (mirrors
/// `JPH::EAllowedDOFs`). A value of 0 is invalid: use a static body instead.
pub type JoltAllowedDofs = u32;
pub const JOLT_ALLOWED_DOFS_TRANSLATION_X: JoltAllowedDofs = 1 << 0;
pub const JOLT_ALLOWED_DOFS_TRANSLATION_Y: JoltAllowedDofs = 1 << 1;
pub const JOLT_ALLOWED_DOFS_TRANSLATION_Z: JoltAllowedDofs = 1 << 2;
pub const JOLT_ALLOWED_DOFS_ROTATION_X: JoltAllowedDofs = 1 << 3;
pub const JOLT_ALLOWED_DOFS_ROTATION_Y: JoltAllowedDofs = 1 << 4;
pub const JOLT_ALLOWED_DOFS_ROTATION_Z: JoltAllowedDofs = 1 << 5;
pub const JOLT_ALLOWED_DOFS_ALL: JoltAllowedDofs = 0x3f;

/// Mirrors the `JoltGroundState` constants in `body.h`.
pub type JoltGroundState = u32;
pub const JOLT_GROUND_STATE_ON_GROUND: JoltGroundState = 0;
pub const JOLT_GROUND_STATE_ON_STEEP_GROUND: JoltGroundState = 1;
pub const JOLT_GROUND_STATE_IN_AIR: JoltGroundState = 2;

/// The closest ground found by [`jolt_body_probe_ground`]. All fields besides
/// `state` are only valid when `state != IN_AIR`; `velocity` is the ground
/// body's velocity at the contact point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JoltGroundProbeResult {
    pub state: JoltGroundState,
    pub body: JoltBodyId,
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub velocity: [f32; 3],
}

/// The closest hit of a raycast (mirrors `JoltRayHit` in `ray.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JoltRayHit {
    /// The body that was hit.
    pub body: JoltBodyId,
    /// Hit distance as a fraction `[0, 1]` of the ray's direction vector.
    pub fraction: f32,
    /// World-space surface normal at the hit point (xyz).
    pub normal: [f32; 3],
}

extern "C" {
    /// Process-global one-time Jolt setup. Thread-safe and idempotent;
    /// `jolt_world_create` calls it implicitly.
    pub fn jolt_global_init();

    /// Creates a physics world with Jolt's default gravity (0, -9.81, 0).
    /// `num_body_mutexes` may be 0 to let Jolt pick a default.
    pub fn jolt_world_create(
        max_bodies: u32,
        num_body_mutexes: u32,
        max_body_pairs: u32,
        max_contact_constraints: u32,
    ) -> *mut JoltWorld;

    pub fn jolt_world_destroy(world: *mut JoltWorld);

    /// Creates the step scratch. The thread pool sizes itself to the machine.
    pub fn jolt_stepper_create(temp_allocator_bytes: u32) -> *mut JoltStepper;

    pub fn jolt_stepper_destroy(stepper: *mut JoltStepper);

    /// Advances the world. Returns `JPH::EPhysicsUpdateError` bits (0 = ok).
    pub fn jolt_world_step(
        world: *mut JoltWorld,
        stepper: *mut JoltStepper,
        delta_time: f32,
        collision_steps: i32,
    ) -> u32;

    /// Creates settings at the origin with an identity rotation and static
    /// motion; a shape must be set before the settings are used.
    pub fn jolt_body_creation_settings_create() -> *mut JoltBodyCreationSettings;

    pub fn jolt_body_creation_settings_destroy(settings: *mut JoltBodyCreationSettings);

    pub fn jolt_body_creation_settings_set_position(
        settings: *mut JoltBodyCreationSettings,
        position: *const f32,
    );

    pub fn jolt_body_creation_settings_set_rotation(
        settings: *mut JoltBodyCreationSettings,
        rotation: *const f32,
    );

    pub fn jolt_body_creation_settings_set_motion_type(
        settings: *mut JoltBodyCreationSettings,
        motion_type: JoltMotionType,
    );

    /// Only meaningful for dynamic bodies (e.g. lock the rotation DOFs to
    /// keep a player capsule upright). Defaults to all.
    pub fn jolt_body_creation_settings_set_allowed_dofs(
        settings: *mut JoltBodyCreationSettings,
        allowed_dofs: JoltAllowedDofs,
    );

    /// `density` (kg/m³) is the density of the shape's volume, from which
    /// Jolt derives a dynamic body's mass; it has no effect on static bodies.
    pub fn jolt_create_sphere_shape(radius: f32, density: f32) -> *mut JoltShape;

    pub fn jolt_create_box_shape(half_extents: *const f32, density: f32) -> *mut JoltShape;

    /// A capsule with total height `2 * (half_height + radius)`: a cylinder
    /// of `2 * half_height` capped by hemispheres of `radius`, along the
    /// local Y axis.
    pub fn jolt_create_capsule_shape(half_height: f32, radius: f32, density: f32)
        -> *mut JoltShape;

    /// A triangle mesh. `vertices` is `vertex_count` xyz triples; `indices`
    /// is `index_count` vertex indices, three per triangle, wound
    /// counter-clockwise. Triangles are single sided for simulation. Returns
    /// null if Jolt rejected the mesh.
    ///
    /// Takes no density: a mesh need not form a closed hull, so it has no
    /// volume to derive a mass from, and Jolt requires mesh-shaped bodies to
    /// be static.
    pub fn jolt_create_mesh_shape(
        vertices: *const f32,
        vertex_count: u32,
        indices: *const u32,
        index_count: u32,
    ) -> *mut JoltShape;

    /// Releases the handle's reference. Bodies already created from the shape
    /// keep theirs and stay valid.
    pub fn jolt_shape_destroy(shape: *mut JoltShape);

    /// The shape's axis-aligned bounds in its own local space, before any body
    /// position, rotation or scale.
    pub fn jolt_shape_get_local_bounds(
        shape: *const JoltShape,
        out_min: *mut f32,
        out_max: *mut f32,
    );

    /// Sets the shape the body is built from. The settings take their own
    /// reference, so `shape` may be destroyed straight afterwards or reused
    /// for any number of other bodies.
    pub fn jolt_body_creation_settings_set_shape(
        settings: *mut JoltBodyCreationSettings,
        shape: *const JoltShape,
    );

    /// Offsets the shape's geometry relative to the body origin (e.g. lift a
    /// capsule so the origin sits at its bottom). Composes with
    /// [`jolt_body_creation_settings_set_shape`] in any call order.
    pub fn jolt_body_creation_settings_set_shape_offset(
        settings: *mut JoltBodyCreationSettings,
        offset: *const f32,
    );

    /// Scales the shape's geometry, so one shape can back bodies of different
    /// sizes (an entity's `Transform` scale). Applied inside the offset,
    /// which therefore stays in unscaled body space.
    ///
    /// Non-uniform and negative scales are supported; a mirrored scale keeps
    /// mesh triangles wound correctly. A component of zero is invalid and
    /// leaves the shape unscaled.
    pub fn jolt_body_creation_settings_set_shape_scale(
        settings: *mut JoltBodyCreationSettings,
        scale: *const f32,
    );

    /// Creates a body from `settings` and adds it to the simulation, active
    /// unless static. The settings remain owned by the caller and can be
    /// reused.
    pub fn jolt_body_create(
        world: *mut JoltWorld,
        settings: *const JoltBodyCreationSettings,
    ) -> JoltBodyId;

    /// Removes a body from the simulation and destroys it, waking any bodies
    /// that were touching it (they would otherwise sleep in mid-air). The id
    /// is invalid afterwards.
    pub fn jolt_body_destroy(world: *mut JoltWorld, body: JoltBodyId);

    /// Collides `body`'s shape against the world (ignoring `body` itself) and
    /// reports the most upward-facing contact within `max_separation` below
    /// the shape. Contacts steeper than `max_slope_angle` (radians from
    /// horizontal) report `ON_STEEP_GROUND`.
    pub fn jolt_body_probe_ground(
        world: *const JoltWorld,
        body: JoltBodyId,
        max_separation: f32,
        max_slope_angle: f32,
        out_result: *mut JoltGroundProbeResult,
    );

    /// Setting a non-zero velocity also wakes the body: `SetLinearVelocity`
    /// alone leaves a sleeping body asleep.
    pub fn jolt_body_set_linear_velocity(
        world: *mut JoltWorld,
        body: JoltBodyId,
        velocity: *const f32,
    );

    pub fn jolt_body_get_linear_velocity(
        world: *const JoltWorld,
        body: JoltBodyId,
        out_velocity: *mut f32,
    );

    /// Reads a body's world-space position (xyz) and rotation (xyzw).
    pub fn jolt_body_get_transform(
        world: *const JoltWorld,
        body: JoltBodyId,
        out_position: *mut f32,
        out_rotation: *mut f32,
    );

    /// Casts a ray from `origin` (xyz) along `direction` (xyz, whose length is
    /// the maximum cast distance) against every body except the `ignore_count`
    /// ids at `ignore`, writing the closest hit to `out_hit`. Returns `false`
    /// — leaving `out_hit` untouched — if nothing was hit.
    pub fn jolt_world_cast_ray(
        world: *const JoltWorld,
        origin: *const f32,
        direction: *const f32,
        ignore: *const JoltBodyId,
        ignore_count: u32,
        out_hit: *mut JoltRayHit,
    ) -> bool;

    pub fn jolt_body_add_impulse(world: *const JoltWorld, body: JoltBodyId, impulse: *const f32);

    pub fn jolt_body_add_impulse_at(
        world: *const JoltWorld,
        body: JoltBodyId,
        impulse: *const f32,
        position: *const f32,
    );

    pub fn jolt_body_add_force(world: *const JoltWorld, body: JoltBodyId, force: *const f32);

    pub fn jolt_body_add_force_at(
        world: *const JoltWorld,
        body: JoltBodyId,
        force: *const f32,
        position: *const f32,
    );

}
