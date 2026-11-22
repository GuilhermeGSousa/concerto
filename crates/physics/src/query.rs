use concerto_ecs::entity::Entity;
use glam::Vec3;

use crate::body::BodyId;

/// Whether a scene query sees the bodies of ragdoll parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RagdollQuery {
    /// The query passes through ragdoll parts, so a foot ray or a camera boom never meets a character's limbs.
    #[default]
    Skip,
    /// The query sees ragdoll parts and everything else.
    Include,
    /// The query sees only ragdoll parts, as a shot that asks which limb it hit does.
    Only,
}

/// What a scene query skips.
#[derive(Clone, Copy, Debug, Default)]
pub struct QueryFilter<'a> {
    pub ignore: &'a [BodyId],
    pub ragdolls: RagdollQuery,
}

impl<'a> QueryFilter<'a> {
    /// A filter that skips `ignore` and ragdoll parts.
    pub fn ignoring(ignore: &'a [BodyId]) -> Self {
        Self {
            ignore,
            ragdolls: RagdollQuery::Skip,
        }
    }

    pub fn with_ragdolls(mut self, ragdolls: RagdollQuery) -> Self {
        self.ragdolls = ragdolls;
        self
    }
}

/// The first body a swept sphere touches, found by [`PhysicsState::cast_sphere`](crate::physics_state::PhysicsState::cast_sphere).
#[derive(Clone, Copy, Debug)]
pub struct ShapeHit {
    pub body: BodyId,
    pub entity: Option<Entity>,
    /// The index of the ragdoll part that was hit, when the body belongs to a ragdoll.
    pub part: Option<usize>,
    /// How far along the sweep the sphere first touches, as a fraction `[0, 1]` of `direction`.
    pub fraction: f32,
    /// World-space contact point on the hit body.
    pub point: Vec3,
    /// World-space normal of the hit surface, pointing away from it.
    pub normal: Vec3,
}
