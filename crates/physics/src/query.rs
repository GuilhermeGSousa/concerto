use concerto_ecs::entity::Entity;
use glam::Vec3;

use crate::body::BodyId;
use crate::layers::Layers;

/// What a scene query skips and which collision layers it sees.
#[derive(Clone, Copy, Debug)]
pub struct QueryFilter<'a> {
    /// Bodies the query passes through.
    pub ignore: &'a [BodyId],
    /// The query sees a body that is in any of these layers.
    ///
    /// By default every layer but [`Layers::RAGDOLL`], so a foot ray or a
    /// camera boom meets a character's capsule and never its limbs.
    pub layers: Layers,
}

impl<'a> QueryFilter<'a> {
    /// The layers a query sees unless it says otherwise: every one but `Layers::RAGDOLL`.
    pub const DEFAULT_LAYERS: Layers = Layers::ALL.without(Layers::RAGDOLL);

    /// A filter that skips `ignore` and sees the default layers.
    pub fn ignoring(ignore: &'a [BodyId]) -> Self {
        Self {
            ignore,
            layers: Self::DEFAULT_LAYERS,
        }
    }

    /// Sees the bodies in any of `layers` and nothing else.
    pub fn with_layers(mut self, layers: Layers) -> Self {
        self.layers = layers;
        self
    }
}

impl Default for QueryFilter<'_> {
    fn default() -> Self {
        Self::ignoring(&[])
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
