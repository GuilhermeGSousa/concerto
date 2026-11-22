//! The Rapier physics backend. Pure Rust, so unlike Jolt it compiles to
//! `wasm32-unknown-unknown` — it is the backend of every web build. Native
//! builds can opt into it with the `force-rapier` feature to test and debug
//! it without a browser.
//!
//! Rapier bundles its own `glam` (a newer major than the engine's), so
//! engine↔Rapier math conversions go through arrays at this boundary, the
//! same way the Jolt backend converts at the FFI boundary.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::{Quat, Vec3};
use log::warn;
use rapier3d::parry::bounding_volume::BoundingVolume;
use rapier3d::parry::partitioning::BvhNode;
use rapier3d::parry::query::{
    cast_shapes, ContactManifold, DefaultQueryDispatcher, PersistentQueryDispatcher, RayCast,
    ShapeCastOptions,
};
use rapier3d::parry::shape::Ball;
use rapier3d::pipeline::EventHandler;
use rapier3d::prelude::{
    ActiveCollisionTypes, ActiveEvents, BroadPhaseBvh, CCDSolver, ColliderBuilder, ColliderHandle,
    ColliderSet, CollisionEvent, CollisionEventFlags, ContactPair, Group, ImpulseJointSet,
    IntegrationParameters, InteractionGroups, InteractionTestMode, IslandManager, LockedAxes,
    MultibodyJointSet, NarrowPhase, QueryFilter, QueryPipeline, Ray, RigidBodyBuilder,
    RigidBodyHandle, RigidBodySet, SharedShape,
};

use crate::backend::{
    Aabb, MeshShapeCreationError, PhysicsBackend, RawContact, RawContactKind, RawGroundHit,
    RawRayHit, RawSensorContact, RawShapeHit, SensorContactKind,
};
use crate::body::BodyActivity;
use crate::collider::{Collider, ColliderOffset};
use crate::layers::{CollisionLayers, Layers};
use crate::ragdoll::{BodyPose, JointDrive, RagdollBuildError, RagdollSettings};
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

fn to_interaction_groups(layers: CollisionLayers) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(layers.memberships.mask()),
        Group::from_bits_truncate(layers.filter.mask()),
        InteractionTestMode::And,
    )
}

fn in_layers(collider: &rapier3d::geometry::Collider, layers: Layers) -> bool {
    collider.collision_groups().memberships.bits() & layers.mask() != 0
}

fn collides(a: &rapier3d::geometry::Collider, b: &rapier3d::geometry::Collider) -> bool {
    let a = a.collision_groups();
    let b = b.collision_groups();
    a.memberships.bits() & b.filter.bits() != 0 && b.memberships.bits() & a.filter.bits() != 0
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

#[derive(Default)]
struct CollisionEventCollector {
    events: Mutex<Vec<CollisionEvent>>,
}

impl EventHandler for CollisionEventCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
    }
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
    collision_events: CollisionEventCollector,
    reporting: HashSet<RigidBodyHandle>,
    contact_changes: Vec<RawContact<RigidBodyHandle>>,
    touches: Vec<RawContact<RigidBodyHandle>>,
}

fn collider_order(handle: ColliderHandle) -> u32 {
    handle.into_raw_parts().0
}

impl RapierBackend {
    fn scene<'a>(&'a self, filter: QueryFilter<'a>) -> QueryPipeline<'a> {
        self.broad_phase.as_query_pipeline(
            self.narrow_phase.query_dispatcher(),
            &self.bodies,
            &self.colliders,
            filter.exclude_sensors(),
        )
    }

    fn gather_touches(&mut self) {
        if self.reporting.is_empty() {
            return;
        }
        for pair in self.narrow_phase.contact_pairs() {
            let (Some(collider1), Some(collider2)) = (
                self.colliders.get(pair.collider1),
                self.colliders.get(pair.collider2),
            ) else {
                continue;
            };
            let (Some(first), Some(second)) = (collider1.parent(), collider2.parent()) else {
                continue;
            };
            if !self.reporting.contains(&first) && !self.reporting.contains(&second) {
                continue;
            }
            let Some(manifold) = pair
                .manifolds
                .iter()
                .find(|manifold| !manifold.points.is_empty())
            else {
                continue;
            };
            let impulse = pair
                .manifolds
                .iter()
                .flat_map(|manifold| manifold.points.iter())
                .map(|point| point.data.impulse)
                .sum();
            self.touches.push(RawContact {
                first,
                second,
                kind: RawContactKind::Touch {
                    point: from_rapier_vec(
                        collider1
                            .position()
                            .transform_point(manifold.points[0].local_p1),
                    ),
                    normal: from_rapier_vec(manifold.data.normal),
                    impulse,
                },
            });
        }
    }
}

/// The handle of a ragdoll on a backend that has none; never created.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NoRagdoll;

/// Per-step scratch: Rapier's execution pipeline and the integration
/// parameters fed to it each step.
pub struct Stepper {
    pipeline: rapier3d::pipeline::PhysicsPipeline,
    integration_parameters: IntegrationParameters,
}

impl PhysicsBackend for RapierBackend {
    type BodyHandle = RigidBodyHandle;

    type ShapeHandle = SharedShape;

    type Stepper = Stepper;

    type RagdollSettingsHandle = ();

    type RagdollHandle = NoRagdoll;

    fn build_ragdoll(_settings: &RagdollSettings) -> Result<(), RagdollBuildError> {
        Ok(())
    }

    fn create_ragdoll(&mut self, _built: &(), _pose: &[BodyPose]) -> Option<Self::RagdollHandle> {
        None
    }

    fn destroy_ragdoll(&mut self, _ragdoll: Self::RagdollHandle) {}

    fn ragdoll_bodies(&self, _ragdoll: Self::RagdollHandle) -> Vec<Self::BodyHandle> {
        Vec::new()
    }

    fn ragdoll_pose(&self, _ragdoll: Self::RagdollHandle, _out: &mut Vec<BodyPose>) {}

    fn set_ragdoll_pose(&mut self, _ragdoll: Self::RagdollHandle, _pose: &[BodyPose]) {}

    fn drive_ragdoll_kinematic(
        &mut self,
        _ragdoll: Self::RagdollHandle,
        _pose: &[BodyPose],
        _delta_time: f32,
    ) {
    }

    fn drive_ragdoll_motors(&mut self, _ragdoll: Self::RagdollHandle, _pose: &[BodyPose]) {}

    fn set_ragdoll_joint_drive(
        &mut self,
        _ragdoll: Self::RagdollHandle,
        _part: usize,
        _drive: JointDrive,
    ) {
    }

    fn set_ragdoll_part_motion(
        &mut self,
        _ragdoll: Self::RagdollHandle,
        _part: usize,
        _motion: MotionType,
    ) {
    }

    fn set_ragdoll_frozen(&mut self, _ragdoll: Self::RagdollHandle, _frozen: bool) {}

    fn ragdoll_joint_motor_impulse(&self, _ragdoll: Self::RagdollHandle, _part: usize) -> Vec3 {
        Vec3::ZERO
    }

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
            collision_events: CollisionEventCollector::default(),
            reporting: HashSet::new(),
            contact_changes: Vec::new(),
            touches: Vec::new(),
        }
    }

    fn new_stepper() -> Stepper {
        Stepper {
            pipeline: rapier3d::pipeline::PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
        }
    }

    fn step(&mut self, stepper: &mut Stepper, delta_time: f32, collision_steps: u32) {
        let collision_steps = collision_steps.max(1);
        stepper.integration_parameters.dt = delta_time / collision_steps as f32;
        for _ in 0..collision_steps {
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
                &self.collision_events,
            );
            self.gather_touches();
        }
    }

    fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
        sensor: bool,
        gravity_scale: f32,
        layers: CollisionLayers,
    ) -> Self::BodyHandle {
        let mut body_builder = match rigid_body {
            None => RigidBodyBuilder::fixed(),
            Some(_) if sensor => RigidBodyBuilder::kinematic_velocity_based(),
            Some(rb) => match rb.motion_type {
                MotionType::Dynamic => RigidBodyBuilder::dynamic(),
                MotionType::Kinematic => RigidBodyBuilder::kinematic_velocity_based(),
            },
        }
        // Only the translation and rotation: the scale belongs to the shape.
        .pose(to_rapier_pose(transform))
        .gravity_scale(gravity_scale);
        if let Some(rigid_body) = rigid_body {
            body_builder = body_builder.locked_axes(to_locked_axes(rigid_body.allowed_dofs));
        }
        let handle = self.bodies.insert(body_builder);

        // Density is unused for static bodies; any sane value works here.
        let density = rigid_body.map_or(1000.0, |rigid_body| rigid_body.density);
        let mut collider_builder =
            ColliderBuilder::new(scaled_shape(&collider.shape().0, transform.scale))
                .density(density)
                .collision_groups(to_interaction_groups(layers));
        if let Some(offset) = offset {
            // The offset is the collider's pose relative to the body, so it
            // shifts the geometry without showing up in `body_transform`.
            collider_builder = collider_builder.translation(to_rapier_vec(offset.0));
        }
        if sensor {
            collider_builder = collider_builder
                .sensor(true)
                .active_events(ActiveEvents::COLLISION_EVENTS)
                .active_collision_types(
                    ActiveCollisionTypes::default()
                        | ActiveCollisionTypes::KINEMATIC_FIXED
                        | ActiveCollisionTypes::KINEMATIC_KINEMATIC,
                );
        }
        let collider_handle =
            self.colliders
                .insert_with_parent(collider_builder, handle, &mut self.bodies);
        self.broad_phase.set_aabb(
            &IntegrationParameters::default(),
            collider_handle,
            self.colliders[collider_handle].compute_aabb(),
        );

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
        self.reporting.remove(&body);
        self.contact_changes
            .retain(|change| change.first != body && change.second != body);
        self.touches
            .retain(|touch| touch.first != body && touch.second != body);
    }

    fn drain_sensor_contacts(&mut self, out: &mut Vec<RawSensorContact<Self::BodyHandle>>) {
        let events = std::mem::take(
            self.collision_events
                .events
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );
        for event in events {
            let (handle1, handle2, flags, kind) = match event {
                CollisionEvent::Started(handle1, handle2, flags) => {
                    (handle1, handle2, flags, SensorContactKind::Enter)
                }
                CollisionEvent::Stopped(handle1, handle2, flags) => {
                    (handle1, handle2, flags, SensorContactKind::Exit)
                }
            };
            let (Some(collider1), Some(collider2)) =
                (self.colliders.get(handle1), self.colliders.get(handle2))
            else {
                continue;
            };
            if !flags.contains(CollisionEventFlags::SENSOR) {
                if let (Some(first), Some(second)) = (collider1.parent(), collider2.parent()) {
                    self.contact_changes.push(RawContact {
                        first,
                        second,
                        kind: match kind {
                            SensorContactKind::Enter => RawContactKind::Start,
                            SensorContactKind::Exit => RawContactKind::End,
                        },
                    });
                }
                continue;
            }
            let (sensor, other) = match (collider1.is_sensor(), collider2.is_sensor()) {
                (true, false) => (collider1, collider2),
                (false, true) => (collider2, collider1),
                _ => continue,
            };
            let (Some(sensor_body), Some(other_body)) = (sensor.parent(), other.parent()) else {
                continue;
            };
            if self
                .bodies
                .get(other_body)
                .is_none_or(|body| body.is_fixed())
            {
                continue;
            }
            out.push(RawSensorContact {
                sensor: sensor_body,
                other: other_body,
                kind,
            });
        }
    }

    fn set_contact_reporting(&mut self, body: Self::BodyHandle, enabled: bool) {
        if enabled {
            self.reporting.insert(body);
        } else {
            self.reporting.remove(&body);
        }
        let Some(rigid_body) = self.bodies.get_mut(body) else {
            return;
        };
        if enabled {
            rigid_body.wake_up(true);
        }
        for handle in rigid_body.colliders().to_vec() {
            let Some(collider) = self.colliders.get_mut(handle) else {
                continue;
            };
            if collider.is_sensor() {
                continue;
            }
            collider.set_active_events(if enabled {
                ActiveEvents::COLLISION_EVENTS
            } else {
                ActiveEvents::empty()
            });
        }
    }

    fn drain_contacts(&mut self, out: &mut Vec<RawContact<Self::BodyHandle>>) {
        let (starts, ends): (Vec<_>, Vec<_>) = self
            .contact_changes
            .drain(..)
            .partition(|change| change.kind == RawContactKind::Start);
        out.extend(starts);
        out.append(&mut self.touches);
        out.extend(ends);
    }

    fn set_body_layers(&mut self, body: Self::BodyHandle, layers: CollisionLayers) {
        let Some(rigid_body) = self.bodies.get_mut(body) else {
            return;
        };
        rigid_body.wake_up(true);
        for handle in rigid_body.colliders().to_vec() {
            if let Some(collider) = self.colliders.get_mut(handle) {
                collider.set_collision_groups(to_interaction_groups(layers));
            }
        }
    }

    fn move_body_kinematic(&mut self, body: Self::BodyHandle, pose: BodyPose, delta_time: f32) {
        let Some(rigid_body) = self.bodies.get_mut(body) else {
            return;
        };
        if delta_time <= 0.0 {
            return;
        }
        let current = rigid_body.position();
        let from = Quat::from_array(current.rotation.to_array());
        let turn = (pose.rotation * from.inverse()).normalize();
        let turn = if turn.w < 0.0 { -turn } else { turn };
        let linear = (pose.translation - from_rapier_vec(current.translation)) / delta_time;
        let angular = turn.to_scaled_axis() / delta_time;
        rigid_body.set_linvel(to_rapier_vec(linear), true);
        rigid_body.set_angvel(to_rapier_vec(angular), true);
    }

    fn set_body_translation(&mut self, body: Self::BodyHandle, translation: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.set_translation(to_rapier_vec(translation), false);
        }
    }

    fn body_activity(&self, body: Self::BodyHandle) -> BodyActivity {
        let Some(rigid_body) = self.bodies.get(body) else {
            return BodyActivity::Static;
        };
        if rigid_body.is_fixed() {
            BodyActivity::Static
        } else if rigid_body.is_sleeping() {
            BodyActivity::Asleep
        } else if rigid_body.is_kinematic() {
            BodyActivity::Kinematic
        } else {
            BodyActivity::Dynamic
        }
    }

    fn body_transform(&self, body: Self::BodyHandle) -> Transform {
        let Some(rigid_body) = self.bodies.get(body) else {
            return Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY);
        };
        let pose = rigid_body.position();

        Transform::from_translation_rotation(
            from_rapier_vec(pose.translation),
            Quat::from_array(pose.rotation.to_array()),
        )
    }

    fn linear_velocity(&self, body: Self::BodyHandle) -> Vec3 {
        self.bodies.get(body).map_or(Vec3::ZERO, |rigid_body| {
            from_rapier_vec(rigid_body.linvel())
        })
    }

    fn angular_velocity(&self, body: Self::BodyHandle) -> Vec3 {
        self.bodies.get(body).map_or(Vec3::ZERO, |rigid_body| {
            from_rapier_vec(rigid_body.angvel())
        })
    }

    fn set_linear_velocity(&mut self, body: Self::BodyHandle, velocity: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.set_linvel(to_rapier_vec(velocity), true);
        }
    }

    fn add_impulse(&mut self, body: Self::BodyHandle, impulse: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.apply_impulse(to_rapier_vec(impulse), true);
        }
    }

    fn add_impulse_at(&mut self, body: Self::BodyHandle, impulse: Vec3, position: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.apply_impulse_at_point(
                to_rapier_vec(impulse),
                to_rapier_vec(position),
                true,
            );
        }
    }

    fn add_force(&mut self, body: Self::BodyHandle, force: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.add_force(to_rapier_vec(force), true);
        }
    }

    fn add_torque(&mut self, body: Self::BodyHandle, torque: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.add_torque(to_rapier_vec(torque), true);
        }
    }

    fn add_force_at(&mut self, body: Self::BodyHandle, force: Vec3, position: Vec3) {
        if let Some(rigid_body) = self.bodies.get_mut(body) {
            rigid_body.add_force_at_point(to_rapier_vec(force), to_rapier_vec(position), true);
        }
    }

    fn cast_ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        ignore: &[Self::BodyHandle],
        layers: Layers,
    ) -> Option<RawRayHit<Self::BodyHandle>> {
        // With an unnormalised direction, a time of impact of 1.0 is the tip
        // of `direction` — so the hit's time of impact is exactly the
        // `RawRayHit` fraction.
        let ray = Ray::new(to_rapier_vec(origin), to_rapier_vec(direction));
        let unignored = |_: ColliderHandle, collider: &rapier3d::prelude::Collider| {
            collider.parent().is_none_or(|body| !ignore.contains(&body))
                && in_layers(collider, layers)
        };
        let scene = self.scene(QueryFilter::new().predicate(&unignored));

        let mut best: Option<RawRayHit<Self::BodyHandle>> = None;
        let mut best_order = u32::MAX;
        for (handle, collider, intersection) in scene.intersect_ray(ray, 1.0, true) {
            let order = collider_order(handle);
            if best.as_ref().is_none_or(|best| {
                intersection.time_of_impact < best.fraction
                    || (intersection.time_of_impact == best.fraction && order < best_order)
            }) {
                best = Some(RawRayHit {
                    body: collider
                        .parent()
                        .expect("every collider is created with a parent body"),
                    fraction: intersection.time_of_impact,
                    normal: from_rapier_vec(intersection.normal),
                });
                best_order = order;
            }
        }

        best
    }

    fn cast_sphere(
        &self,
        origin: Vec3,
        radius: f32,
        direction: Vec3,
        ignore: &[Self::BodyHandle],
        layers: Layers,
    ) -> Option<RawShapeHit<Self::BodyHandle>> {
        let ball = Ball::new(radius);
        let ball_pose = rapier3d::math::Pose::from_translation(to_rapier_vec(origin));
        let velocity = to_rapier_vec(direction);
        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            stop_at_penetration: true,
            ..ShapeCastOptions::default()
        };

        let sweep = Ray::new(to_rapier_vec(origin), velocity);
        let unignored = |_: ColliderHandle, collider: &rapier3d::prelude::Collider| {
            collider.parent().is_none_or(|body| !ignore.contains(&body))
                && in_layers(collider, layers)
        };
        let scene = self.scene(QueryFilter::new().predicate(&unignored));

        let mut best: Option<RawShapeHit<Self::BodyHandle>> = None;
        let mut best_order = u32::MAX;
        for leaf in scene.bvh.leaves(|node: &BvhNode| {
            node.aabb()
                .loosened(radius)
                .intersects_local_ray(&sweep, 1.0)
        }) {
            let Some((collider, handle)) = self.colliders.get_unknown_gen(leaf) else {
                continue;
            };
            if !scene.filter.test(&self.bodies, handle, collider) {
                continue;
            }
            let order = collider_order(handle);
            let pose = collider.position();
            let Ok(Some(hit)) = cast_shapes(
                &ball_pose,
                velocity,
                &ball,
                pose,
                rapier3d::math::Vector::ZERO,
                collider.shape(),
                options,
            ) else {
                continue;
            };
            if best.as_ref().is_none_or(|best| {
                hit.time_of_impact < best.fraction
                    || (hit.time_of_impact == best.fraction && order < best_order)
            }) {
                best = Some(RawShapeHit {
                    body: collider
                        .parent()
                        .expect("every collider is created with a parent body"),
                    fraction: hit.time_of_impact,
                    point: from_rapier_vec(pose.transform_point(hit.witness2)),
                    normal: from_rapier_vec(pose.rotation * hit.normal2),
                });
                best_order = order;
            }
        }

        best
    }

    fn probe_ground(
        &self,
        body: Self::BodyHandle,
        max_separation: f32,
    ) -> Option<RawGroundHit<Self::BodyHandle>> {
        let rigid_body = self.bodies.get(body)?;
        let scene = self.scene(QueryFilter::new().exclude_rigid_body(body));
        let mut grounds: Vec<ColliderHandle> = Vec::new();

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

        for collider_handle in rigid_body.colliders() {
            let probe_collider = &self.colliders[*collider_handle];
            let probe_pose = probe_collider.position();
            let candidates = probe_collider.compute_aabb().loosened(max_separation);

            grounds.clear();
            grounds.extend(
                scene
                    .intersect_aabb_conservative(candidates)
                    .filter(|(_, ground_collider)| {
                        collides(probe_collider, ground_collider)
                            && candidates.intersects(&ground_collider.compute_aabb())
                    })
                    .map(|(handle, _)| handle),
            );
            grounds.sort_unstable_by_key(|handle| collider_order(*handle));

            for ground_handle in &grounds {
                let ground_collider = &self.colliders[*ground_handle];
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
