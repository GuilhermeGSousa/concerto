//! The Jolt Physics backend, via our own in-repo `jolt-ffi` FFI bindings.
//! The native default; Jolt's C++ cannot target `wasm32-unknown-unknown`
//! (it requires thread primitives the toolchains do not provide), so web
//! builds use the Rapier backend instead.

use std::collections::HashSet;

use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::{Quat, Vec3};

use crate::aabb::Aabb;
use crate::backend::{
    MeshShapeCreationError, PhysicsBackend, RawContact, RawContactKind, RawGroundHit, RawRayHit,
    RawSensorContact, RawShapeHit, SensorContactKind,
};
use crate::body::BodyActivity;
use crate::collider::{Collider, ColliderOffset};
use crate::ragdoll::{BodyPose, JointDrive, PartShape, RagdollBuildError, RagdollSettings};
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
const SENSOR_CONTACT_BATCH: usize = 64;
const CONTACT_BATCH: usize = 64;

/// Owns the Jolt physics world: the body store, broad/narrow phases, and the
/// collision-layer interfaces it was initialised with (all held together on
/// the C++ side of the `jolt-ffi` shim).
pub struct JoltBackend {
    world: *mut concerto_jolt_ffi::JoltWorld,
    static_sensors: HashSet<concerto_jolt_ffi::JoltBodyId>,
    next_collision_group: u32,
}

/// Owns one reference to Jolt ragdoll settings, released on drop.
pub struct RagdollSettingsHandle(*mut concerto_jolt_ffi::JoltRagdollSettings);

// SAFETY: Jolt ragdoll settings are immutable once built and creating a ragdoll only reads them; the reference count they are released through is atomic.
unsafe impl Send for RagdollSettingsHandle {}
unsafe impl Sync for RagdollSettingsHandle {}

impl Drop for RagdollSettingsHandle {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `jolt_ragdoll_settings_create` and this is its only release.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_settings_destroy(self.0);
        }
    }
}

/// A Jolt ragdoll owned by the world it was created in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RagdollHandle(*mut concerto_jolt_ffi::JoltRagdoll);

// SAFETY: the handle is only an address; it is dereferenced through `JoltBackend`, whose exclusive access the scheduler guarantees.
unsafe impl Send for RagdollHandle {}
unsafe impl Sync for RagdollHandle {}

const POSE_FLOATS: usize = 7;

fn flatten_pose(pose: &[BodyPose]) -> Vec<f32> {
    let mut floats = Vec::with_capacity(pose.len() * POSE_FLOATS);
    for transform in pose {
        floats.extend(transform.translation.to_array());
        floats.extend(transform.rotation.to_array());
    }
    floats
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

impl JoltBackend {
    fn pose_fits(&self, ragdoll: RagdollHandle, pose: &[BodyPose]) -> bool {
        // SAFETY: `ragdoll` is a live ragdoll of this world.
        let parts = unsafe { concerto_jolt_ffi::jolt_ragdoll_part_count(ragdoll.0) };
        pose.len() == parts as usize
    }
}

impl PhysicsBackend for JoltBackend {
    type BodyHandle = concerto_jolt_ffi::JoltBodyId;
    type ShapeHandle = ShapeHandle;

    type Stepper = Stepper;

    type RagdollSettingsHandle = RagdollSettingsHandle;

    type RagdollHandle = RagdollHandle;

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

        Self {
            world,
            static_sensors: HashSet::new(),
            next_collision_group: 0,
        }
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

    fn step(&mut self, stepper: &mut Stepper, delta_time: f32, collision_steps: u32) {
        // SAFETY: both pointers are valid and exclusively borrowed.
        unsafe {
            concerto_jolt_ffi::jolt_world_step(
                self.world,
                stepper.stepper,
                delta_time,
                collision_steps.max(1) as i32,
            );
        }
    }

    fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
        sensor: bool,
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
        let id = unsafe {
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
            concerto_jolt_ffi::jolt_body_creation_settings_set_is_sensor(settings, sensor);
            let id = concerto_jolt_ffi::jolt_body_create(self.world, settings);
            concerto_jolt_ffi::jolt_body_creation_settings_destroy(settings);
            id
        };
        if sensor && rigid_body.is_none() {
            self.static_sensors.insert(id);
        }
        id
    }

    fn destroy_body(&mut self, body: Self::BodyHandle) {
        // SAFETY: `body` refers to a body created in this world and not yet
        // destroyed (`Collider`'s lifecycle guarantees one destroy per
        // create).
        unsafe {
            concerto_jolt_ffi::jolt_body_destroy(self.world, body);
        }
        self.static_sensors.remove(&body);
    }

    fn drain_sensor_contacts(&mut self, out: &mut Vec<RawSensorContact<Self::BodyHandle>>) {
        let mut batch = [concerto_jolt_ffi::JoltSensorContact {
            sensor: 0,
            other: 0,
            entered: 0,
        }; SENSOR_CONTACT_BATCH];
        loop {
            // SAFETY: `self.world` is a valid world and `batch` holds `SENSOR_CONTACT_BATCH` writable contacts.
            let count = unsafe {
                concerto_jolt_ffi::jolt_world_drain_sensor_contacts(
                    self.world,
                    batch.as_mut_ptr(),
                    SENSOR_CONTACT_BATCH as u32,
                )
            } as usize;
            out.extend(batch[..count].iter().map(|contact| RawSensorContact {
                sensor: contact.sensor,
                other: contact.other,
                kind: if contact.entered != 0 {
                    SensorContactKind::Enter
                } else {
                    SensorContactKind::Exit
                },
            }));
            if count < SENSOR_CONTACT_BATCH {
                break;
            }
        }
    }

    fn set_contact_reporting(&mut self, body: Self::BodyHandle, enabled: bool) {
        // SAFETY: `body` is a body in this world, and the world is not stepping while it is exclusively borrowed.
        unsafe {
            concerto_jolt_ffi::jolt_body_set_contact_reporting(self.world, body, enabled);
        }
    }

    fn drain_contacts(&mut self, out: &mut Vec<RawContact<Self::BodyHandle>>) {
        let mut batch = [concerto_jolt_ffi::JoltContact {
            body1: 0,
            body2: 0,
            kind: 0,
            point: [0.0; 3],
            normal: [0.0; 3],
            impulse: 0.0,
        }; CONTACT_BATCH];
        loop {
            // SAFETY: `self.world` is a valid world and `batch` holds `CONTACT_BATCH` writable contacts.
            let count = unsafe {
                concerto_jolt_ffi::jolt_world_drain_contacts(
                    self.world,
                    batch.as_mut_ptr(),
                    CONTACT_BATCH as u32,
                )
            } as usize;
            out.extend(batch[..count].iter().map(|contact| RawContact {
                first: contact.body1,
                second: contact.body2,
                kind: match contact.kind {
                    concerto_jolt_ffi::JOLT_CONTACT_START => RawContactKind::Start,
                    concerto_jolt_ffi::JOLT_CONTACT_END => RawContactKind::End,
                    _ => RawContactKind::Touch {
                        point: Vec3::from(contact.point),
                        normal: Vec3::from(contact.normal),
                        impulse: contact.impulse,
                    },
                },
            }));
            if count < CONTACT_BATCH {
                break;
            }
        }
    }

    fn body_activity(&self, body: Self::BodyHandle) -> BodyActivity {
        // SAFETY: `self.world` is a valid world; the shim answers static and inactive for an id it does not hold.
        let (motion_type, active) = unsafe {
            (
                concerto_jolt_ffi::jolt_body_get_motion_type(self.world, body),
                concerto_jolt_ffi::jolt_body_is_active(self.world, body),
            )
        };
        match motion_type {
            concerto_jolt_ffi::JOLT_MOTION_TYPE_STATIC => BodyActivity::Static,
            _ if !active => BodyActivity::Asleep,
            concerto_jolt_ffi::JOLT_MOTION_TYPE_KINEMATIC => BodyActivity::Kinematic,
            _ => BodyActivity::Dynamic,
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
        if self.static_sensors.contains(&body) {
            return;
        }
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

    fn cast_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        ignore: &[Self::BodyHandle],
    ) -> Option<RawRayHit<Self::BodyHandle>> {
        let origin_array = origin.to_array();
        let direction_array = direction.to_array();
        let mut hit = concerto_jolt_ffi::JoltRayHit {
            body: 0,
            fraction: 0.0,
            normal: [0.0; 3],
        };

        // SAFETY: the arrays are valid xyz triples, `ignore` holds `ignore.len()` ids, and `hit` is a valid out-buffer.
        let did_hit = unsafe {
            concerto_jolt_ffi::jolt_world_cast_ray(
                self.world,
                origin_array.as_ptr(),
                direction_array.as_ptr(),
                ignore.as_ptr(),
                ignore.len() as u32,
                &mut hit,
            )
        };

        did_hit.then(|| RawRayHit {
            body: hit.body,
            fraction: hit.fraction,
            normal: Vec3::from(hit.normal),
        })
    }

    fn cast_sphere(
        &self,
        origin: Vec3,
        radius: f32,
        direction: Vec3,
        ignore: &[Self::BodyHandle],
    ) -> Option<RawShapeHit<Self::BodyHandle>> {
        let origin_array = origin.to_array();
        let direction_array = direction.to_array();
        let mut hit = concerto_jolt_ffi::JoltShapeHit {
            body: 0,
            fraction: 0.0,
            point: [0.0; 3],
            normal: [0.0; 3],
        };

        // SAFETY: the arrays are valid xyz triples, `ignore` holds `ignore.len()` ids, and `hit` is a valid out-buffer.
        let did_hit = unsafe {
            concerto_jolt_ffi::jolt_world_cast_sphere(
                self.world,
                origin_array.as_ptr(),
                radius,
                direction_array.as_ptr(),
                ignore.as_ptr(),
                ignore.len() as u32,
                &mut hit,
            )
        };

        did_hit.then(|| RawShapeHit {
            body: hit.body,
            fraction: hit.fraction,
            point: Vec3::from(hit.point),
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

    fn build_ragdoll(
        settings: &RagdollSettings,
    ) -> Result<Self::RagdollSettingsHandle, RagdollBuildError> {
        let parts: Vec<concerto_jolt_ffi::JoltRagdollPart> = settings
            .parts
            .iter()
            .map(|part| {
                let (shape_kind, shape_size) = match part.shape {
                    PartShape::Sphere { radius } => (
                        concerto_jolt_ffi::JOLT_PART_SHAPE_SPHERE,
                        [radius, 0.0, 0.0],
                    ),
                    PartShape::Capsule {
                        half_height,
                        radius,
                    } => (
                        concerto_jolt_ffi::JOLT_PART_SHAPE_CAPSULE,
                        [half_height, radius, 0.0],
                    ),
                    PartShape::Box { half_extents } => (
                        concerto_jolt_ffi::JOLT_PART_SHAPE_BOX,
                        half_extents.to_array(),
                    ),
                };
                concerto_jolt_ffi::JoltRagdollPart {
                    parent: part.parent.map_or(-1, |parent| parent as i32),
                    position: part.pose.translation.to_array(),
                    rotation: part.pose.rotation.to_array(),
                    shape_kind,
                    shape_size,
                    shape_position: part.shape_offset.translation.to_array(),
                    shape_rotation: part.shape_offset.rotation.to_array(),
                    mass: part.mass,
                    twist_axis: part.joint.twist_axis.to_array(),
                    plane_axis: part.joint.plane_axis.to_array(),
                    normal_half_cone: part.joint.normal_half_cone,
                    plane_half_cone: part.joint.plane_half_cone,
                    twist_min: part.joint.twist_min,
                    twist_max: part.joint.twist_max,
                    max_friction_torque: part.joint.max_friction_torque,
                    motor_frequency: part.joint.motor.frequency,
                    motor_damping: part.joint.motor.damping,
                    motor_torque_limit: part.joint.motor.torque_limit,
                }
            })
            .collect();
        // SAFETY: `parts` holds `parts.len()` parts, read during the call only.
        let built = unsafe {
            concerto_jolt_ffi::jolt_ragdoll_settings_create(parts.as_ptr(), parts.len() as u32)
        };
        if built.is_null() {
            Err(RagdollBuildError::Rejected)
        } else {
            Ok(RagdollSettingsHandle(built))
        }
    }

    fn create_ragdoll(
        &mut self,
        built: &Self::RagdollSettingsHandle,
        pose: &[BodyPose],
    ) -> Option<Self::RagdollHandle> {
        // SAFETY: `built.0` is live settings.
        let parts = unsafe { concerto_jolt_ffi::jolt_ragdoll_settings_part_count(built.0) };
        if pose.len() != parts as usize {
            return None;
        }
        let floats = flatten_pose(pose);
        let collision_group = self.next_collision_group;
        self.next_collision_group = self.next_collision_group.wrapping_add(1);
        // SAFETY: the world and settings are live, and `floats` holds seven floats for each of the settings' parts.
        let ragdoll = unsafe {
            concerto_jolt_ffi::jolt_ragdoll_create(
                self.world,
                built.0,
                collision_group,
                floats.as_ptr(),
            )
        };
        (!ragdoll.is_null()).then_some(RagdollHandle(ragdoll))
    }

    fn destroy_ragdoll(&mut self, ragdoll: Self::RagdollHandle) {
        // SAFETY: `ragdoll` was created in this world and `PhysicsState` destroys each ragdoll once.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_destroy(self.world, ragdoll.0);
        }
    }

    fn ragdoll_bodies(&self, ragdoll: Self::RagdollHandle) -> Vec<Self::BodyHandle> {
        // SAFETY: `ragdoll` is a live ragdoll of this world, and each index is below its part count.
        unsafe {
            let parts = concerto_jolt_ffi::jolt_ragdoll_part_count(ragdoll.0);
            (0..parts)
                .map(|part| concerto_jolt_ffi::jolt_ragdoll_part_body(ragdoll.0, part))
                .collect()
        }
    }

    fn ragdoll_pose(&self, ragdoll: Self::RagdollHandle, out: &mut Vec<BodyPose>) {
        // SAFETY: `ragdoll` is a live ragdoll of this world, and `floats` holds seven floats for each of its parts.
        let floats = unsafe {
            let parts = concerto_jolt_ffi::jolt_ragdoll_part_count(ragdoll.0) as usize;
            let mut floats = vec![0.0f32; parts * POSE_FLOATS];
            concerto_jolt_ffi::jolt_ragdoll_get_pose(self.world, ragdoll.0, floats.as_mut_ptr());
            floats
        };
        out.clear();
        out.extend(floats.as_chunks::<POSE_FLOATS>().0.iter().map(|pose| {
            BodyPose::new(
                Vec3::new(pose[0], pose[1], pose[2]),
                Quat::from_xyzw(pose[3], pose[4], pose[5], pose[6]),
            )
        }));
    }

    fn set_ragdoll_pose(&mut self, ragdoll: Self::RagdollHandle, pose: &[BodyPose]) {
        // SAFETY: `ragdoll` is a live ragdoll of this world.
        let parts = unsafe { concerto_jolt_ffi::jolt_ragdoll_part_count(ragdoll.0) };
        if pose.len() != parts as usize {
            return;
        }
        let floats = flatten_pose(pose);
        // SAFETY: `floats` holds seven floats for each of the ragdoll's parts.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_set_pose(ragdoll.0, floats.as_ptr());
        }
    }

    fn drive_ragdoll_kinematic(
        &mut self,
        ragdoll: Self::RagdollHandle,
        pose: &[BodyPose],
        delta_time: f32,
    ) {
        if !self.pose_fits(ragdoll, pose) {
            return;
        }
        let floats = flatten_pose(pose);
        // SAFETY: `ragdoll` is a live ragdoll of this world and `floats` holds seven floats for each of its parts.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_drive_kinematic(ragdoll.0, floats.as_ptr(), delta_time);
        }
    }

    fn drive_ragdoll_motors(&mut self, ragdoll: Self::RagdollHandle, pose: &[BodyPose]) {
        if !self.pose_fits(ragdoll, pose) {
            return;
        }
        let floats = flatten_pose(pose);
        // SAFETY: `ragdoll` is a live ragdoll of this world and `floats` holds seven floats for each of its parts.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_drive_motors(ragdoll.0, floats.as_ptr());
        }
    }

    fn set_ragdoll_joint_drive(
        &mut self,
        ragdoll: Self::RagdollHandle,
        part: usize,
        drive: JointDrive,
    ) {
        let (enabled, rotation, motor) = match drive {
            JointDrive::Off => (false, Quat::IDENTITY, Default::default()),
            JointDrive::Target(target) => (true, target.rotation, target.motor),
        };
        let rotation = rotation.to_array();
        // SAFETY: `ragdoll` is a live ragdoll of this world, the shim ignores a part it does not have, and `rotation` is a valid xyzw quaternion.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_set_joint_motor(
                ragdoll.0,
                part as u32,
                enabled,
                rotation.as_ptr(),
                motor.frequency,
                motor.damping,
                motor.torque_limit,
            );
        }
    }

    fn set_ragdoll_part_motion(
        &mut self,
        ragdoll: Self::RagdollHandle,
        part: usize,
        motion: MotionType,
    ) {
        let motion_type = match motion {
            MotionType::Dynamic => concerto_jolt_ffi::JOLT_MOTION_TYPE_DYNAMIC,
            MotionType::Kinematic => concerto_jolt_ffi::JOLT_MOTION_TYPE_KINEMATIC,
        };
        // SAFETY: `ragdoll` is a live ragdoll of this world and the shim ignores a part it does not have.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_set_part_motion_type(
                self.world,
                ragdoll.0,
                part as u32,
                motion_type,
            );
        }
    }

    fn ragdoll_joint_motor_impulse(&self, ragdoll: Self::RagdollHandle, part: usize) -> Vec3 {
        let mut impulse = [0.0f32; 3];
        // SAFETY: `ragdoll` is a live ragdoll of this world, the shim answers zero for a part it does not have, and `impulse` is a valid xyz triple.
        unsafe {
            concerto_jolt_ffi::jolt_ragdoll_get_joint_motor_impulse(
                ragdoll.0,
                part as u32,
                impulse.as_mut_ptr(),
            );
        }
        Vec3::from(impulse)
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
