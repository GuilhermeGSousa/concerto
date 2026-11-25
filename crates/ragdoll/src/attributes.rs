use concerto_animation::attribute::PoseAttribute;
use glam::{Quat, Vec3};

/// How much of a bone is drawn from physics, from 0 (the animated pose) to 1 (the simulated one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsAmount(pub f32);

impl PoseAttribute for PhysicsAmount {
    fn neutral() -> Self {
        Self(0.0)
    }

    fn blend(&mut self, self_weight: f32, other: &Self, other_weight: f32) {
        let total = self_weight + other_weight;
        if total > 0.0 {
            self.0 = (self.0 * self_weight + other.0 * other_weight) / total;
        }
    }
}

/// What behaviours ask of the motor of a bone's part this physics step, in its joint's frame; `cover` is how much of the bone they hold, and follow holds the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotorCommand {
    pub cover: f32,
    /// The rotation the part should have relative to its parent part.
    pub target: Quat,
    pub stiffness: f32,
    pub damping: f32,
    pub torque_limit: f32,
    /// A world-space force on the part at its centre of mass, in N.
    pub force: Vec3,
    /// A world-space torque on the part, in N·m.
    pub torque: Vec3,
}

impl PoseAttribute for MotorCommand {
    fn neutral() -> Self {
        Self {
            cover: 0.0,
            target: Quat::IDENTITY,
            stiffness: 0.0,
            damping: 0.0,
            torque_limit: 0.0,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
        }
    }

    fn blend(&mut self, self_weight: f32, other: &Self, other_weight: f32) {
        let mine = self.cover * self_weight;
        let theirs = other.cover * other_weight;
        let held = mine + theirs;
        if held <= 0.0 {
            *self = Self::neutral();
            return;
        }
        let share = theirs / held;
        let mix = |a: f32, b: f32| a + (b - a) * share;
        *self = Self {
            cover: held / (self_weight + other_weight),
            target: self.target.slerp(other.target, share).normalize(),
            stiffness: mix(self.stiffness, other.stiffness),
            damping: mix(self.damping, other.damping),
            torque_limit: mix(self.torque_limit, other.torque_limit),
            force: self.force.lerp(other.force, share),
            torque: self.torque.lerp(other.torque, share),
        };
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::attribute::PoseAttribute;
    use glam::{Quat, Vec3};

    use super::{MotorCommand, PhysicsAmount};

    struct CommandRow {
        case: &'static str,
        a: MotorCommand,
        b: MotorCommand,
        weight: f32,
        expected: MotorCommand,
    }

    struct AmountRow {
        a: f32,
        b: f32,
        weight: f32,
        expected: f32,
    }

    fn held(cover: f32, stiffness: f32) -> MotorCommand {
        MotorCommand {
            cover,
            stiffness,
            damping: stiffness * 0.1,
            torque_limit: stiffness * 0.5,
            ..MotorCommand::neutral()
        }
    }

    fn finite(command: &MotorCommand) -> bool {
        [
            command.cover,
            command.stiffness,
            command.damping,
            command.torque_limit,
        ]
        .iter()
        .all(|value| value.is_finite())
            && command.target.is_finite()
            && command.force.is_finite()
            && command.torque.is_finite()
    }

    fn close(a: &MotorCommand, b: &MotorCommand) -> bool {
        (a.cover - b.cover).abs() < 1e-4
            && (a.stiffness - b.stiffness).abs() < 1e-3
            && (a.damping - b.damping).abs() < 1e-3
            && (a.torque_limit - b.torque_limit).abs() < 1e-3
            && a.target.angle_between(b.target) < 1e-3
            && a.force.distance(b.force) < 1e-3
            && a.torque.distance(b.torque) < 1e-3
    }

    #[test]
    fn motor_commands_blend_by_cover() {
        let turned = Quat::from_rotation_y(1.0);
        let commands = [
            CommandRow {
                case: "a full cover blended half with no cover keeps its strength and covers half",
                a: MotorCommand {
                    target: turned,
                    ..held(1.0, 400.0)
                },
                b: MotorCommand::neutral(),
                weight: 0.5,
                expected: MotorCommand {
                    target: turned,
                    ..held(0.5, 400.0)
                },
            },
            CommandRow {
                case: "equal covers average their strength and slerp their targets halfway",
                a: held(1.0, 100.0),
                b: MotorCommand {
                    target: turned,
                    ..held(1.0, 300.0)
                },
                weight: 0.5,
                expected: MotorCommand {
                    target: Quat::from_rotation_y(0.5),
                    ..held(1.0, 200.0)
                },
            },
            CommandRow {
                case: "two zero covers blend to neutral",
                a: MotorCommand {
                    target: turned,
                    force: Vec3::X,
                    ..held(0.0, 100.0)
                },
                b: held(0.0, 300.0),
                weight: 0.5,
                expected: MotorCommand::neutral(),
            },
            CommandRow {
                case: "forces and torques mix weighted by cover",
                a: MotorCommand {
                    force: Vec3::X * 30.0,
                    torque: Vec3::Y * 30.0,
                    ..held(1.0, 0.0)
                },
                b: MotorCommand {
                    force: Vec3::Z * 30.0,
                    ..held(0.5, 0.0)
                },
                weight: 0.5,
                expected: MotorCommand {
                    force: Vec3::new(20.0, 0.0, 10.0),
                    torque: Vec3::Y * 20.0,
                    ..held(0.75, 0.0)
                },
            },
        ];
        for row in commands {
            let mut blended = row.a;
            blended.blend(1.0 - row.weight, &row.b, row.weight);
            assert!(finite(&blended), "{}: {blended:?}", row.case);
            assert!(
                close(&blended, &row.expected),
                "{}: {blended:?}, expected {:?}",
                row.case,
                row.expected
            );
        }

        let amounts = [
            AmountRow {
                a: 0.2,
                b: 1.0,
                weight: 0.25,
                expected: 0.4,
            },
            AmountRow {
                a: 1.0,
                b: 0.0,
                weight: 1.0,
                expected: 0.0,
            },
            AmountRow {
                a: 0.6,
                b: 0.2,
                weight: 0.0,
                expected: 0.6,
            },
        ];
        for row in amounts {
            let mut blended = PhysicsAmount(row.a);
            blended.blend(1.0 - row.weight, &PhysicsAmount(row.b), row.weight);
            assert!(
                (blended.0 - row.expected).abs() < 1e-5,
                "{} blended with {} at {}: {}, expected {}",
                row.a,
                row.b,
                row.weight,
                blended.0,
                row.expected
            );
        }
        let mut unweighted = PhysicsAmount(0.3);
        unweighted.blend(0.0, &PhysicsAmount(1.0), 0.0);
        assert!(unweighted.0.is_finite());
    }
}
