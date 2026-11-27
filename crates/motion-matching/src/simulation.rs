use concerto_ecs::{reflect::ReflectComponent, Component};
use concerto_foundation::math::{spring_damper::halflife_to_damping, Spring};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

/// The world velocity and facing a character should reach, set by other systems.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct DesiredMotion {
    /// The world-space velocity to reach, in metres per second.
    pub velocity: Vec3,
    /// The world-space facing to reach.
    pub rotation: Quat,
}

impl Default for DesiredMotion {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

/// A predicted position, velocity and facing at a fixed time ahead.
#[derive(Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Default)]
pub struct TrajectorySample {
    /// The predicted position, in world space.
    pub position: Vec3,
    /// The predicted velocity, in metres per second.
    pub velocity: Vec3,
    /// The predicted facing.
    pub rotation: Quat,
}

impl Default for TrajectorySample {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

fn frequency(halflife: f32) -> f32 {
    halflife_to_damping(halflife) / 2.0
}

/// The spring simulation of a character, stepped on the fixed timestep and sampled ahead for motion matching.
///
/// Its tuning is public and its state is private, read through
/// [`interpolated_facing`](Self::interpolated_facing).
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
#[require(DesiredMotion)]
pub struct CharacterSimulation {
    /// The halflife, in seconds, the velocity springs toward the desired one with.
    pub velocity_halflife: f32,
    /// The halflife, in seconds, the facing springs toward the desired one with.
    pub rotation_halflife: f32,
    /// The time between trajectory samples, in seconds.
    pub sample_dt: f32,
    /// The number of trajectory samples predicted ahead.
    pub sample_count: usize,
    /// The speed, in metres per second, the capsule's pull toward the character is capped at
    /// unless the spring already moves faster.
    pub max_pull_speed: f32,
    #[reflect(skip)]
    pub(crate) velocity: Spring<Vec3>,
    #[reflect(skip)]
    pub(crate) facing: Spring<Quat>,
    #[reflect(skip)]
    pub(crate) pull: Vec3,
    #[reflect(skip)]
    pub(crate) previous_facing: Quat,
}

impl Default for CharacterSimulation {
    fn default() -> Self {
        let halflife = 0.27;
        Self {
            velocity_halflife: halflife,
            rotation_halflife: halflife,
            sample_dt: 1.0 / 3.0,
            sample_count: 3,
            max_pull_speed: 8.0,
            velocity: Spring::critically_damped(Vec3::ZERO, frequency(halflife)),
            facing: Spring::critically_damped(Quat::IDENTITY, frequency(halflife)),
            pull: Vec3::ZERO,
            previous_facing: Quat::IDENTITY,
        }
    }
}

impl CharacterSimulation {
    /// Restarts the springs moving at `velocity` and facing `facing`, with nothing left to pull.
    pub fn restart(&mut self, velocity: Vec3, facing: Quat) {
        self.velocity.value = velocity;
        self.velocity.velocity = Default::default();
        self.facing.value = facing;
        self.facing.velocity = Default::default();
        self.previous_facing = facing;
        self.pull = Vec3::ZERO;
    }

    fn springs(&self) -> (Spring<Vec3>, Spring<Quat>) {
        (
            Spring {
                angular_frequency: frequency(self.velocity_halflife),
                ..self.velocity
            },
            Spring {
                angular_frequency: frequency(self.rotation_halflife),
                ..self.facing
            },
        )
    }

    /// The facing `alpha` of the way from the last fixed step to the current one.
    pub fn interpolated_facing(&self, alpha: f32) -> Quat {
        self.previous_facing.slerp(self.facing.value, alpha)
    }

    /// Advances the springs `dt` towards `desired` and returns the horizontal velocity the body should move at.
    pub(crate) fn advance(&mut self, desired: &DesiredMotion, dt: f32) -> Vec3 {
        let (mut velocity, mut facing) = self.springs();
        velocity.update(desired.velocity.with_y(0.0), dt);
        facing.update(desired.rotation, dt);
        self.velocity = velocity;
        self.facing = facing;
        velocity.value
    }

    /// The samples `sample_dt * (i + 1)` ahead of a body at `position` while `desired` is held constant.
    pub(crate) fn predict(&self, position: Vec3, desired: &DesiredMotion) -> Vec<TrajectorySample> {
        let (mut velocity, mut facing) = self.springs();
        let target = desired.velocity.with_y(0.0);
        let mut position = position;
        (0..self.sample_count)
            .map(|_| {
                position += velocity.integral(target, self.sample_dt);
                TrajectorySample {
                    position,
                    velocity: velocity.update(target, self.sample_dt),
                    rotation: facing.update(desired.rotation, self.sample_dt),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const EPS: f32 = 1.0e-3;

    fn horizontal(range: f32) -> impl Strategy<Value = Vec3> {
        (-range..range, -range..range).prop_map(|(x, z)| Vec3::new(x, 0.0, z))
    }

    proptest! {
        #[test]
        fn the_prediction_is_where_the_simulation_goes_while_the_desired_motion_holds(
            velocity in horizontal(2.0),
            acceleration in horizontal(1.0),
            facing in -3.0f32..3.0,
            spin in -1.0f32..1.0,
            desired_velocity in horizontal(5.0),
            climb in -9.0f32..9.0,
            turn in -2.5f32..2.5,
            start in prop::array::uniform3(-5.0f32..5.0),
            halflives in prop::array::uniform2(0.05f32..1.0),
            sample_dt in 0.1f32..0.5,
            sample_count in 1usize..5,
        ) {
            const STEPS: usize = 300;
            let mut simulation = CharacterSimulation {
                velocity_halflife: halflives[0],
                rotation_halflife: halflives[1],
                sample_dt,
                sample_count,
                ..CharacterSimulation::default()
            };
            simulation.velocity.value = velocity;
            simulation.velocity.velocity = acceleration;
            simulation.facing.value = Quat::from_rotation_y(facing);
            simulation.facing.velocity = Vec3::new(0.0, spin, 0.0);
            let desired = DesiredMotion {
                velocity: desired_velocity.with_y(climb),
                rotation: Quat::from_rotation_y(facing + turn),
            };
            let start = Vec3::from_array(start);

            let samples = simulation.predict(start, &desired);

            prop_assert_eq!(samples.len(), sample_count);
            let dt = sample_dt / STEPS as f32;
            let mut position = start;
            let mut velocity = simulation.velocity.value;
            for sample in &samples {
                for _ in 0..STEPS {
                    let next = simulation.advance(&desired, dt);
                    position += (velocity + next) * 0.5 * dt;
                    velocity = next;
                }
                prop_assert!(
                    (sample.position - position).length() < EPS,
                    "{:?} vs {position:?}",
                    sample.position
                );
                prop_assert!((sample.position.y - start.y).abs() < EPS);
                prop_assert!((sample.velocity - velocity).length() < EPS);
                prop_assert!(sample.velocity.y.abs() < EPS);
                let facing = if sample.rotation.dot(simulation.facing.value) < 0.0 {
                    -simulation.facing.value
                } else {
                    simulation.facing.value
                };
                prop_assert!(
                    sample.rotation.abs_diff_eq(facing, EPS),
                    "{:?} vs {facing:?}",
                    sample.rotation
                );
            }
        }
    }

    #[test]
    fn a_spring_covers_forty_percent_of_the_way_in_one_halflife() {
        for halflife in [CharacterSimulation::default().velocity_halflife, 0.54] {
            let mut simulation = CharacterSimulation {
                velocity_halflife: halflife,
                rotation_halflife: halflife,
                ..CharacterSimulation::default()
            };
            let desired = DesiredMotion {
                velocity: Vec3::X,
                rotation: Quat::from_rotation_y(1.0),
            };

            let velocity = simulation.advance(&desired, halflife);

            assert!(
                (velocity.x - 0.4034).abs() < EPS,
                "{halflife}: {velocity:?}"
            );
            let rotated = Quat::from_rotation_y(0.4034);
            assert!(
                simulation.facing.value.abs_diff_eq(rotated, EPS),
                "{halflife}"
            );
        }
    }
}
