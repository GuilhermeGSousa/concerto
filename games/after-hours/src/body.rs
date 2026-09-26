//! Kinematic-feeling movement for physics capsules, and small vector helpers.
use concerto::{
    ecs::{Component, Query, ResMut},
    foundation::time::Time,
    physics::{body::BodyId, physics_state::PhysicsState},
};
use glam::Vec3;

/// Horizontal movement intent, applied to the physics body every fixed step.
#[derive(Component, Default)]
pub struct Mover {
    /// Velocity the body is trying to reach.
    pub desired: Vec3,
    /// Velocity actually applied last step, easing toward `desired`.
    current: Vec3,
    /// How quickly `current` chases `desired`, in 1/s.
    pub responsiveness: f32,
}

impl Mover {
    pub fn new(responsiveness: f32) -> Self {
        Self {
            responsiveness,
            ..Default::default()
        }
    }

    pub fn velocity(&self) -> Vec3 {
        self.current
    }
}

/// Fixed step: eases each body toward its desired horizontal velocity,
/// leaving gravity's vertical component alone.
pub fn apply_movers(movers: Query<(&mut Mover, &BodyId)>, mut physics: ResMut<PhysicsState>) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    for (mut mover, body) in movers.iter() {
        let blend = 1.0 - (-mover.responsiveness * dt).exp();
        let desired = mover.desired;
        mover.current = mover.current.lerp(desired, blend);
        let vertical = physics.linear_velocity(*body).y;
        let horizontal = mover.current;
        physics.set_linear_velocity(*body, Vec3::new(horizontal.x, vertical, horizontal.z));
    }
}

pub fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Yaw (around +Y) that turns the mannequin's +Z forward toward `dir`.
pub fn yaw_toward(dir: Vec3) -> f32 {
    dir.x.atan2(dir.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn yaw_toward_turns_plus_z_onto_the_direction() {
        for dir in [Vec3::X, Vec3::NEG_X, Vec3::new(1.0, 0.0, -1.0).normalize()] {
            let turned = Quat::from_rotation_y(yaw_toward(dir)) * Vec3::Z;
            assert!((turned - dir).length() < 1e-5);
        }
    }
}
