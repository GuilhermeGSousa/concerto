use concerto_core::{math::Spring, time::Time};
use concerto_ecs::{Component, Query, ResMut};
use glam::Vec3;

use crate::{body::BodyId, physics_state::PhysicsState};

#[derive(Component)]
pub struct CharacterMovement {
    translation_sprint: Spring<Vec3>,
    target_velocity: Vec3,
}

impl CharacterMovement {
    pub fn new(halflife: f32) -> Self {
        Self {
            translation_sprint: Spring::critically_damped_with_halflife(Vec3::ZERO, halflife),
            target_velocity: Vec3::ZERO,
        }
    }

    pub(crate) fn update_velocity(&mut self, dt: f32) -> Vec3 {
        self.translation_sprint.update(self.target_velocity, dt)
    }

    pub fn current_velocity(&self) -> Vec3 {
        self.translation_sprint.value
    }

    pub fn set_target_velocity(&mut self, target_velocity: Vec3) {
        self.target_velocity = target_velocity;
    }
}

pub(crate) fn apply_character_movement(
    bodies: Query<(&mut CharacterMovement, &BodyId)>,
    mut physics: ResMut<PhysicsState>,
) {
    for (mut character_movement, body_id) in bodies.iter() {
        let current_velocity = physics.linear_velocity(*body_id);

        let mut updated_velocity =
            character_movement.update_velocity(Time::fixed_delta_time().as_secs_f32());

        updated_velocity.y = current_velocity.y;
        physics.set_linear_velocity(*body_id, updated_velocity);
    }
}
