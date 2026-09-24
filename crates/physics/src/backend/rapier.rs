//! The Rapier physics backend. Pure Rust, so unlike Jolt it compiles to
//! `wasm32-unknown-unknown` — it is the backend of every web build. Native
//! builds can opt into it with the `force-rapier` feature to test and debug
//! it without a browser.
//!
//! Rapier bundles its own `glam` (a newer major than the engine's), so
//! engine↔Rapier math conversions go through arrays at this boundary, the
//! same way the Jolt backend converts at the FFI boundary.

use std::sync::Arc;

use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::{Quat, Vec3};
use log::warn;
use rapier3d::parry::bounding_volume::BoundingVolume;
use rapier3d::parry::query::{ContactManifold, DefaultQueryDispatcher, PersistentQueryDispatcher};
use rapier3d::prelude::{
    BroadPhaseBvh, CCDSolver, ColliderBuilder, ColliderSet, ImpulseJointSet, IntegrationParameters,
    IslandManager, LockedAxes, MultibodyJointSet, NarrowPhase, Ray, RigidBodyBuilder,
    RigidBodyHandle, RigidBodySet, SharedShape,
};

use crate::backend::{Aabb, MeshShapeCreationError, PhysicsBackend, RawGroundHit, RawRayHit};
use crate::collider::{Collider, ColliderOffset};
use crate::rigid_body::{AllowedDofs, MotionType, RigidBody};

/// Jolt's default gravity, so both backends simulate the same world.
const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// Subdivisions used when a non-uniform scale has to turn a round shape into
/// a polygonal approximation. Shapes that scale exactly — cuboids, triangle
/// meshes, uniformly scaled balls — ignore it.
const SCALE_SUBDIVISIONS: u32 = 20;

/// Applies an entity's `Transform` scale to its shape.
///
/// Unlike Jolt, which wraps the shape in a `ScaledShape` and keeps sharing the
/// original, Rapier scales by rebuilding: the result is a new shape that does
/// not share the source's allocation or its triangle BVH. Scaled colliders
/// therefore cost more here than on the Jolt backend.
fn scaled_shape(shape: &SharedShape, scale: Vec3) -> SharedShape {
    if (scale - Vec3::ONE).abs().max_element() <= 1e-6 {
        return shape.clone();
    }
    match shape.scale_dyn(to_rapier_vec(scale), SCALE_SUBDIVISIONS) {
        Some(scaled) => SharedShape(Arc::from(scaled)),
        // Only shapes that cannot represent the scale at all (e.g. a
        // heightfield mirrored on an axis); better an unscaled collider than
        // none.
        None => {
            warn!("collider shape cannot be scaled by {scale:?}, using it unscaled");
            shape.clone()
        }
    }
}

fn to_rapier_vec(v: Vec3) -> rapier3d::math::Vector {
    rapier3d::math::Vector::from_array(v.to_array())
}

fn from_rapier_vec(v: rapier3d::math::Vector) -> Vec3 {
    Vec3::from_array(v.to_array())
}

fn to_rapier_pose(transform: &Transform) -> rapier3d::math::Pose {
    rapier3d::math::Pose::from_parts(
        to_rapier_vec(transform.translation),
        rapier3d::glamx::Rot3::from_array(transform.rotation.to_array()),
    )
}

/// Locks every degree of freedom `dofs` does not allow.
fn to_locked_axes(dofs: AllowedDofs) -> LockedAxes {
    let mut locked = LockedAxes::empty();
    for (axis, lock) in [
        (AllowedDofs::TRANSLATION_X, LockedAxes::TRANSLATION_LOCKED_X),
        (AllowedDofs::TRANSLATION_Y, LockedAxes::TRANSLATION_LOCKED_Y),
        (AllowedDofs::TRANSLATION_Z, LockedAxes::TRANSLATION_LOCKED_Z),
        (AllowedDofs::ROTATION_X, LockedAxes::ROTATION_LOCKED_X),
        (AllowedDofs::ROTATION_Y, LockedAxes::ROTATION_LOCKED_Y),
        (AllowedDofs::ROTATION_Z, LockedAxes::ROTATION_LOCKED_Z),
    ] {
        if !dofs.contains(axis) {
            locked |= lock;
        }
    }
    locked
}

/// Owns the Rapier physics world: the body and collider sets plus the
/// broad/narrow phases, islands, joints, and CCD state stepping needs.
pub struct RapierBackend {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
}

/// Per-step scratch: Rapier's execution pipeline and the integration
/// parameters fed to it each step.
pub struct Stepper {
    pipeline: rapier3d::pipeline::PhysicsPipeline,
    integration_parameters: IntegrationParameters,
}

// Scene queries (`cast_ray`, `probe_ground`) iterate the collider set
// directly instead of going through Rapier's BVH-backed `QueryPipeline`.
// The BVH is only refreshed inside `PhysicsPipeline::step`, so it knows
// nothing about bodies created since the last step (Jolt, by contrast,
// indexes bodies into its broad phase at creation time — and the engine
// raycasts against freshly spawned worlds, e.g. in tests). A linear scan is
// always correct, and at this engine's body counts it is not a bottleneck;
// revisit if profiling ever says otherwise.

impl PhysicsBackend for RapierBackend {
    type BodyHandle = RigidBodyHandle;

    type ShapeHandle = SharedShape;

    type Stepper = Stepper;

    fn new() -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
        }
    }

    fn new_stepper() -> Stepper {
        Stepper {
            pipeline: rapier3d::pipeline::PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
        }
    }

    fn step(&mut self, stepper: &mut Stepper, delta_time: f32) {
        stepper.integration_parameters.dt = delta_time;
        stepper.pipeline.step(
            rapier3d::math::Vector::from_array(GRAVITY),
            &stepper.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &(),
            &(),
        );
    }

    fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
    ) -> Self::BodyHandle {
        let mut body_builder = match rigid_body {
            None => RigidBodyBuilder::fixed(),
            Some(rb) => match rb.motion_type {
                MotionType::Dynamic => RigidBodyBuilder::dynamic(),
                MotionType::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
            },
        }
        // Only the translation and rotation: the scale belongs to the shape.
        .pose(to_rapier_pose(transform));
        if let Some(rigid_body) = rigid_body {
            body_builder = body_builder.locked_axes(to_locked_axes(rigid_body.allowed_dofs));
        }
        let handle = self.bodies.insert(body_builder);

        // Density is unused for static bodies; any sane value works here.
        let density = rigid_body.map_or(1000.0, |rigid_body| rigid_body.density);
        let mut collider_builder =
            ColliderBuilder::new(scaled_shape(&collider.shape().0, transform.scale))
                .density(density);
        if let Some(offset) = offset {
            // The offset is the collider's pose relative to the body, so it
            // shifts the geometry without showing up in `body_transform`.
            collider_builder = collider_builder.translation(to_rapier_vec(offset.0));
        }
        self.colliders
            .insert_with_parent(collider_builder, handle, &mut self.bodies);

        handle
    }

    fn shape_local_aabb(shape: &Self::ShapeHandle) -> Aabb {
        let aabb = shape.compute_local_aabb();

        Aabb::new(from_rapier_vec(aabb.mins), from_rapier_vec(aabb.maxs))
    }

    fn create_sphere_shape(radius: f32) -> Self::ShapeHandle {
        SharedShape::ball(radius)
    }

    fn create_cuboid_shape(width: f32, height: f32, length: f32) -> Self::ShapeHandle {
        SharedShape::cuboid(width, height, length)
    }

    fn create_capsule_shape(half_height: f32, radius: f32) -> Self::ShapeHandle {
        SharedShape::capsule_y(half_height, radius)
    }

    fn create_shape_from_mesh(mesh: &Mesh) -> Result<Self::ShapeHandle, MeshShapeCreationError> {
        let vertices: Vec<rapier3d::math::Vector> = mesh
            .vertices
            .iter()
            .map(|vertex| rapier3d::math::Vector::from_array(vertex.pos_coords))
            .collect();
        // `chunks_exact` drops a trailing partial triangle, matching the Jolt
        // shim rather than failing the whole mesh over it.
        let indices: Vec<[u32; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| [triangle[0], triangle[1], triangle[2]])
            .collect();

        SharedShape::trimesh(vertices, indices).map_err(|_| MeshShapeCreationError)
    }

    fn destroy_body(&mut self, body: Self::BodyHandle) {
        self.bodies.remove(
            body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    fn body_transform(&self, body: Self::BodyHandle) -> Transform {
        let pose = self.bodies[body].position();

        Transform::from_translation_rotation(
            from_rapier_vec(pose.translation),
            Quat::from_array(pose.rotation.to_array()),
        )
    }

    fn linear_velocity(&self, body: Self::BodyHandle) -> Vec3 {
        from_rapier_vec(self.bodies[body].linvel())
    }

    fn set_linear_velocity(&mut self, body: Self::BodyHandle, velocity: Vec3) {
        self.bodies[body].set_linvel(to_rapier_vec(velocity), true);
    }

    fn add_impulse(&mut self, body: Self::BodyHandle, impulse: Vec3) {
        self.bodies[body].apply_impulse(to_rapier_vec(impulse), true);
    }

    fn add_impulse_at(&mut self, body: Self::BodyHandle, impulse: Vec3, position: Vec3) {
        self.bodies[body].apply_impulse_at_point(
            to_rapier_vec(impulse),
            to_rapier_vec(position),
            true,
        );
    }

    fn add_force(&mut self, body: Self::BodyHandle, force: Vec3) {
        self.bodies[body].add_force(to_rapier_vec(force), true);
    }

    fn add_force_at(&mut self, body: Self::BodyHandle, force: Vec3, position: Vec3) {
        self.bodies[body].add_force_at_point(to_rapier_vec(force), to_rapier_vec(position), true);
    }

    fn cast_ray(&self, origin: Vec3, direction: Vec3) -> Option<RawRayHit<Self::BodyHandle>> {
        // With an unnormalised direction, a time of impact of 1.0 is the tip
        // of `direction` — so the hit's time of impact is exactly the
        // `RawRayHit` fraction.
        let ray = Ray::new(to_rapier_vec(origin), to_rapier_vec(direction));

        let mut best: Option<RawRayHit<Self::BodyHandle>> = None;
        for (_, collider) in self.colliders.iter() {
            let Some(intersection) =
                collider
                    .shape()
                    .cast_ray_and_get_normal(collider.position(), &ray, 1.0, true)
            else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|best| intersection.time_of_impact < best.fraction)
            {
                best = Some(RawRayHit {
                    body: collider
                        .parent()
                        .expect("every collider is created with a parent body"),
                    fraction: intersection.time_of_impact,
                    normal: from_rapier_vec(intersection.normal),
                });
            }
        }

        best
    }

    fn probe_ground(
        &self,
        body: Self::BodyHandle,
        max_separation: f32,
    ) -> Option<RawGroundHit<Self::BodyHandle>> {
        // Collide the body's shape against every collider whose bounds come
        // within `max_separation` of it, exactly like the Jolt probe's
        // narrow-phase query with a separation margin. Contact manifolds are
        // the primitive the contact solver itself runs on, so — unlike
        // closest-point or shape-cast queries — their normals stay reliable
        // for a body resting exactly on (or slightly inside) the ground.
        let dispatcher = DefaultQueryDispatcher;
        let mut manifolds: Vec<ContactManifold<(), ()>> = Vec::new();

        // Keeps the manifold whose normal points most upward, matching the
        // Jolt shim's collector.
        let mut best: Option<RawGroundHit<Self::BodyHandle>> = None;
        let mut best_dot = f32::NEG_INFINITY;

        for collider_handle in self.bodies[body].colliders() {
            let probe_collider = &self.colliders[*collider_handle];
            let probe_pose = probe_collider.position();
            let candidates = probe_collider.compute_aabb().loosened(max_separation);

            for (_, ground_collider) in self.colliders.iter() {
                if ground_collider.parent() == Some(body)
                    || !candidates.intersects(&ground_collider.compute_aabb())
                {
                    continue;
                }
                let ground_pose = ground_collider.position();

                manifolds.clear();
                let mut workspace = None;
                if dispatcher
                    .contact_manifolds(
                        &probe_pose.inv_mul(ground_pose),
                        probe_collider.shape(),
                        ground_collider.shape(),
                        max_separation,
                        &mut manifolds,
                        &mut workspace,
                    )
                    .is_err()
                {
                    continue;
                }

                for manifold in &manifolds {
                    // The deepest point is the manifold's main contact; a
                    // manifold whose contacts are all beyond the margin is no
                    // ground.
                    let Some(deepest) = manifold
                        .points
                        .iter()
                        .filter(|point| point.dist <= max_separation)
                        .min_by(|a, b| a.dist.total_cmp(&b.dist))
                    else {
                        continue;
                    };

                    // `local_n2` points out of the ground toward the probed
                    // shape (up when standing), like the Jolt probe's negated
                    // penetration axis.
                    let normal = from_rapier_vec(ground_pose.rotation * manifold.local_n2);
                    let dot = normal.dot(Vec3::Y);
                    if dot > best_dot {
                        let ground_body = ground_collider
                            .parent()
                            .expect("every collider is created with a parent body");
                        let point = ground_pose.transform_point(deepest.local_p2);
                        best = Some(RawGroundHit {
                            body: ground_body,
                            point: from_rapier_vec(point),
                            normal,
                            velocity: from_rapier_vec(
                                self.bodies[ground_body].velocity_at_point(point),
                            ),
                        });
                        best_dot = dot;
                    }
                }
            }
        }

        best
    }
}
