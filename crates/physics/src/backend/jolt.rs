//! The Jolt Physics backend, via our own in-repo `jolt-ffi` FFI bindings.
//! The native default; Jolt's C++ cannot target `wasm32-unknown-unknown`
//! (it requires thread primitives the toolchains do not provide), so web
//! builds use the Rapier backend instead.

use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::{Quat, Vec3};

use crate::aabb::Aabb;
use crate::backend::{MeshShapeCreationError, PhysicsBackend, RawGroundHit, RawRayHit};
use crate::collider::{Collider, ColliderOffset};
use crate::rigid_body::{AllowedDofs, MotionType, RigidBody};

const MAX_BODIES: u32 = 10_240;
const NUM_BODY_MUTEXES: u32 = 0; // 0 = let Jolt pick a default
                                 // These bound the per-step scratch Jolt allocates from the temp allocator
                                 // (see `new_stepper`). Keep them in step with that allocator's size.
const MAX_BODY_PAIRS: u32 = 10_240;
const MAX_CONTACT_CONSTRAINTS: u32 = 10_240;
// Shapes are shared between bodies, so density can no longer come from an
// individual body's `RigidBody`. See the note on `create_sphere_shape`.
const SHAPE_DENSITY: f32 = 1000.0; // water

/// Owns the Jolt physics world: the body store, broad/narrow phases, and the
/// collision-layer interfaces it was initialised with (all held together on
/// the C++ side of the `jolt-ffi` shim).
pub struct JoltBackend {
    world: *mut concerto_jolt_ffi::JoltWorld,
}

// SAFETY: `JoltBackend` holds a raw pointer into Jolt, which is not inherently
// `Send`/`Sync`. The ECS requires resources to be `Send + Sync`, and the
// scheduler upholds the actual safety contract: a `ResMut<PhysicsState>` is
// granted exclusive access, so the world is never touched from two threads at
// once.
unsafe impl Send for JoltBackend {}
unsafe impl Sync for JoltBackend {}

/// Owns one reference to a Jolt collision shape, released on drop.
///
/// Shapes are immutable and refcounted, so a single handle can back any
/// number of bodies; each body takes its own reference, so dropping this
/// never invalidates a shape still in use.
pub struct ShapeHandle(*mut concerto_jolt_ffi::JoltShape);

// SAFETY: a Jolt shape is immutable once built, so there is nothing to race
// on when it is read from several threads. The only mutable state is
// `JPH::RefTarget`'s reference count, which is an atomic with release/acquire
// ordering, making the drop below safe from any thread.
unsafe impl Send for ShapeHandle {}
unsafe impl Sync for ShapeHandle {}

impl ShapeHandle {
    fn as_ptr(&self) -> *const concerto_jolt_ffi::JoltShape {
        self.0
    }
}

impl Drop for ShapeHandle {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a `jolt_create_*_shape` call, which
        // transferred one reference, and this is the only release of it.
        unsafe {
            concerto_jolt_ffi::jolt_shape_destroy(self.0);
        }
    }
}

/// Per-step scratch: a temporary allocator and a job system thread pool
/// (owned together on the C++ side of the `jolt-ffi` shim).
pub struct Stepper {
    stepper: *mut concerto_jolt_ffi::JoltStepper,
}

// SAFETY: like `JoltBackend`, this holds a raw Jolt pointer; the scheduler
// guarantees exclusive `ResMut` access, so the pointer is never used from two
// threads concurrently.
unsafe impl Send for Stepper {}
unsafe impl Sync for Stepper {}

fn to_jolt_dofs(dofs: AllowedDofs) -> concerto_jolt_ffi::JoltAllowedDofs {
    let mut jolt_dofs = 0;
    for (axis, jolt_axis) in [
        (
            AllowedDofs::TRANSLATION_X,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_TRANSLATION_X,
        ),
        (
            AllowedDofs::TRANSLATION_Y,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_TRANSLATION_Y,
        ),
        (
            AllowedDofs::TRANSLATION_Z,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_TRANSLATION_Z,
        ),
        (
            AllowedDofs::ROTATION_X,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_ROTATION_X,
        ),
        (
            AllowedDofs::ROTATION_Y,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_ROTATION_Y,
        ),
        (
            AllowedDofs::ROTATION_Z,
            concerto_jolt_ffi::JOLT_ALLOWED_DOFS_ROTATION_Z,
        ),
    ] {
        if dofs.contains(axis) {
            jolt_dofs |= jolt_axis;
        }
    }
    jolt_dofs
}

impl PhysicsBackend for JoltBackend {
    type BodyHandle = concerto_jolt_ffi::JoltBodyId;
    type ShapeHandle = ShapeHandle;

    type Stepper = Stepper;

    fn new() -> Self {
        // SAFETY: `jolt_world_create` performs Jolt's process-global init
        // (idempotent) and returns an owned world, freed in `Drop`.
        let world = unsafe {
            concerto_jolt_ffi::jolt_world_create(
                MAX_BODIES,
                NUM_BODY_MUTEXES,
                MAX_BODY_PAIRS,
                MAX_CONTACT_CONSTRAINTS,
            )
        };

        Self { world }
    }

    fn new_stepper() -> Stepper {
        // SAFETY: `jolt_stepper_create` returns an owned stepper which is
        // freed in `Drop`.
        //
        // 32 MiB of scratch, sized to comfortably hold the body-pair and
        // contact-constraint buffers Jolt allocates each step for the maxima
        // configured in `new`.
        let stepper = unsafe { concerto_jolt_ffi::jolt_stepper_create(32 * 1024 * 1024) };

        Stepper { stepper }
    }

    fn step(&mut self, stepper: &mut Stepper, delta_time: f32) {
        // SAFETY: both pointers are valid and exclusively borrowed.
        unsafe {
            concerto_jolt_ffi::jolt_world_step(self.world, stepper.stepper, delta_time, 1);
        }
    }

    fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
    ) -> Self::BodyHandle {
        let position = transform.translation.to_array();
        let rotation = transform.rotation.to_array();
        let scale = transform.scale.to_array();
        let motion_type = match rigid_body {
            Some(rb) => match rb.motion_type {
                MotionType::Dynamic => concerto_jolt_ffi::JOLT_MOTION_TYPE_DYNAMIC,
                MotionType::Kinematic => concerto_jolt_ffi::JOLT_MOTION_TYPE_KINEMATIC,
            },
            None => concerto_jolt_ffi::JOLT_MOTION_TYPE_STATIC,
        };
        // SAFETY: `self.world` is a valid world; the position/half-extent
        // arrays are valid xyz triples and the rotation a valid xyzw
        // quaternion. The settings are created, used, and destroyed within
        // this call.
        unsafe {
            let settings = concerto_jolt_ffi::jolt_body_creation_settings_create();
            concerto_jolt_ffi::jolt_body_creation_settings_set_position(
                settings,
                position.as_ptr(),
            );
            concerto_jolt_ffi::jolt_body_creation_settings_set_rotation(
                settings,
                rotation.as_ptr(),
            );
            concerto_jolt_ffi::jolt_body_creation_settings_set_motion_type(settings, motion_type);
            if let Some(rigid_body) = rigid_body {
                concerto_jolt_ffi::jolt_body_creation_settings_set_allowed_dofs(
                    settings,
                    to_jolt_dofs(rigid_body.allowed_dofs),
                );
            }
            if let Some(offset) = offset {
                let offset = offset.0.to_array();
                concerto_jolt_ffi::jolt_body_creation_settings_set_shape_offset(
                    settings,
                    offset.as_ptr(),
                );
            }
            // Shapes are shared between bodies, so an entity's scale cannot be
            // baked into the geometry: it goes on the body instead.
            concerto_jolt_ffi::jolt_body_creation_settings_set_shape_scale(
                settings,
                scale.as_ptr(),
            );
            // The body takes its own reference, so the collider keeps owning
            // the shape and may share it with any number of other bodies.
            concerto_jolt_ffi::jolt_body_creation_settings_set_shape(
                settings,
                collider.shape().0.as_ptr(),
            );
            let id = concerto_jolt_ffi::jolt_body_create(self.world, settings);
            concerto_jolt_ffi::jolt_body_creation_settings_destroy(settings);
            id
        }
    }

    fn destroy_body(&mut self, body: Self::BodyHandle) {
        // SAFETY: `body` refers to a body created in this world and not yet
        // destroyed (`Collider`'s lifecycle guarantees one destroy per
        // create).
        unsafe {
            concerto_jolt_ffi::jolt_body_destroy(self.world, body);
        }
    }

    fn body_transform(&self, body: Self::BodyHandle) -> Transform {
        let mut position = [0.0f32; 3];
        let mut rotation = [0.0f32; 4];
        // SAFETY: `body` is a valid body id within this world, and the output
        // buffers have the sizes the shim writes (xyz and xyzw).
        unsafe {
            concerto_jolt_ffi::jolt_body_get_transform(
                self.world,
                body,
                position.as_mut_ptr(),
                rotation.as_mut_ptr(),
            );
        }

        Transform::from_translation_rotation(Vec3::from(position), Quat::from_array(rotation))
    }

    fn linear_velocity(&self, body: Self::BodyHandle) -> Vec3 {
        let mut velocity = [0.0f32; 3];
        // SAFETY: `body` is a body in this world and the out-buffer is a
        // valid xyz triple.
        unsafe {
            concerto_jolt_ffi::jolt_body_get_linear_velocity(
                self.world,
                body,
                velocity.as_mut_ptr(),
            );
        }
        Vec3::from(velocity)
    }

    fn set_linear_velocity(&mut self, body: Self::BodyHandle, velocity: Vec3) {
        let velocity = velocity.to_array();
        // SAFETY: `body` is a body in this world; `velocity` is a valid xyz
        // triple.
        unsafe {
            concerto_jolt_ffi::jolt_body_set_linear_velocity(self.world, body, velocity.as_ptr());
        }
    }

    fn add_impulse(&mut self, body: Self::BodyHandle, impulse: Vec3) {
        // SAFETY: `body` is a body in this world; the array a valid triple.
        unsafe {
            concerto_jolt_ffi::jolt_body_add_impulse(self.world, body, impulse.to_array().as_ptr());
        }
    }

    fn add_impulse_at(&mut self, body: Self::BodyHandle, impulse: Vec3, position: Vec3) {
        // SAFETY: `body` is a body in this world; the arrays valid triples.
        unsafe {
            concerto_jolt_ffi::jolt_body_add_impulse_at(
                self.world,
                body,
                impulse.to_array().as_ptr(),
                position.to_array().as_ptr(),
            );
        }
    }

    fn add_force(&mut self, body: Self::BodyHandle, force: Vec3) {
        // SAFETY: `body` is a body in this world; the array a valid triple.
        unsafe {
            concerto_jolt_ffi::jolt_body_add_force(self.world, body, force.to_array().as_ptr());
        }
    }

    fn add_force_at(&mut self, body: Self::BodyHandle, force: Vec3, position: Vec3) {
        // SAFETY: `body` is a body in this world; the arrays valid triples.
        unsafe {
            concerto_jolt_ffi::jolt_body_add_force_at(
                self.world,
                body,
                force.to_array().as_ptr(),
                position.to_array().as_ptr(),
            );
        }
    }

    fn cast_ray(&self, origin: Vec3, direction: Vec3) -> Option<RawRayHit<Self::BodyHandle>> {
        let origin_array = origin.to_array();
        let direction_array = direction.to_array();
        let mut hit = concerto_jolt_ffi::JoltRayHit {
            body: 0,
            fraction: 0.0,
            normal: [0.0; 3],
        };

        // SAFETY: the input arrays are valid xyz triples and `hit` is a valid
        // out-buffer; the shim only writes it when returning true.
        let did_hit = unsafe {
            concerto_jolt_ffi::jolt_world_cast_ray(
                self.world,
                origin_array.as_ptr(),
                direction_array.as_ptr(),
                &mut hit,
            )
        };

        did_hit.then(|| RawRayHit {
            body: hit.body,
            fraction: hit.fraction,
            normal: Vec3::from(hit.normal),
        })
    }

    fn probe_ground(
        &self,
        body: Self::BodyHandle,
        max_separation: f32,
    ) -> Option<RawGroundHit<Self::BodyHandle>> {
        let mut result = concerto_jolt_ffi::JoltGroundProbeResult {
            state: concerto_jolt_ffi::JOLT_GROUND_STATE_IN_AIR,
            body: 0,
            position: [0.0; 3],
            normal: [0.0; 3],
            velocity: [0.0; 3],
        };
        // SAFETY: `body` is a body in this world and `result` is a valid
        // out-buffer.
        //
        // The shim also classifies the contact against a slope angle, but
        // that now happens in the facade — pass 0 and use only the
        // contact-vs-in-air distinction.
        unsafe {
            concerto_jolt_ffi::jolt_body_probe_ground(
                self.world,
                body,
                max_separation,
                0.0,
                &mut result,
            );
        }

        (result.state != concerto_jolt_ffi::JOLT_GROUND_STATE_IN_AIR).then(|| RawGroundHit {
            body: result.body,
            point: Vec3::from(result.position),
            normal: Vec3::from(result.normal),
            velocity: Vec3::from(result.velocity),
        })
    }

    fn shape_local_aabb(shape: &Self::ShapeHandle) -> Aabb {
        let mut min = [0.0f32; 3];
        let mut max = [0.0f32; 3];
        // SAFETY: `shape` is a live handle; both out-buffers are valid xyz
        // triples the shim always fills.
        unsafe {
            concerto_jolt_ffi::jolt_shape_get_local_bounds(
                shape.as_ptr(),
                min.as_mut_ptr(),
                max.as_mut_ptr(),
            );
        }
        Aabb::new(min.into(), max.into())
    }

    fn create_sphere_shape(radius: f32) -> Self::ShapeHandle {
        // SAFETY: the constructor allocates a shape and transfers one
        // reference, released by `ShapeHandle::drop`.
        ShapeHandle(unsafe { concerto_jolt_ffi::jolt_create_sphere_shape(radius, SHAPE_DENSITY) })
    }

    fn create_cuboid_shape(width: f32, height: f32, length: f32) -> Self::ShapeHandle {
        let half_extents = [width, height, length];
        // SAFETY: `half_extents` is a valid xyz triple, read during the call
        // only; the returned reference is released by `ShapeHandle::drop`.
        ShapeHandle(unsafe {
            concerto_jolt_ffi::jolt_create_box_shape(half_extents.as_ptr(), SHAPE_DENSITY)
        })
    }

    fn create_capsule_shape(half_height: f32, radius: f32) -> Self::ShapeHandle {
        // SAFETY: as `create_sphere_shape`.
        ShapeHandle(unsafe {
            concerto_jolt_ffi::jolt_create_capsule_shape(half_height, radius, SHAPE_DENSITY)
        })
    }

    fn create_shape_from_mesh(mesh: &Mesh) -> Result<Self::ShapeHandle, MeshShapeCreationError> {
        // `Vertex` interleaves normals, UVs and skinning weights with the
        // positions, so the positions have to be packed before Jolt can read
        // them as xyz triples.
        let positions: Vec<f32> = mesh
            .vertices
            .iter()
            .flat_map(|vertex| vertex.pos_coords)
            .collect();

        // SAFETY: `positions` holds `mesh.vertices.len()` xyz triples and
        // `mesh.indices` `len()` indices; both are read during the call only.
        let shape = unsafe {
            concerto_jolt_ffi::jolt_create_mesh_shape(
                positions.as_ptr(),
                mesh.vertices.len() as u32,
                mesh.indices.as_ptr(),
                mesh.indices.len() as u32,
            )
        };

        if !shape.is_null() {
            Ok(ShapeHandle(shape))
        } else {
            Err(MeshShapeCreationError)
        }
    }
}

impl Drop for JoltBackend {
    fn drop(&mut self) {
        // SAFETY: `self.world` was created in `new` and is destroyed exactly
        // once here.
        unsafe {
            concerto_jolt_ffi::jolt_world_destroy(self.world);
        }
    }
}

impl Drop for Stepper {
    fn drop(&mut self) {
        // SAFETY: the stepper was created in `new_stepper` and is freed once
        // here.
        unsafe {
            concerto_jolt_ffi::jolt_stepper_destroy(self.stepper);
        }
    }
}
