use concerto_ecs::resource::Resource;
use concerto_foundation::time::Time;

use crate::backend::PhysicsBackend;
use crate::physics_state::PhysicsState;
use crate::settings::PhysicsSettings;
use crate::ActiveBackend;

/// Per-step scratch resources for advancing the simulation.
///
/// What these are is up to the backend: for Jolt, a temp allocator and a job
/// system. It owns them for the lifetime of the app, separate from
/// [`PhysicsState`] so stepping can borrow both.
#[derive(Resource)]
pub struct PhysicsPipeline {
    stepper: <ActiveBackend as PhysicsBackend>::Stepper,
}

impl PhysicsPipeline {
    pub fn new() -> Self {
        PhysicsPipeline {
            stepper: ActiveBackend::new_stepper(),
        }
    }

    /// Advances `state` by one fixed timestep under the default `PhysicsSettings`.
    pub fn step(&mut self, state: &mut PhysicsState) {
        self.step_with(state, &PhysicsSettings::default());
    }

    /// Advances `state` by one fixed timestep, split into the collision steps `settings` asks for.
    pub fn step_with(&mut self, state: &mut PhysicsState, settings: &PhysicsSettings) {
        state.backend_mut().step(
            &mut self.stepper,
            Time::fixed_delta_time().as_secs_f32(),
            settings.collision_steps(),
        );
        state.gather_sensor_contacts();
        state.gather_contacts();
    }
}

impl Default for PhysicsPipeline {
    fn default() -> Self {
        Self::new()
    }
}
