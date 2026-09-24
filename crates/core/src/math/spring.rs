//! Damped-spring smoothing for gameplay values (movement, cameras, ...).

use glam::{Quat, Vec2, Vec3, Vec4};

/// A value a [`Spring`] can animate: `f32`, the glam vectors, or [`Quat`].
pub trait SpringValue: Copy {
    fn add(self, rhs: Self) -> Self;
    fn sub(self, rhs: Self) -> Self;
    fn scale(self, factor: f32) -> Self;

    /// The representation of `self` nearest `reference` — quaternions pick
    /// between `q` and `-q` (the same rotation, double cover).
    fn align_to(self, _reference: Self) -> Self {
        self
    }

    /// Projects back onto the valid manifold — quaternions renormalize.
    fn constrain(self) -> Self {
        self
    }
}

macro_rules! impl_spring_value {
    ($($t:ty),*) => {$(
        impl SpringValue for $t {
            fn add(self, rhs: Self) -> Self {
                self + rhs
            }

            fn sub(self, rhs: Self) -> Self {
                self - rhs
            }

            fn scale(self, factor: f32) -> Self {
                self * factor
            }
        }
    )*};
}

impl_spring_value!(f32, Vec2, Vec3, Vec4);

impl SpringValue for Quat {
    fn add(self, rhs: Self) -> Self {
        self + rhs
    }

    fn sub(self, rhs: Self) -> Self {
        self - rhs
    }

    fn scale(self, factor: f32) -> Self {
        self * factor
    }

    fn align_to(self, reference: Self) -> Self {
        if self.dot(reference) < 0.0 {
            -self
        } else {
            self
        }
    }

    fn constrain(self) -> Self {
        self.normalize()
    }
}

/// A damped harmonic oscillator tracking a moving target.
///
/// `update` uses the oscillator's closed-form solution, so the spring is
/// unconditionally stable and framerate independent: two 8 ms steps land
/// exactly where one 16 ms step does.
///
/// ```
/// use concerto_core::math::Spring;
/// use glam::Vec3;
///
/// let mut position = Spring::critically_damped(Vec3::ZERO, 8.0);
/// let smoothed = position.update(Vec3::new(0.0, 0.0, 5.0), 1.0 / 60.0);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Spring<T: SpringValue> {
    pub value: T,
    pub velocity: T,
    /// Natural (undamped) angular frequency in rad/s; higher is snappier.
    pub angular_frequency: f32,
    /// 1 = critically damped (fastest approach without overshoot), < 1
    /// overshoots and oscillates, > 1 approaches more sluggishly.
    pub damping_ratio: f32,
}

impl<T: SpringValue> Spring<T> {
    pub fn new(value: T, angular_frequency: f32, damping_ratio: f32) -> Self {
        Self {
            value,
            velocity: value.sub(value),
            angular_frequency,
            damping_ratio,
        }
    }

    pub fn critically_damped(value: T, angular_frequency: f32) -> Self {
        Self::new(value, angular_frequency, 1.0)
    }

    /// Critically damped, tuned so the remaining distance to the target
    /// halves every `halflife` seconds (when starting at rest).
    pub fn critically_damped_with_halflife(value: T, halflife: f32) -> Self {
        // Root of (1 + x)e^{-x} = 1/2: the ω·t at which a critically damped
        // spring from rest has covered half its displacement.
        const OMEGA_TIMES_HALFLIFE: f32 = 1.678_347;
        Self::critically_damped(value, OMEGA_TIMES_HALFLIFE / halflife.max(1.0e-5))
    }

    pub fn update(&mut self, target: T, dt: f32) -> T {
        let c = Coefficients::compute(self.angular_frequency, self.damping_ratio, dt);
        let target = target.align_to(self.value);
        let displacement = self.value.sub(target);

        let value = target
            .add(displacement.scale(c.pos_pos))
            .add(self.velocity.scale(c.pos_vel));
        self.velocity = displacement
            .scale(c.vel_pos)
            .add(self.velocity.scale(c.vel_vel));
        self.value = value.constrain();
        self.value
    }

    /// Teleports to `value`, zeroing the velocity.
    pub fn reset_to(&mut self, value: T) {
        self.velocity = value.sub(value);
        self.value = value;
    }
}

/// Update matrix advancing (displacement, velocity) of `x'' = -ω²x - 2ζωx'`
/// by `dt`, per damping regime.
struct Coefficients {
    pos_pos: f32,
    pos_vel: f32,
    vel_pos: f32,
    vel_vel: f32,
}

impl Coefficients {
    fn compute(angular_frequency: f32, damping_ratio: f32, dt: f32) -> Self {
        let w = angular_frequency;
        let zeta = damping_ratio.max(0.0);

        if w < f32::EPSILON || dt <= 0.0 {
            return Self {
                pos_pos: 1.0,
                pos_vel: 0.0,
                vel_pos: 0.0,
                vel_vel: 1.0,
            };
        }

        if (zeta - 1.0).abs() < 1.0e-4 {
            let e = (-w * dt).exp();
            Self {
                pos_pos: e * (1.0 + w * dt),
                pos_vel: e * dt,
                vel_pos: -e * w * w * dt,
                vel_vel: e * (1.0 - w * dt),
            }
        } else if zeta < 1.0 {
            let wz = w * zeta;
            let alpha = w * (1.0 - zeta * zeta).sqrt();
            let e = (-wz * dt).exp();
            let (s, c) = (alpha * dt).sin_cos();
            let s_over_alpha = s / alpha;
            Self {
                pos_pos: e * (c + wz * s_over_alpha),
                pos_vel: e * s_over_alpha,
                vel_pos: -e * w * w * s_over_alpha,
                vel_vel: e * (c - wz * s_over_alpha),
            }
        } else {
            let zb = w * (zeta * zeta - 1.0).sqrt();
            let z1 = -w * zeta - zb;
            let z2 = -w * zeta + zb;
            let e1 = (z1 * dt).exp();
            let e2 = (z2 * dt).exp();
            let inv = 1.0 / (z2 - z1);
            Self {
                pos_pos: (z2 * e1 - z1 * e2) * inv,
                pos_vel: (e2 - e1) * inv,
                vel_pos: z1 * z2 * (e1 - e2) * inv,
                vel_vel: (z2 * e2 - z1 * e1) * inv,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn converges_to_target() {
        let mut spring = Spring::critically_damped(0.0f32, 10.0);
        for _ in 0..120 {
            spring.update(10.0, DT);
        }
        assert!((spring.value - 10.0).abs() < 0.01, "was {}", spring.value);
        assert!(spring.velocity.abs() < 0.1, "was {}", spring.velocity);
    }

    #[test]
    fn framerate_independent() {
        let mut coarse = Spring::new(0.0f32, 6.0, 0.5);
        let mut fine = coarse;

        coarse.update(1.0, 0.1);
        for _ in 0..10 {
            fine.update(1.0, 0.01);
        }

        assert!(
            (coarse.value - fine.value).abs() < 1.0e-4,
            "one 100 ms step ({}) should equal ten 10 ms steps ({})",
            coarse.value,
            fine.value
        );
        assert!((coarse.velocity - fine.velocity).abs() < 1.0e-3);
    }

    #[test]
    fn damping_regimes() {
        let mut under = Spring::new(0.0f32, 10.0, 0.2);
        let mut critical = Spring::new(0.0f32, 10.0, 1.0);
        let mut over = Spring::new(0.0f32, 10.0, 3.0);

        let mut under_peak = 0.0f32;
        for _ in 0..180 {
            under_peak = under_peak.max(under.update(1.0, DT));
            assert!(critical.update(1.0, DT) <= 1.0 + 1.0e-4);
            assert!(over.update(1.0, DT) <= 1.0 + 1.0e-4);
        }

        assert!(
            under_peak > 1.1,
            "underdamped should overshoot, peaked at {under_peak}"
        );
        assert!(
            over.value < critical.value,
            "overdamped ({}) should trail critically damped ({})",
            over.value,
            critical.value
        );
    }

    #[test]
    fn halflife_halves_displacement() {
        let mut spring = Spring::critically_damped_with_halflife(0.0f32, 0.25);
        spring.update(1.0, 0.25);
        assert!(
            (spring.value - 0.5).abs() < 1.0e-3,
            "after one halflife the spring should sit halfway, was {}",
            spring.value
        );
    }

    #[test]
    fn vec3_converges() {
        let target = Vec3::new(1.0, 2.0, 3.0);
        let mut spring = Spring::critically_damped(Vec3::ZERO, 10.0);
        for _ in 0..120 {
            spring.update(target, DT);
        }
        assert!(spring.value.distance(target) < 0.01, "was {}", spring.value);
    }

    #[test]
    fn quat_handles_double_cover() {
        let rotation = Quat::from_rotation_y(1.0);
        // -rotation is the same orientation; the spring must take the short
        // way there instead of swinging through a full turn.
        let mut spring = Spring::critically_damped(Quat::IDENTITY, 10.0);
        for _ in 0..120 {
            spring.update(-rotation, DT);
        }
        assert!(
            spring.value.angle_between(rotation) < 0.01,
            "was {:?}",
            spring.value
        );
        assert!(
            (spring.value.length() - 1.0).abs() < 1.0e-4,
            "value should stay normalized"
        );
    }

    #[test]
    fn zero_dt_and_zero_frequency_are_noops() {
        let mut spring = Spring::critically_damped(2.0f32, 10.0);
        spring.update(5.0, 0.0);
        assert_eq!(spring.value, 2.0);

        let mut inert = Spring::critically_damped(2.0f32, 0.0);
        inert.update(5.0, DT);
        assert_eq!(inert.value, 2.0);
    }
}
