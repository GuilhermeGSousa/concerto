use concerto_ecs::Component;
use concerto_foundation::math::spring_damper::{
    simple_spring_damper_exact_quat, spring_character_predict, CharacterState,
};
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

/// The spring simulation of a character, stepped on the fixed timestep and sampled ahead for motion matching.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
#[require(DesiredMotion)]
pub struct CharacterSimulation {
    pub state: CharacterState,
    pub rotation: Quat,
    pub angular_velocity: Vec3,
    pub velocity_halflife: f32,
    pub rotation_halflife: f32,
    pub sample_dt: f32,
    pub sample_count: usize,
    pub trajectory: Vec<TrajectorySample>,
}

impl Default for CharacterSimulation {
    fn default() -> Self {
        Self {
            state: CharacterState::default(),
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            velocity_halflife: 0.27,
            rotation_halflife: 0.27,
            sample_dt: 1.0 / 3.0,
            sample_count: 3,
            trajectory: Vec::new(),
        }
    }
}

/// The samples `sample_dt * (i + 1)` ahead of `simulation` while `desired` is held constant.
pub fn predict_trajectory(
    simulation: &CharacterSimulation,
    desired: &DesiredMotion,
) -> Vec<TrajectorySample> {
    let desired_velocity = Vec3::new(desired.velocity.x, 0.0, desired.velocity.z);
    spring_character_predict(
        simulation.state,
        desired_velocity,
        simulation.velocity_halflife,
        simulation.sample_dt,
        simulation.sample_count,
    )
    .into_iter()
    .enumerate()
    .map(|(i, state)| {
        let (rotation, _) = simple_spring_damper_exact_quat(
            simulation.rotation,
            simulation.angular_velocity,
            desired.rotation,
            simulation.rotation_halflife,
            simulation.sample_dt * (i + 1) as f32,
        );
        TrajectorySample {
            position: state.position,
            velocity: state.velocity,
            rotation,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use concerto_foundation::math::spring_damper::spring_character_update;
    use concerto_reflect::{with_property, PropertyPath};
    use std::any::Any;

    use super::*;

    const EPS: f32 = 1.0e-4;

    fn moving_simulation() -> CharacterSimulation {
        CharacterSimulation {
            state: CharacterState {
                position: Vec3::new(1.0, 0.0, -2.0),
                velocity: Vec3::new(0.5, 0.0, 1.0),
                acceleration: Vec3::new(-0.2, 0.0, 0.3),
            },
            rotation: Quat::from_rotation_y(0.4),
            angular_velocity: Vec3::new(0.0, 0.7, 0.0),
            ..CharacterSimulation::default()
        }
    }

    fn moving_desired() -> DesiredMotion {
        DesiredMotion {
            velocity: Vec3::new(3.0, 0.0, 4.0),
            rotation: Quat::from_rotation_y(1.5),
        }
    }

    #[test]
    fn prediction_sample_i_equals_i_plus_one_updates() {
        let simulation = moving_simulation();
        let desired = moving_desired();
        let samples = predict_trajectory(&simulation, &desired);
        assert_eq!(samples.len(), simulation.sample_count);

        let mut state = simulation.state;
        let mut rotation = simulation.rotation;
        let mut angular_velocity = simulation.angular_velocity;
        for sample in &samples {
            state = spring_character_update(
                state,
                desired.velocity,
                simulation.velocity_halflife,
                simulation.sample_dt,
            );
            (rotation, angular_velocity) = simple_spring_damper_exact_quat(
                rotation,
                angular_velocity,
                desired.rotation,
                simulation.rotation_halflife,
                simulation.sample_dt,
            );
            assert!((sample.position - state.position).length() < EPS);
            assert!((sample.velocity - state.velocity).length() < EPS);
            assert!(sample.rotation.angle_between(rotation) < EPS);
        }
    }

    #[test]
    fn zero_desired_velocity_from_rest_stays_put() {
        let simulation = CharacterSimulation::default();
        let samples = predict_trajectory(&simulation, &DesiredMotion::default());
        assert_eq!(samples.len(), 3);
        for sample in samples {
            assert!(sample.position.length() < EPS);
            assert!(sample.velocity.length() < EPS);
            assert!(sample.rotation.angle_between(Quat::IDENTITY) < EPS);
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
        let samples = predict_trajectory(&simulation, &desired);
        assert_eq!(samples.len(), 5);
        for sample in samples {
            assert!(sample.position.y.abs() < EPS);
            assert!(sample.velocity.y.abs() < EPS);
        }
    }

    #[test]
    fn simulation_defaults_match_the_spec() {
        let simulation = CharacterSimulation::default();
        assert_eq!(simulation.velocity_halflife, 0.27);
        assert_eq!(simulation.rotation_halflife, 0.27);
        assert_eq!(simulation.sample_count, 3);
        assert!((simulation.sample_dt - 1.0 / 3.0).abs() < 1.0e-6);
        assert!(simulation.trajectory.is_empty());
        assert_eq!(simulation.rotation, Quat::IDENTITY);
    }

    #[test]
    fn trajectory_sample_reflects_its_fields() {
        let sample = TrajectorySample {
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, 6.0),
            rotation: Quat::from_rotation_y(0.5),
        };
        let mut position = None;
        let mut velocity = None;
        let mut rotation = None;
        with_property(&sample, &PropertyPath::new(["position"]), &mut |value| {
            position = (value as &dyn Any).downcast_ref::<Vec3>().copied();
        })
        .unwrap();
        with_property(&sample, &PropertyPath::new(["velocity"]), &mut |value| {
            velocity = (value as &dyn Any).downcast_ref::<Vec3>().copied();
        })
        .unwrap();
        with_property(&sample, &PropertyPath::new(["rotation"]), &mut |value| {
            rotation = (value as &dyn Any).downcast_ref::<Quat>().copied();
        })
        .unwrap();
        assert_eq!(position, Some(sample.position));
        assert_eq!(velocity, Some(sample.velocity));
        assert_eq!(rotation, Some(sample.rotation));
    }
}
