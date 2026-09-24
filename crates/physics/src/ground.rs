use concerto_ecs::{component::Component, entity::Entity, query::Query, resource::Res};
use glam::Vec3;

use crate::{body::BodyId, physics_state::PhysicsState};

/// One ground contact; only exists while touching something.
#[derive(Clone, Copy, Debug)]
pub struct GroundContact {
    pub entity: Option<Entity>,
    pub point: Vec3,
    pub normal: Vec3,
    /// The ground body's velocity at the contact point (moving platforms).
    pub velocity: Vec3,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum GroundState {
    #[default]
    InAir,
    /// Standing on walkable ground.
    OnGround(GroundContact),
    /// Touching ground steeper than the probe's `max_slope_angle`.
    OnSteepGround(GroundContact),
}

impl GroundState {
    pub fn contact(&self) -> Option<&GroundContact> {
        match self {
            Self::InAir => None,
            Self::OnGround(contact) | Self::OnSteepGround(contact) => Some(contact),
        }
    }

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
    pub fn ground(&self) -> &GroundState {
        &self.state
    }

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

pub(crate) fn probe_ground(query: Query<(&mut GroundProbe, &BodyId)>, state: Res<PhysicsState>) {
    for (mut probe, body_id) in query.iter() {
        probe.state = state.probe_ground(*body_id, probe.max_separation, probe.max_slope_angle);
    }
}
