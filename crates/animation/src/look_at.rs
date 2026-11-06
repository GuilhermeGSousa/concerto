use glam::{Quat, Vec3};

const MIN_PERPENDICULAR_SQUARED: f32 = 1e-8;

/// The direction a bone looks along and its up, in the bone's own space; orthonormal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookFrame {
    forward: Vec3,
    up: Vec3,
}

impl LookFrame {
    /// `forward` normalized and `up` made perpendicular to it; `None` when either is zero or not finite, or they are parallel.
    pub fn new(forward: Vec3, up: Vec3) -> Option<LookFrame> {
        let forward = forward.try_normalize()?;
        let up = up.try_normalize()?;
        let perpendicular = up - forward * up.dot(forward);
        if perpendicular.length_squared() <= MIN_PERPENDICULAR_SQUARED {
            return None;
        }
        Some(LookFrame {
            forward,
            up: perpendicular.try_normalize()?,
        })
    }

    pub fn forward(&self) -> Vec3 {
        self.forward
    }

    pub fn up(&self) -> Vec3 {
        self.up
    }

    /// `up × forward`: the axis a positive yaw turns `forward` toward.
    pub fn side(&self) -> Vec3 {
        self.up.cross(self.forward)
    }
}

/// A turn of a `LookFrame`, in radians: yaw about its up toward its side, then pitch toward its up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct YawPitch {
    pub yaw: f32,
    pub pitch: f32,
}

/// The yaw and pitch that turn `frame`'s forward onto `direction`, both in the frame's space; zero for a zero direction.
pub fn yaw_pitch_to(frame: &LookFrame, direction: Vec3) -> YawPitch {
    let Some(t) = direction.try_normalize() else {
        return YawPitch::default();
    };
    let along = t.dot(frame.forward());
    let across = t.dot(frame.side());
    let rise = t.dot(frame.up());
    YawPitch {
        yaw: across.atan2(along),
        pitch: rise.atan2((along * along + across * across).sqrt()),
    }
}

/// The rotation, in the frame's space, that turns its forward by `angles`.
pub fn look_rotation(frame: &LookFrame, angles: YawPitch) -> Quat {
    Quat::from_axis_angle(frame.up(), angles.yaw)
        * Quat::from_axis_angle(frame.side(), -angles.pitch)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use glam::Vec3;

    use super::{LookFrame, YawPitch, look_rotation, yaw_pitch_to};

    fn skewed() -> LookFrame {
        LookFrame::new(Vec3::new(0.3, -0.2, 1.0), Vec3::new(0.1, 1.0, 0.4)).unwrap()
    }

    #[test]
    fn look_rotation_turns_forward_onto_the_direction() {
        let frame = skewed();
        let directions = [
            frame.side() * 2.0,
            frame.up() * 0.5,
            -frame.up() * 3.0,
            -frame.forward() - frame.side(),
            Vec3::new(0.7, -1.3, 0.4),
        ];
        for direction in directions {
            let angles = yaw_pitch_to(&frame, direction);
            let turned = look_rotation(&frame, angles) * frame.forward();
            let expected = direction.normalize();
            assert!(
                turned.distance(expected) < 1e-5,
                "{direction}: turned {turned}, expected {expected}"
            );
        }
    }

    #[test]
    fn toward_the_side_yaws_positive_and_toward_up_pitches_a_right_angle() {
        let frame = skewed();

        let side = yaw_pitch_to(&frame, frame.side());
        assert!(side.yaw > 0.0, "{side:?}");
        assert!(side.pitch.abs() < 1e-5, "{side:?}");

        let up = yaw_pitch_to(&frame, frame.up());
        assert!((up.pitch - FRAC_PI_2).abs() < 1e-5, "{up:?}");
        assert!(up.yaw.is_finite(), "{up:?}");
    }

    #[test]
    fn a_frame_needs_a_finite_forward_and_an_up_off_its_line() {
        let forward = Vec3::new(0.3, -0.2, 1.0);
        let up = Vec3::new(0.1, 1.0, 0.4);
        assert_eq!(LookFrame::new(Vec3::ZERO, up), None);
        assert_eq!(LookFrame::new(forward, Vec3::ZERO), None);
        assert_eq!(LookFrame::new(forward, forward * 2.0), None);
        assert_eq!(LookFrame::new(Vec3::new(f32::NAN, 0.0, 1.0), up), None);
        assert_eq!(LookFrame::new(forward, Vec3::new(0.0, f32::NAN, 0.0)), None);
    }

    #[test]
    fn a_skewed_up_is_made_perpendicular_to_forward() {
        let forward = Vec3::new(0.3, -0.2, 1.0);
        let up = Vec3::new(0.1, 1.0, 0.4);
        let frame = LookFrame::new(forward, up).unwrap();

        assert!(frame.forward().distance(forward.normalize()) < 1e-6);
        assert!((frame.up().length() - 1.0).abs() < 1e-6);
        assert!(frame.up().dot(frame.forward()).abs() < 1e-6);
        assert!(frame.up().dot(forward.cross(up)).abs() < 1e-6);
        assert!(frame.up().dot(up) > 0.0);
    }

    #[test]
    fn a_zero_direction_gives_no_turn() {
        assert_eq!(yaw_pitch_to(&skewed(), Vec3::ZERO), YawPitch::default());
    }
}
