use concerto_ecs::{component::Component, entity::Entity, query::Query, resource::Res};
use glam::Vec3;

use crate::{body::BodyId, physics_state::PhysicsState};

/// One ground contact; only exists while touching something.
#[derive(Clone, Copy, Debug)]
pub struct GroundContact {
    /// The entity of the ground body, when it has one.
    pub entity: Option<Entity>,
    /// World-space contact point on the ground.
    pub point: Vec3,
    /// World-space contact normal, pointing away from the ground.
    pub normal: Vec3,
    /// The ground body's velocity at the contact point (moving platforms).
    pub velocity: Vec3,
}

/// What a [`GroundProbe`]'s body stands on.
#[derive(Clone, Copy, Debug, Default)]
pub enum GroundState {
    #[default]
    /// Touching nothing within the probe's `max_separation`.
    InAir,
    /// Standing on walkable ground.
    OnGround(GroundContact),
    /// Touching ground steeper than the probe's `max_slope_angle`.
    OnSteepGround(GroundContact),
}

impl GroundState {
    /// The ground contact, or `None` in the air.
    pub fn contact(&self) -> Option<&GroundContact> {
        match self {
            Self::InAir => None,
            Self::OnGround(contact) | Self::OnSteepGround(contact) => Some(contact),
        }
    }

    /// Whether the body stands on walkable ground; steep ground does not count.
    pub fn is_grounded(&self) -> bool {
        matches!(self, Self::OnGround(_))
    }
}

/// Samples what the entity's body stands on each fixed tick; read the result
/// with [`ground`](GroundProbe::ground).
#[derive(Component)]
pub struct GroundProbe {
    /// Radians from horizontal; steeper contacts report
    /// [`GroundState::OnSteepGround`].
    pub max_slope_angle: f32,
    /// How far below the shape a contact still counts as touching.
    pub max_separation: f32,
    pub(crate) state: GroundState,
}

impl GroundProbe {
    /// What the body stood on at the last fixed step.
    pub fn ground(&self) -> &GroundState {
        &self.state
    }

    /// Whether the body stood on walkable ground at the last fixed step.
    pub fn is_grounded(&self) -> bool {
        self.state.is_grounded()
    }
}

impl Default for GroundProbe {
    fn default() -> Self {
        Self {
            max_slope_angle: 50.0_f32.to_radians(),
            max_separation: 0.05,
            state: GroundState::InAir,
        }
    }
}

pub(crate) fn probe_ground(
    mut query: Query<(&mut GroundProbe, &BodyId)>,
    state: Res<PhysicsState>,
) {
    for (mut probe, body_id) in query.iter_mut() {
        probe.state = state.probe_ground(*body_id, probe.max_separation, probe.max_slope_angle);
    }
}
