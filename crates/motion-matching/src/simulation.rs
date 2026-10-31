use concerto_ecs::{reflect::ReflectComponent, Component};
use concerto_foundation::math::{spring_damper::halflife_to_damping, Spring};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

/// The world velocity and facing a character should reach, set by other systems.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct DesiredMotion {
    pub velocity: Vec3,
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
    pub position: Vec3,
    pub velocity: Vec3,
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

/// The spring simulation of a character, stepped on the fixed timestep and sampled ahead for motion matching; its tuning is public and its state is not.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
#[require(DesiredMotion)]
pub struct CharacterSimulation {
    pub velocity_halflife: f32,
    pub rotation_halflife: f32,
    pub sample_dt: f32,
    pub sample_count: usize,
    #[reflect(skip)]
    pub(crate) velocity: Spring<Vec3>,
    #[reflect(skip)]
    pub(crate) facing: Spring<Quat>,
}

impl Default for CharacterSimulation {
    fn default() -> Self {
        let halflife = 0.27;
        Self {
            velocity_halflife: halflife,
            rotation_halflife: halflife,
            sample_dt: 1.0 / 3.0,
            sample_count: 3,
            velocity: Spring::critically_damped(Vec3::ZERO, frequency(halflife)),
            facing: Spring::critically_damped(Quat::IDENTITY, frequency(halflife)),
        }
    }
}

impl CharacterSimulation {
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
    use super::*;
    use crate::definition::TrajectorySettings;

    const EPS: f32 = 1.0e-3;

    fn moving_simulation() -> CharacterSimulation {
        let mut simulation = CharacterSimulation::default();
        simulation.velocity.value = Vec3::new(0.5, 0.0, 1.0);
        simulation.velocity.velocity = Vec3::new(-0.2, 0.0, 0.3);
        simulation.facing.value = Quat::from_rotation_y(0.4);
        simulation.facing.velocity = Vec3::new(0.0, 0.7, 0.0);
        simulation
    }

    fn moving_desired() -> DesiredMotion {
        DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.5),
        }
    }

    #[test]
    fn prediction_is_where_the_simulation_goes_while_the_desired_motion_holds() {
        const STEPS: usize = 300;
        let mut simulation = moving_simulation();
        let desired = moving_desired();
        let start = Vec3::new(1.0, 0.5, -2.0);
        let samples = simulation.predict(start, &desired);
        assert_eq!(samples.len(), simulation.sample_count);

        let dt = simulation.sample_dt / STEPS as f32;
        let mut position = start;
        let mut velocity = simulation.velocity.value;
        for sample in &samples {
            for _ in 0..STEPS {
                let next = simulation.advance(&desired, dt);
                position += (velocity + next) * 0.5 * dt;
                velocity = next;
            }
            assert!(
                (sample.position - position).length() < EPS,
                "{:?} vs {position:?}",
                sample.position
            );
            assert!((sample.velocity - velocity).length() < EPS);
            assert!(sample.rotation.abs_diff_eq(simulation.facing.value, EPS));
        }
    }

    #[test]
    fn zero_desired_velocity_from_rest_stays_at_the_start() {
        let start = Vec3::new(4.0, 1.0, -3.0);
        let samples = CharacterSimulation::default().predict(start, &DesiredMotion::default());
        assert_eq!(samples.len(), 3);
        for sample in samples {
            assert!((sample.position - start).length() < EPS);
            assert!(sample.velocity.length() < EPS);
            assert!(sample.rotation.abs_diff_eq(Quat::IDENTITY, EPS));
        }
    }

    #[test]
    fn prediction_follows_sample_count_and_ignores_desired_vertical_velocity() {
        let simulation = CharacterSimulation {
            sample_count: 5,
            ..CharacterSimulation::default()
        };
        let desired = DesiredMotion {
            velocity: Vec3::new(0.0, 9.0, 0.0),
            rotation: Quat::IDENTITY,
        };
        let samples = simulation.predict(Vec3::new(0.0, 2.0, 0.0), &desired);
        assert_eq!(samples.len(), 5);
        for sample in samples {
            assert!((sample.position.y - 2.0).abs() < EPS);
            assert!(sample.velocity.y.abs() < EPS);
        }
    }

    #[test]
    fn the_default_halflife_covers_forty_percent_of_the_distance_in_one_halflife() {
        let mut simulation = CharacterSimulation::default();
        let desired = DesiredMotion {
            velocity: Vec3::X,
            rotation: Quat::IDENTITY,
        };

        let velocity = simulation.advance(&desired, 0.27);

        assert!((velocity.x - 0.4034).abs() < EPS, "was {velocity:?}");
    }

    #[test]
    fn changed_halflives_take_effect_in_steps_and_in_predictions() {
        let simulation = CharacterSimulation {
            velocity_halflife: 0.54,
            rotation_halflife: 0.54,
            sample_dt: 0.54,
            sample_count: 1,
            ..CharacterSimulation::default()
        };
        let desired = DesiredMotion {
            velocity: Vec3::X,
            rotation: Quat::from_rotation_y(1.0),
        };
        let rotated = Quat::from_rotation_y(0.4034);

        let sample = simulation.predict(Vec3::ZERO, &desired)[0];
        assert!((sample.velocity.x - 0.4034).abs() < EPS);
        assert!(sample.rotation.abs_diff_eq(rotated, EPS));

        let mut stepped = simulation;
        let velocity = stepped.advance(&desired, 0.54);
        assert!((velocity.x - 0.4034).abs() < EPS);
        assert!(stepped.facing.value.abs_diff_eq(rotated, EPS));
    }

    #[test]
    fn the_default_sampling_is_the_default_of_a_motion_database() {
        let simulation = CharacterSimulation::default();
        let database = TrajectorySettings::default();

        assert_eq!(simulation.sample_dt, database.sample_dt);
        assert_eq!(simulation.sample_count, database.sample_count);
    }
}
