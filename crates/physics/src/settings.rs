use concerto_ecs::resource::Resource;
use concerto_foundation::time::Time;

/// How the simulation divides each fixed step.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PhysicsSettings {
    /// The longest collision step in seconds; a fixed step is split into the fewest equal steps no longer than this.
    pub max_collision_step: f32,
}

impl PhysicsSettings {
    /// The number of collision steps one fixed step is split into.
    pub fn collision_steps(&self) -> u32 {
        collision_steps_for(
            Time::fixed_delta_time().as_secs_f32(),
            self.max_collision_step,
        )
    }
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            max_collision_step: 1.0 / 60.0,
        }
    }
}

fn collision_steps_for(fixed_step: f32, max_collision_step: f32) -> u32 {
    if max_collision_step.is_nan() || max_collision_step <= 0.0 {
        return 1;
    }
    (fixed_step / max_collision_step - 1e-4)
        .ceil()
        .clamp(1.0, 64.0) as u32
}
