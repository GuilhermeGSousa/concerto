use std::collections::{HashMap, HashSet};

use concerto_ecs::entity::Entity;
use concerto_ecs::resource::Resource;
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use glam::Vec3;

use crate::backend::{PhysicsBackend, RawContactKind, SensorContactKind};
use crate::body::{BodyActivity, BodyId};
use crate::collider::{Collider, ColliderOffset};
use crate::contact::{Contact, ContactChange};
use crate::ground::{GroundContact, GroundState};
use crate::query::{QueryFilter, ShapeHit};
use crate::ragdoll::{BodyPose, BuiltRagdoll, JointDrive, RagdollId, RagdollSettings};
use crate::ray::RayHit;
use crate::rigid_body::{MotionType, RigidBody};
use crate::sensor::SensorContact;
use crate::ActiveBackend;

const MIN_SPHERE_CAST_RADIUS: f32 = 1e-3;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SensorOverlap {
    sensor: BodyId,
    other: BodyId,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BodyPair {
    first: BodyId,
    second: BodyId,
}

impl BodyPair {
    fn flipped(self) -> Self {
        Self {
            first: self.second,
            second: self.first,
        }
    }

    fn holds(&self, body: BodyId) -> bool {
        self.first == body || self.second == body
    }
}

struct Touch {
    point: Vec3,
    normal: Vec3,
    impulse: f32,
    summed: bool,
}

#[derive(Clone, Copy)]
struct BodyOwner {
    entity: Entity,
    part: Option<usize>,
}

struct RagdollRecord {
    bodies: Vec<BodyId>,
    built: BuiltRagdoll,
}

/// Owns the physics world of the active backend, plus the engine-side
/// body↔entity bookkeeping shared by every backend.
#[derive(Resource)]
pub struct PhysicsState {
    backend: ActiveBackend,
    /// Maps body ids to the entity holding the matching [`Collider`]
    /// component. Kept in sync by `Collider`'s lifecycle callbacks so
    /// body-to-entity lookups (e.g. after a raycast) are O(1).
    ///
    /// [`Collider`]: crate::collider::Collider
    body_to_entity: HashMap<BodyId, BodyOwner>,
    ragdolls: HashMap<RagdollId, RagdollRecord>,
    overlaps: HashSet<SensorOverlap>,
    sensor_contacts: Vec<SensorContact>,
    reporting: HashSet<BodyId>,
    touching: HashMap<BodyPair, Touch>,
    contact_changes: Vec<ContactChange>,
}

impl PhysicsState {
    pub fn new() -> Self {
        Self {
            backend: ActiveBackend::new(),
            body_to_entity: HashMap::new(),
            ragdolls: HashMap::new(),
            overlaps: HashSet::new(),
            sensor_contacts: Vec::new(),
            reporting: HashSet::new(),
            touching: HashMap::new(),
            contact_changes: Vec::new(),
        }
    }

    /// The backend, for [`PhysicsPipeline`] to drive stepping.
    ///
    /// [`PhysicsPipeline`]: crate::physics_pipeline::PhysicsPipeline
    pub(crate) fn backend_mut(&mut self) -> &mut ActiveBackend {
        &mut self.backend
    }

    /// The entity whose [`Collider`](crate::collider::Collider) owns `body`,
    /// if any.
    pub fn get_entity(&self, body: BodyId) -> Option<Entity> {
        self.body_to_entity.get(&body).map(|owner| owner.entity)
    }

    /// The index of the ragdoll part `body` is, when it belongs to a ragdoll.
    pub fn get_part(&self, body: BodyId) -> Option<usize> {
        self.body_to_entity.get(&body).and_then(|owner| owner.part)
    }

    pub(crate) fn register_body_entity(&mut self, body: BodyId, entity: Entity) {
        self.body_to_entity
            .insert(body, BodyOwner { entity, part: None });
    }

    /// Creates a ragdoll owned by `owner` with each part at its world pose in `pose`, one per part in part order. `None` when the backend has no ragdolls (Rapier), `pose` has the wrong length, or the world is out of bodies.
    pub fn create_ragdoll(
        &mut self,
        built: &BuiltRagdoll,
        owner: Entity,
        pose: &[BodyPose],
    ) -> Option<RagdollId> {
        let ragdoll = RagdollId(self.backend.create_ragdoll(&built.handle, pose)?);
        let bodies: Vec<BodyId> = self
            .backend
            .ragdoll_bodies(ragdoll.0)
            .into_iter()
            .map(BodyId)
            .collect();
        for (part, body) in bodies.iter().enumerate() {
            self.body_to_entity.insert(
                *body,
                BodyOwner {
                    entity: owner,
                    part: Some(part),
                },
            );
        }
        self.ragdolls.insert(
            ragdoll,
            RagdollRecord {
                bodies,
                built: built.clone(),
            },
        );
        Some(ragdoll)
    }

    /// Removes the ragdoll from the simulation and destroys it; an id already destroyed is ignored.
    pub fn destroy_ragdoll(&mut self, ragdoll: RagdollId) {
        let Some(record) = self.ragdolls.remove(&ragdoll) else {
            return;
        };
        self.backend.destroy_ragdoll(ragdoll.0);
        for body in record.bodies {
            self.forget_body(body);
            self.unregister_body_entity(body);
        }
    }

    /// Every ragdoll in the world.
    pub fn ragdolls(&self) -> impl Iterator<Item = RagdollId> + '_ {
        self.ragdolls.keys().copied()
    }

    /// The settings the ragdoll was built from.
    pub fn ragdoll_settings(&self, ragdoll: RagdollId) -> Option<&RagdollSettings> {
        self.ragdolls
            .get(&ragdoll)
            .map(|record| record.built.settings())
    }

    /// How `body` is moving right now: static, kinematic, dynamic or asleep.
    pub fn body_activity(&self, body: BodyId) -> BodyActivity {
        self.backend.body_activity(body.0)
    }

    /// Stops every part of the ragdoll colliding with `body`, a character's own capsule; `None` lets them collide with everything again.
    pub fn set_ragdoll_ignored_body(&mut self, ragdoll: RagdollId, body: Option<BodyId>) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend
                .set_ragdoll_ignored_body(ragdoll.0, body.map(|body| body.0));
        }
    }

    /// The body of each part of the ragdoll, in part order; empty for a destroyed ragdoll.
    pub fn ragdoll_bodies(&self, ragdoll: RagdollId) -> &[BodyId] {
        self.ragdolls
            .get(&ragdoll)
            .map_or(&[], |record| record.bodies.as_slice())
    }

    /// Writes the world pose of every part into `out`, in part order; `out` is left empty for a destroyed ragdoll.
    pub fn ragdoll_pose(&self, ragdoll: RagdollId, out: &mut Vec<BodyPose>) {
        out.clear();
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.ragdoll_pose(ragdoll.0, out);
        }
    }

    /// Moves every part to its world pose in `pose` at once, keeping velocities; ignored when `pose` has the wrong length.
    pub fn set_ragdoll_pose(&mut self, ragdoll: RagdollId, pose: &[BodyPose]) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.set_ragdoll_pose(ragdoll.0, pose);
        }
    }

    pub(crate) fn unregister_body_entity(&mut self, body: BodyId) {
        self.body_to_entity.remove(&body);
    }

    /// Creates a body with the given shape at `transform`'s position and
    /// rotation and adds it to the simulation: dynamic with `rigid_body`'s
    /// parameters when it is `Some`, static otherwise. Called by
    /// `Collider::on_add`.
    pub(crate) fn create_body(
        &mut self,
        collider: &Collider,
        transform: &Transform,
        rigid_body: Option<&RigidBody>,
        offset: Option<&ColliderOffset>,
        sensor: bool,
        gravity_scale: f32,
    ) -> BodyId {
        BodyId(self.backend.create_body(
            collider,
            transform,
            rigid_body,
            offset,
            sensor,
            gravity_scale,
        ))
    }

    /// Sets each part's velocity so that it reaches its world pose in `pose` at the end of the next fixed step. Kinematic parts follow it exactly and push dynamic bodies aside; see [`set_ragdoll_part_motion`](Self::set_ragdoll_part_motion).
    pub fn drive_ragdoll_kinematic(&mut self, ragdoll: RagdollId, pose: &[BodyPose]) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.drive_ragdoll_kinematic(
                ragdoll.0,
                pose,
                Time::fixed_delta_time().as_secs_f32(),
            );
        }
    }

    /// Turns every joint's motor on, aimed at the rotation each part has relative to its parent in `pose`, with the spring the joint last had.
    pub fn drive_ragdoll_motors(&mut self, ragdoll: RagdollId, pose: &[BodyPose]) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.drive_ragdoll_motors(ragdoll.0, pose);
        }
    }

    /// Sets what the motor of the joint between `part` and its parent does; the root has no joint and ignores it.
    pub fn set_ragdoll_joint_drive(&mut self, ragdoll: RagdollId, part: usize, drive: JointDrive) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.set_ragdoll_joint_drive(ragdoll.0, part, drive);
        }
    }

    /// Makes one part kinematic or dynamic, keeping its velocity, so a kinematic pelvis can carry a powered torso.
    pub fn set_ragdoll_part_motion(&mut self, ragdoll: RagdollId, part: usize, motion: MotionType) {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend
                .set_ragdoll_part_motion(ragdoll.0, part, motion);
        }
    }

    /// The angular impulse the motor of the joint between `part` and its parent applied in the last collision step, about the joint's twist, normal and plane axes; a motor at its torque limit is saturated.
    pub fn ragdoll_joint_motor_impulse(&self, ragdoll: RagdollId, part: usize) -> Vec3 {
        if self.ragdolls.contains_key(&ragdoll) {
            self.backend.ragdoll_joint_motor_impulse(ragdoll.0, part)
        } else {
            Vec3::ZERO
        }
    }

    /// Removes `body` from the simulation and destroys it. Called by
    /// `Collider::on_remove`.
    pub(crate) fn destroy_body(&mut self, body: BodyId) {
        self.backend.destroy_body(body.0);
        self.forget_body(body);
    }

    fn forget_body(&mut self, body: BodyId) {
        self.reporting.remove(&body);
        let ended: Vec<BodyPair> = self
            .touching
            .keys()
            .filter(|pair| pair.holds(body))
            .copied()
            .collect();
        for pair in ended {
            self.touching.remove(&pair);
            let other = if pair.first == body {
                pair.second
            } else {
                pair.first
            };
            self.push_contact_change(other, body, false);
        }
        let Self {
            body_to_entity,
            overlaps,
            sensor_contacts,
            ..
        } = self;
        overlaps.retain(|overlap| {
            if overlap.sensor != body && overlap.other != body {
                return true;
            }
            if let (Some(sensor), Some(other)) = (
                body_to_entity.get(&overlap.sensor),
                body_to_entity.get(&overlap.other),
            ) {
                sensor_contacts.push(SensorContact {
                    sensor: sensor.entity,
                    other: other.entity,
                    kind: SensorContactKind::Exit,
                });
            }
            false
        });
    }

    pub(crate) fn gather_sensor_contacts(&mut self) {
        let mut raw = Vec::new();
        self.backend.drain_sensor_contacts(&mut raw);
        for contact in raw {
            let overlap = SensorOverlap {
                sensor: BodyId(contact.sensor),
                other: BodyId(contact.other),
            };
            let changed = match contact.kind {
                SensorContactKind::Enter => self.overlaps.insert(overlap),
                SensorContactKind::Exit => self.overlaps.remove(&overlap),
            };
            if !changed {
                continue;
            }
            if let (Some(sensor), Some(other)) = (
                self.get_entity(overlap.sensor),
                self.get_entity(overlap.other),
            ) {
                self.sensor_contacts.push(SensorContact {
                    sensor,
                    other,
                    kind: contact.kind,
                });
            }
        }
    }

    pub(crate) fn set_contact_reporting(&mut self, body: BodyId, enabled: bool) {
        self.backend.set_contact_reporting(body.0, enabled);
        if enabled {
            self.reporting.insert(body);
            return;
        }
        self.reporting.remove(&body);
        let reporting = &self.reporting;
        self.touching
            .retain(|pair, _| reporting.contains(&pair.first) || reporting.contains(&pair.second));
    }

    fn touching_pair(&self, pair: BodyPair) -> Option<BodyPair> {
        [pair, pair.flipped()]
            .into_iter()
            .find(|pair| self.touching.contains_key(pair))
    }

    fn push_contact_change(&mut self, body: BodyId, other: BodyId, started: bool) {
        if !self.reporting.contains(&body) {
            return;
        }
        if let Some(entity) = self.get_entity(body) {
            self.contact_changes.push(ContactChange {
                entity,
                other: self.get_entity(other),
                started,
            });
        }
    }

    pub(crate) fn gather_contacts(&mut self) {
        let mut raw = Vec::new();
        self.backend.drain_contacts(&mut raw);
        for touch in self.touching.values_mut() {
            touch.summed = false;
        }
        for contact in raw {
            let pair = BodyPair {
                first: BodyId(contact.first),
                second: BodyId(contact.second),
            };
            let known = self.touching_pair(pair);
            match contact.kind {
                RawContactKind::Start => {
                    if known.is_none() {
                        self.touching.insert(
                            pair,
                            Touch {
                                point: Vec3::ZERO,
                                normal: Vec3::ZERO,
                                impulse: 0.0,
                                summed: true,
                            },
                        );
                        self.push_contact_change(pair.first, pair.second, true);
                        self.push_contact_change(pair.second, pair.first, true);
                    }
                }
                RawContactKind::Touch {
                    point,
                    normal,
                    impulse,
                } => {
                    let Some(known) = known else {
                        continue;
                    };
                    let normal = if known == pair { normal } else { -normal };
                    let touch = self
                        .touching
                        .get_mut(&known)
                        .expect("the pair was just found");
                    if !touch.summed {
                        touch.impulse = 0.0;
                        touch.summed = true;
                    }
                    touch.point = point;
                    touch.normal = normal;
                    touch.impulse += impulse;
                }
                RawContactKind::End => {
                    if let Some(known) = known {
                        self.touching.remove(&known);
                        self.push_contact_change(pair.first, pair.second, false);
                        self.push_contact_change(pair.second, pair.first, false);
                    }
                }
            }
        }
    }

    pub(crate) fn take_contact_changes(&mut self) -> Vec<ContactChange> {
        std::mem::take(&mut self.contact_changes)
    }

    pub(crate) fn contacts_of(&self, body: BodyId) -> Vec<Contact> {
        self.touching
            .iter()
            .filter(|(pair, _)| pair.holds(body))
            .map(|(pair, touch)| {
                let second = pair.second == body;
                Contact {
                    other: self.get_entity(if second { pair.first } else { pair.second }),
                    point: touch.point,
                    normal: if second { touch.normal } else { -touch.normal },
                    impulse: touch.impulse,
                }
            })
            .collect()
    }

    pub(crate) fn take_sensor_contacts(&mut self) -> Vec<SensorContact> {
        std::mem::take(&mut self.sensor_contacts)
    }

    /// Reports what `body` is standing on: the most upward-facing contact
    /// within `max_separation` below its shape, classified against
    /// `max_slope_angle` (radians from horizontal).
    pub fn probe_ground(
        &self,
        body: BodyId,
        max_separation: f32,
        max_slope_angle: f32,
    ) -> GroundState {
        let Some(hit) = self.backend.probe_ground(body.0, max_separation) else {
            return GroundState::InAir;
        };

        let contact = GroundContact {
            entity: self.get_entity(BodyId(hit.body)),
            point: hit.point,
            normal: hit.normal,
            velocity: hit.velocity,
        };
        if hit.normal.dot(Vec3::Y) < max_slope_angle.cos() {
            GroundState::OnSteepGround(contact)
        } else {
            GroundState::OnGround(contact)
        }
    }

    /// Makes a dynamic or kinematic body collide with nothing and be skipped by every query, or puts it back; a static body, a sensor and a ragdoll part are left alone.
    pub fn set_body_ghost(&mut self, body: BodyId, ghost: bool) {
        self.backend.set_body_ghost(body.0, ghost);
    }

    /// Moves the body's origin at once, keeping its rotation and velocity.
    pub fn set_body_translation(&mut self, body: BodyId, translation: Vec3) {
        self.backend.set_body_translation(body.0, translation);
    }

    pub fn set_linear_velocity(&mut self, body: BodyId, velocity: Vec3) {
        self.backend.set_linear_velocity(body.0, velocity);
    }

    pub fn linear_velocity(&self, body: BodyId) -> Vec3 {
        self.backend.linear_velocity(body.0)
    }

    /// Reads a body's current world transform out of the simulation.
    pub fn body_transform(&self, body: BodyId) -> Transform {
        self.backend.body_transform(body.0)
    }

    /// Casts a ray from `origin` along `direction` — whose length is the
    /// maximum cast distance — and returns the closest hit, if any, skipping
    /// the bodies `filter` ignores.
    pub fn cast_ray(&self, origin: Vec3, direction: Vec3, filter: QueryFilter) -> Option<RayHit> {
        let ignore: Vec<_> = filter.ignore.iter().map(|body| body.0).collect();
        self.backend
            .cast_ray(origin, direction, &ignore, filter.ragdolls)
            .map(|hit| {
                let body = BodyId(hit.body);

                RayHit {
                    body,
                    entity: self.get_entity(body),
                    part: self.get_part(body),
                    fraction: hit.fraction,
                    point: origin + direction * hit.fraction,
                    normal: hit.normal,
                }
            })
    }

    /// Sweeps a sphere of `radius` from `origin` along `direction`, whose length is the sweep's reach.
    pub fn cast_sphere(
        &self,
        origin: Vec3,
        radius: f32,
        direction: Vec3,
        filter: QueryFilter,
    ) -> Option<ShapeHit> {
        let ignore: Vec<_> = filter.ignore.iter().map(|body| body.0).collect();
        self.backend
            .cast_sphere(
                origin,
                radius.max(MIN_SPHERE_CAST_RADIUS),
                direction,
                &ignore,
                filter.ragdolls,
            )
            .map(|hit| {
                let body = BodyId(hit.body);

                ShapeHit {
                    body,
                    entity: self.get_entity(body),
                    part: self.get_part(body),
                    fraction: hit.fraction,
                    point: hit.point,
                    normal: hit.normal,
                }
            })
    }

    pub fn add_impulse(&mut self, body: BodyId, impulse: Vec3) {
        self.backend.add_impulse(body.0, impulse);
    }

    pub fn add_impulse_at(&mut self, body: BodyId, impulse: Vec3, position: Vec3) {
        self.backend.add_impulse_at(body.0, impulse, position);
    }

    pub fn add_force(&mut self, body: BodyId, force: Vec3) {
        self.backend.add_force(body.0, force);
    }

    pub fn add_force_at(&mut self, body: BodyId, force: Vec3, position: Vec3) {
        self.backend.add_force_at(body.0, force, position);
    }
}

impl Default for PhysicsState {
    fn default() -> Self {
        Self::new()
    }
}
