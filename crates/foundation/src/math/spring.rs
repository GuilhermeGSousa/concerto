//! Damped-spring smoothing for gameplay values (movement, cameras, ...).

use glam::{Quat, Vec2, Vec3, Vec4};

use super::spring_damper::{from_scaled_angle_axis, quat_abs, to_scaled_angle_axis};

/// A vector a [`Spring`] measures displacement and velocity in: `f32` or a glam vector.
pub trait SpringVector: Copy + Default {
    fn add(self, rhs: Self) -> Self;
    fn sub(self, rhs: Self) -> Self;
    fn scale(self, factor: f32) -> Self;
}

macro_rules! impl_spring_vector {
    ($($t:ty),*) => {$(
        impl SpringVector for $t {
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

impl_spring_vector!(f32, Vec2, Vec3, Vec4);

/// A value a [`Spring`] can animate: a [`SpringVector`], or a [`Quat`] whose velocity is a scaled angle-axis.
pub trait SpringValue: Copy {
    type Velocity: SpringVector;

    /// How far `self` sits from `target`.
    fn displacement(self, target: Self) -> Self::Velocity;

    /// The value `displacement` away from `self`.
    fn displaced(self, displacement: Self::Velocity) -> Self;
}

impl<T: SpringVector> SpringValue for T {
    type Velocity = T;

    fn displacement(self, target: Self) -> T {
        self.sub(target)
    }

    fn displaced(self, displacement: T) -> Self {
        self.add(displacement)
    }
}

impl SpringValue for Quat {
    type Velocity = Vec3;

    fn displacement(self, target: Self) -> Vec3 {
        to_scaled_angle_axis(quat_abs(self * target.inverse()))
    }

    fn displaced(self, displacement: Vec3) -> Self {
        from_scaled_angle_axis(displacement) * self
    }
}

/// A damped harmonic oscillator tracking a moving target.
///
/// `update` uses the oscillator's closed-form solution, so the spring is
/// unconditionally stable and framerate independent: two 8 ms steps land
/// exactly where one 16 ms step does.
///
/// ```
/// use concerto_foundation::math::Spring;
/// use glam::Vec3;
///
/// let mut position = Spring::critically_damped(Vec3::ZERO, 8.0);
/// let smoothed = position.update(Vec3::new(0.0, 0.0, 5.0), 1.0 / 60.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring<T: SpringValue> {
    pub value: T,
    pub velocity: T::Velocity,
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
            velocity: T::Velocity::default(),
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
        let displacement = self.value.displacement(target);

        self.value = target.displaced(
            displacement
                .scale(c.pos_pos)
                .add(self.velocity.scale(c.pos_vel)),
        );
        self.velocity = displacement
            .scale(c.vel_pos)
            .add(self.velocity.scale(c.vel_vel));
        self.value
    }

    /// Teleports to `value`, zeroing the velocity.
    pub fn reset_to(&mut self, value: T) {
        self.velocity = T::Velocity::default();
        self.value = value;
    }
}

impl<T: SpringVector> Spring<T> {
    /// The integral of `value` over the next `dt` seconds of tracking `target`, in closed form; the spring does not move.
    pub fn integral(&self, target: T, dt: f32) -> T {
        let w = self.angular_frequency;
        if w < f32::EPSILON {
            return self.value.scale(dt);
        }
        let mut next = *self;
        next.update(target, dt);
        let damping = 2.0 * self.damping_ratio.max(0.0) * w;
        target.scale(dt).sub(
            next.velocity
                .sub(self.velocity)
                .add(next.value.sub(self.value).scale(damping))
                .scale(1.0 / (w * w)),
        )
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
    fn quat_is_framerate_independent() {
        let target = Quat::from_rotation_y(2.5);
        let mut coarse = Spring::new(Quat::IDENTITY, 6.0, 1.0);
        let mut fine = coarse;

        coarse.update(target, 0.2);
        for _ in 0..20 {
            fine.update(target, 0.01);
        }

        assert!(
            coarse.value.abs_diff_eq(fine.value, 1.0e-4),
            "one 200 ms step ({:?}) should equal twenty 10 ms steps ({:?})",
            coarse.value,
            fine.value
        );
    }

    #[test]
    fn quat_velocity_is_a_scaled_angle_axis() {
        let mut spring = Spring::critically_damped(Quat::IDENTITY, 10.0);

        spring.update(Quat::from_rotation_y(1.0), 0.1);

        assert!(
            spring
                .value
                .abs_diff_eq(Quat::from_rotation_y(0.264_241), 1.0e-5),
            "was {:?}",
            spring.value
        );
        assert!(
            spring
                .velocity
                .abs_diff_eq(Vec3::new(0.0, 3.678_794, 0.0), 1.0e-5),
            "was {:?}",
            spring.velocity
        );
    }

    #[test]
    fn integral_of_a_critically_damped_spring_from_rest() {
        let spring = Spring::critically_damped(0.0f32, 2.0);

        assert!((spring.integral(1.0, 1.0) - 0.270_671).abs() < 1.0e-5);
    }

    #[test]
    fn integral_is_the_area_under_the_value_in_every_damping_regime() {
        const STEPS: usize = 700;
        let dt = 0.7 / STEPS as f32;

        for damping_ratio in [0.4, 1.0, 2.5] {
            let mut spring = Spring::new(1.0f32, 6.0, damping_ratio);
            spring.velocity = -2.0;
            let integral = spring.integral(3.0, 0.7);

            let mut area = 0.0;
            let mut previous = spring.value;
            for _ in 0..STEPS {
                let next = spring.update(3.0, dt);
                area += (previous + next) * 0.5 * dt;
                previous = next;
            }

            assert!(
                (integral - area).abs() < 1.0e-3,
                "damping {damping_ratio}: {integral} vs {area}"
            );
        }
    }

    #[test]
    fn integral_of_an_inert_spring_is_the_held_value() {
        let spring = Spring::critically_damped(2.0f32, 0.0);

        assert_eq!(spring.integral(5.0, 0.5), 1.0);
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
