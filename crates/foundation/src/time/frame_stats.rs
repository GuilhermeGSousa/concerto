use std::time::Duration;

use concerto_ecs::resource::Resource;

/// Rolling window of recent CPU frame times, for FPS displays and quick
/// performance sanity checks (see docs/profiling.md).
///
/// Updated once per frame by the `TimePlugin`; read it anywhere you want to
/// show or log frame timing.
#[derive(Resource)]
pub struct FrameStats {
    /// Frame times in milliseconds, ring buffer.
    samples: [f32; Self::WINDOW],
    head: usize,
    len: usize,
    /// Seconds since the last one-per-second summary was emitted.
    summary_timer: f32,
}

impl FrameStats {
    /// Number of frames kept in the rolling window (~2 s at 60 FPS).
    pub const WINDOW: usize = 120;

    pub fn new() -> Self {
        Self {
            samples: [0.0; Self::WINDOW],
            head: 0,
            len: 0,
            summary_timer: 0.0,
        }
    }

    /// Records one frame's delta time.
    pub fn push(&mut self, delta: Duration) {
        self.samples[self.head] = delta.as_secs_f32() * 1000.0;
        self.head = (self.head + 1) % Self::WINDOW;
        self.len = (self.len + 1).min(Self::WINDOW);
    }

    /// Advances the summary timer; returns `true` at most once per second.
    /// Callers use this to throttle logging/redraws of the stats.
    pub fn tick_summary(&mut self, delta: Duration) -> bool {
        self.summary_timer += delta.as_secs_f32();
        if self.summary_timer >= 1.0 {
            self.summary_timer = 0.0;
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn window(&self) -> &[f32] {
        &self.samples[..self.len]
    }

    /// Mean frame time over the window, in milliseconds.
    pub fn average_ms(&self) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        self.window().iter().sum::<f32>() / self.len as f32
    }

    /// Worst frame time in the window, in milliseconds.
    pub fn max_ms(&self) -> f32 {
        self.window().iter().copied().fold(0.0, f32::max)
    }

    /// Frame time at the given percentile (0.0–1.0) over the window,
    /// in milliseconds. E.g. `percentile_ms(0.99)` for p99.
    pub fn percentile_ms(&self, percentile: f32) -> f32 {
        if self.len == 0 {
            return 0.0;
        }
        let mut sorted: Vec<f32> = self.window().to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let idx = ((sorted.len() as f32 - 1.0) * percentile.clamp(0.0, 1.0)).round() as usize;
        sorted[idx]
    }

    /// Average frames per second over the window.
    pub fn fps(&self) -> f32 {
        let avg = self.average_ms();
        if avg <= 0.0 {
            0.0
        } else {
            1000.0 / avg
        }
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_over_uniform_frames() {
        let mut stats = FrameStats::new();
        for _ in 0..200 {
            stats.push(Duration::from_millis(10));
        }
        assert_eq!(stats.average_ms(), 10.0);
        assert_eq!(stats.max_ms(), 10.0);
        assert_eq!(stats.percentile_ms(0.99), 10.0);
        assert_eq!(stats.fps(), 100.0);
    }

    #[test]
    fn percentile_picks_spikes() {
        let mut stats = FrameStats::new();
        for i in 0..FrameStats::WINDOW {
            let ms = if i % 60 == 0 { 50 } else { 10 };
            stats.push(Duration::from_millis(ms));
        }
        assert_eq!(stats.percentile_ms(0.5), 10.0);
        assert_eq!(stats.max_ms(), 50.0);
    }

    #[test]
    fn summary_ticks_once_per_second() {
        let mut stats = FrameStats::new();
        let mut ticks = 0;
        // 0.25 is exactly representable, so 4 calls accumulate to exactly 1.0.
        for _ in 0..12 {
            if stats.tick_summary(Duration::from_millis(250)) {
                ticks += 1;
            }
        }
        assert_eq!(ticks, 3);
    }
}
