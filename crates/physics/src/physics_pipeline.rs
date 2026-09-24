use concerto_ecs::resource::Resource;
use concerto_foundation::time::Time;

use crate::backend::PhysicsBackend;
use crate::physics_state::PhysicsState;
use crate::ActiveBackend;

/// Per-step scratch resources for advancing the simulation (what these are is
/// up to the backend — e.g. a temp allocator and job system for Jolt). Owns
/// them for the lifetime of the app, separate from [`PhysicsState`] so
/// stepping can borrow both.
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

    /// Advances `state` by one fixed timestep.
    pub fn step(&mut self, state: &mut PhysicsState) {
        state
            .backend_mut()
            .step(&mut self.stepper, Time::fixed_delta_time().as_secs_f32());
    }
}

impl Default for PhysicsPipeline {
    fn default() -> Self {
        Self::new()
    }
}
