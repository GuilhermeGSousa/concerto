use std::collections::HashMap;

use concerto_core::transform::Transform;
use concerto_ecs::entity::Entity;
use concerto_ecs::resource::Resource;
use glam::Vec3;

use crate::backend::PhysicsBackend;
use crate::body::BodyId;
use crate::collider::{Collider, ColliderOffset};
use crate::ground::{GroundContact, GroundState};
use crate::ray::RayHit;
use crate::rigid_body::RigidBody;
use crate::ActiveBackend;

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
    body_to_entity: HashMap<BodyId, Entity>,
}

impl PhysicsState {
    pub fn new() -> Self {
        Self {
            backend: ActiveBackend::new(),
            body_to_entity: HashMap::new(),
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
        self.body_to_entity.get(&body).copied()
    }

    pub(crate) fn register_body_entity(&mut self, body: BodyId, entity: Entity) {
        self.body_to_entity.insert(body, entity);
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
    ) -> BodyId {
        BodyId(
            self.backend
                .create_body(collider, transform, rigid_body, offset),
        )
    }

    /// Removes `body` from the simulation and destroys it. Called by
    /// `Collider::on_remove`.
    pub(crate) fn destroy_body(&mut self, body: BodyId) {
        self.backend.destroy_body(body.0);
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
    /// maximum cast distance — and returns the closest hit, if any.
    pub fn cast_ray(&self, origin: Vec3, direction: Vec3) -> Option<RayHit> {
        self.backend.cast_ray(origin, direction).map(|hit| {
            let body = BodyId(hit.body);

            RayHit {
                body,
                entity: self.get_entity(body),
                fraction: hit.fraction,
                point: origin + direction * hit.fraction,
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
