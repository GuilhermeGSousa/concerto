use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

use super::spring::SpringVector;

const LN_2: f32 = std::f32::consts::LN_2;
const EPSILON: f32 = 1.0e-5;

/// The damping coefficient whose exact decay halves a value every `halflife` seconds.
pub fn halflife_to_damping(halflife: f32) -> f32 {
    4.0 * LN_2 / (halflife + EPSILON)
}

/// The halflife, in seconds, of a decay with the given damping coefficient.
pub fn damping_to_halflife(damping: f32) -> f32 {
    4.0 * LN_2 / (damping + EPSILON)
}

/// Decays an offset and its velocity to zero; a rest offset halves every `halflife`.
pub fn decay_spring_damper_exact<T: SpringVector>(x: T, v: T, halflife: f32, dt: f32) -> (T, T) {
    let y = halflife_to_damping(halflife) / 2.0;
    let j1 = v.add(x.scale(y));
    let eydt = (-y * dt).exp();
    (
        x.add(j1.scale(dt)).scale(eydt),
        v.sub(j1.scale(y * dt)).scale(eydt),
    )
}

/// Critically damped spring toward a fixed `goal`, returning the new position and velocity.
pub fn simple_spring_damper_exact<T: SpringVector>(
    x: T,
    v: T,
    goal: T,
    halflife: f32,
    dt: f32,
) -> (T, T) {
    let y = halflife_to_damping(halflife) / 2.0;
    let j0 = x.sub(goal);
    let j1 = v.add(j0.scale(y));
    let eydt = (-y * dt).exp();
    (
        j0.add(j1.scale(dt)).scale(eydt).add(goal),
        v.sub(j1.scale(y * dt)).scale(eydt),
    )
}

/// [`simple_spring_damper_exact`] for rotations; the velocity is a scaled angle-axis.
pub fn simple_spring_damper_exact_quat(
    x: Quat,
    v: Vec3,
    goal: Quat,
    halflife: f32,
    dt: f32,
) -> (Quat, Vec3) {
    let y = halflife_to_damping(halflife) / 2.0;
    let j0 = to_scaled_angle_axis(quat_abs(x * goal.inverse()));
    let j1 = v + j0 * y;
    let eydt = (-y * dt).exp();
    (
        from_scaled_angle_axis(eydt * (j0 + j1 * dt)) * goal,
        eydt * (v - j1 * y * dt),
    )
}

/// Position, velocity and acceleration of a character driven by [`spring_character_update`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Default)]
pub struct CharacterState {
    pub position: Vec3,
    pub velocity: Vec3,
    pub acceleration: Vec3,
}

/// Advances a critically damped character controller whose velocity approaches `desired_velocity`.
pub fn spring_character_update(
    state: CharacterState,
    desired_velocity: Vec3,
    halflife: f32,
    dt: f32,
) -> CharacterState {
    let y = halflife_to_damping(halflife) / 2.0;
    let j0 = state.velocity - desired_velocity;
    let j1 = state.acceleration + j0 * y;
    let eydt = (-y * dt).exp();
    CharacterState {
        position: eydt * ((-j1) / (y * y) + (-j0 - j1 * dt) / y)
            + j1 / (y * y)
            + j0 / y
            + desired_velocity * dt
            + state.position,
        velocity: eydt * (j0 + j1 * dt) + desired_velocity,
        acceleration: eydt * (state.acceleration - j1 * y * dt),
    }
}

/// The states `dt`, `2 * dt`, ... `count * dt` seconds ahead, in closed form, each equal to `i` steps of [`spring_character_update`].
pub fn spring_character_predict(
    state: CharacterState,
    desired_velocity: Vec3,
    halflife: f32,
    dt: f32,
    count: usize,
) -> Vec<CharacterState> {
    (1..=count)
        .map(|i| spring_character_update(state, desired_velocity, halflife, dt * i as f32))
        .collect()
}

/// The part of `goal` an adjustment closes in `dt` when it halves every `halflife`.
pub fn damp_adjustment_exact(goal: Vec3, halflife: f32, dt: f32, eps: f32) -> Vec3 {
    goal * (1.0 - (-(LN_2 * dt) / (halflife + eps)).exp())
}

/// The same rotation as `q` with a non-negative `w`, the shortest arc from identity.
pub fn quat_abs(q: Quat) -> Quat {
    if q.w < 0.0 {
        -q
    } else {
        q
    }
}

/// The rotation vector of `q`: its axis scaled by its angle in radians.
pub fn to_scaled_angle_axis(q: Quat) -> Vec3 {
    let axis = Vec3::new(q.x, q.y, q.z);
    let length = axis.length();
    if length < 1.0e-8 {
        return axis * 2.0;
    }
    2.0 * q.w.clamp(-1.0, 1.0).acos() * axis / length
}

/// The rotation about `v / |v|` by `|v|` radians.
pub fn from_scaled_angle_axis(v: Vec3) -> Quat {
    let half = v / 2.0;
    let half_angle = half.length();
    if half_angle < 1.0e-8 {
        return Quat::from_xyzw(half.x, half.y, half.z, 1.0).normalize();
    }
    let scale = half_angle.sin() / half_angle;
    Quat::from_xyzw(
        half.x * scale,
        half.y * scale,
        half.z * scale,
        half_angle.cos(),
    )
}

/// The angular velocity, as a scaled angle-axis, that turns `current` into `next` in `dt` seconds.
pub fn angular_velocity(next: Quat, current: Quat, dt: f32) -> Vec3 {
    to_scaled_angle_axis(quat_abs(next * current.inverse())) / dt
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn assert_close(a: Vec3, b: Vec3, tolerance: f32) {
        assert!(a.distance(b) < tolerance, "{a:?} vs {b:?}");
    }

    #[test]
    fn halflife_and_damping_round_trip() {
        let damping = halflife_to_damping(0.25);
        assert!((damping_to_halflife(damping) - 0.25).abs() < 1.0e-3);
    }

    #[test]
    fn decay_matches_the_closed_form() {
        let (x, v) = decay_spring_damper_exact(1.0f32, 0.0, 0.25, 0.25);
        let y = halflife_to_damping(0.25) / 2.0;
        assert!((x - (1.0 + 0.25 * y) * (-y * 0.25).exp()).abs() < 1.0e-6);
        assert!(v < 0.0);
        let (x, _) =
            decay_spring_damper_exact(1.0f32, -halflife_to_damping(0.25) / 2.0, 0.25, 0.25);
        assert!((x - 0.25).abs() < 1.0e-3, "was {x}");
    }

    #[test]
    fn decay_is_framerate_independent() {
        let (coarse_x, coarse_v) = decay_spring_damper_exact(Vec3::X, Vec3::Y, 0.3, 0.2);
        let (mut x, mut v) = (Vec3::X, Vec3::Y);
        for _ in 0..20 {
            (x, v) = decay_spring_damper_exact(x, v, 0.3, 0.01);
        }
        assert_close(coarse_x, x, 1.0e-4);
        assert_close(coarse_v, v, 1.0e-3);
    }

    #[test]
    fn simple_spring_halves_distance_over_one_halflife_from_rest() {
        let (x, _) = simple_spring_damper_exact(0.0f32, 0.0, 1.0, 0.25, 0.25);
        let (decayed, _) = decay_spring_damper_exact(-1.0f32, 0.0, 0.25, 0.25);
        assert!((x - (1.0 + decayed)).abs() < 1.0e-6);
        assert!(x > 0.0 && x < 1.0);
    }

    #[test]
    fn simple_spring_converges_to_goal() {
        let (mut x, mut v) = (Vec3::ZERO, Vec3::ZERO);
        for _ in 0..240 {
            (x, v) = simple_spring_damper_exact(x, v, Vec3::new(1.0, 2.0, 3.0), 0.1, DT);
        }
        assert_close(x, Vec3::new(1.0, 2.0, 3.0), 1.0e-3);
        assert!(v.length() < 1.0e-2);
    }

    #[test]
    fn quat_spring_converges_by_the_short_way() {
        let goal = Quat::from_rotation_y(1.0);
        let (mut x, mut v) = (Quat::IDENTITY, Vec3::ZERO);
        for _ in 0..240 {
            (x, v) = simple_spring_damper_exact_quat(x, v, -goal, 0.1, DT);
        }
        assert!(x.angle_between(goal) < 1.0e-2, "was {x:?}");
        assert!(v.length() < 1.0e-2);
    }

    #[test]
    fn scaled_angle_axis_round_trips() {
        let v = Vec3::new(0.3, -0.5, 0.2);
        assert_close(to_scaled_angle_axis(from_scaled_angle_axis(v)), v, 1.0e-5);
        assert_close(
            to_scaled_angle_axis(Quat::from_rotation_z(0.7)),
            Vec3::new(0.0, 0.0, 0.7),
            1.0e-5,
        );
        assert_eq!(to_scaled_angle_axis(Quat::IDENTITY), Vec3::ZERO);
        assert!(from_scaled_angle_axis(Vec3::ZERO).abs_diff_eq(Quat::IDENTITY, 1.0e-6));
    }

    #[test]
    fn quat_abs_takes_the_shortest_arc() {
        let q = Quat::from_rotation_y(0.4);
        assert_eq!(quat_abs(-q), q);
        assert_eq!(quat_abs(q), q);
    }

    #[test]
    fn angular_velocity_matches_the_rotation_rate() {
        let next = Quat::from_rotation_x(0.1);
        assert_close(
            angular_velocity(next, Quat::IDENTITY, 0.5),
            Vec3::new(0.2, 0.0, 0.0),
            1.0e-5,
        );
        assert_close(
            angular_velocity(-next, Quat::IDENTITY, 0.5),
            Vec3::new(0.2, 0.0, 0.0),
            1.0e-5,
        );
    }

    #[test]
    fn character_velocity_approaches_desired() {
        let mut state = CharacterState {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
        };
        let desired = Vec3::new(2.0, 0.0, 1.0);
        for _ in 0..240 {
            state = spring_character_update(state, desired, 0.2, DT);
        }
        assert_close(state.velocity, desired, 1.0e-3);
        assert!(state.acceleration.length() < 1.0e-2);
    }

    #[test]
    fn character_prediction_matches_repeated_updates() {
        let start = CharacterState {
            position: Vec3::new(1.0, 0.0, -1.0),
            velocity: Vec3::new(0.5, 0.0, 0.2),
            acceleration: Vec3::new(0.1, 0.0, -0.3),
        };
        let desired = Vec3::new(2.0, 0.0, 1.0);
        let predicted = spring_character_predict(start, desired, 0.2, 0.1, 5);
        assert_eq!(predicted.len(), 5);
        let mut state = start;
        for expected in predicted {
            for _ in 0..10 {
                state = spring_character_update(state, desired, 0.2, 0.01);
            }
            assert_close(state.position, expected.position, 1.0e-3);
            assert_close(state.velocity, expected.velocity, 1.0e-3);
            assert_close(state.acceleration, expected.acceleration, 1.0e-3);
        }
    }

    #[test]
    fn damp_adjustment_closes_half_over_one_halflife() {
        let step = damp_adjustment_exact(Vec3::new(2.0, 0.0, 0.0), 0.25, 0.25, 1.0e-8);
        assert_close(step, Vec3::new(1.0, 0.0, 0.0), 1.0e-5);
    }

    #[test]
    fn character_state_reflects_its_fields_and_defaults_to_zero() {
        use concerto_reflect::{with_property, PropertyPath};
        use std::any::Any;

        let state = CharacterState {
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, 6.0),
            acceleration: Vec3::new(7.0, 8.0, 9.0),
        };
        for (name, expected) in [
            ("position", state.position),
            ("velocity", state.velocity),
            ("acceleration", state.acceleration),
        ] {
            let mut seen = None;
            with_property(&state, &PropertyPath::new([name]), &mut |value| {
                seen = (value as &dyn Any).downcast_ref::<Vec3>().copied();
            })
            .unwrap();
            assert_eq!(seen, Some(expected));
        }
        let default = CharacterState::default();
        assert_eq!(default.position, Vec3::ZERO);
        assert_eq!(default.velocity, Vec3::ZERO);
        assert_eq!(default.acceleration, Vec3::ZERO);
    }
}
