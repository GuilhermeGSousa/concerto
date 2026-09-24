use std::time::Duration;

use concerto_ecs::resource::Resource;

mod frame_stats;
mod instant;

pub use frame_stats::FrameStats;

use crate::time::instant::Instant;

#[derive(Resource)]
pub struct Time {
    last_update: Instant,
    delta: Duration,
    fixed_overstep: Duration,
    accumulated_fixed_time: Duration,
}

impl Time {
    const FIXED_DELTA_TIME: Duration = Duration::from_millis(33);

    pub fn new() -> Self {
        Self {
            last_update: Instant::now(),
            delta: Duration::default(),
            fixed_overstep: Duration::default(),
            accumulated_fixed_time: Duration::default(),
        }
    }

    pub fn delta(&self) -> Duration {
        self.delta
    }

    pub fn accumulate_fixed_time(&mut self) {
        self.accumulated_fixed_time += self.delta();
    }

    pub fn expend_fixed_time(&mut self) -> bool {
        let result = self.accumulated_fixed_time >= Time::fixed_delta_time();

        if result {
            self.accumulated_fixed_time -= Time::fixed_delta_time();
        } else {
            self.fixed_overstep = self.accumulated_fixed_time;
        }
        result
    }

    pub fn update(&mut self) {
        self.delta = Instant::now() - self.last_update;
        self.last_update = Instant::now();
    }

    pub fn fixed_delta_time() -> Duration {
        Self::FIXED_DELTA_TIME
    }

    /// Seconds elapsed since the last fixed step. The physics world is frozen between
    /// steps, so systems running at frame rate must extrapolate by this to see current state.
    pub fn fixed_overstep(&self) -> Duration {
        self.fixed_overstep
    }

    /// Fraction of the way from the last fixed step to the next, for interpolating
    /// fixed-step state into frame-rate rendering.
    pub fn fixed_alpha(&self) -> f32 {
        (self.fixed_overstep.as_secs_f32() / Self::FIXED_DELTA_TIME.as_secs_f32()).clamp(0.0, 1.0)
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}
