use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_mesh::{JointPose, Skeleton};
use concerto_reflect::{to_value, Reflect, TypeRegistry};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

use crate::{
    bake::bake,
    channel::{FeatureChannel, FrameContext, ReflectFeatureChannel},
    database::MotionDatabase,
    definition::{
        MotionDatabaseDefinition, MotionEntry, SimulationBoneSettings, StoredChannel,
        TrajectorySettings,
    },
};

pub(crate) const TURN_RATE: f32 = 1.5;

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
pub(crate) struct YawRate {
    weight: f32,
}

impl FeatureChannel for YawRate {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.body.angular_velocity.y;
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
pub(crate) struct Speed {
    weight: f32,
}

impl FeatureChannel for Speed {
    fn dims(&self) -> usize {
        1
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = frame.body.velocity.length();
    }

    fn weight(&self) -> f32 {
        self.weight
    }
}

pub(crate) fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; 3],
        vec![None, Some(0), Some(1)],
        vec!["Root".into(), "Hips".into(), "Spine2".into()],
        vec![
            JointPose::IDENTITY,
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::PI),
                ..JointPose::IDENTITY
            },
            JointPose {
                translation: Vec3::new(0.0, 0.3, 0.0),
                ..JointPose::IDENTITY
            },
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)],
    )
    .unwrap()
}

pub(crate) fn walk_forward() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)]),
        ),
    );
    clip
}

pub(crate) fn turn_left() -> AnimationClip {
    let mut clip = AnimationClip::default();
    let keys = 5;
    let times = (0..keys).map(|key| key as f32 * 0.5).collect();
    let rotations = (0..keys)
        .map(|key| Quat::from_rotation_y(std::f32::consts::PI + TURN_RATE * key as f32 * 0.5))
        .collect();
    clip.add_channel(
        Uuid::from_u128(2),
        AnimationChannel::new(times, AnimationChanelOutput::Rotation(rotations)),
    );
    clip
}

pub(crate) fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<YawRate>();
    registry.register::<Speed>();
    registry
}

pub(crate) fn two_clip_database() -> MotionDatabase {
    let definition = MotionDatabaseDefinition {
        entries: vec![
            MotionEntry {
                clip: Default::default(),
                range: None,
            },
            MotionEntry {
                clip: Default::default(),
                range: None,
            },
        ],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        trajectory: TrajectorySettings {
            sample_dt: 0.25,
            sample_count: 3,
        },
        channels: vec![
            StoredChannel {
                type_name: "YawRate".into(),
                value: to_value(&YawRate { weight: 1.0 }).unwrap(),
            },
            StoredChannel {
                type_name: "Speed".into(),
                value: to_value(&Speed { weight: 1.0 }).unwrap(),
            },
        ],
        ..Default::default()
    };
    bake(
        &definition,
        &skeleton(),
        &[&walk_forward(), &turn_left()],
        &registry(),
    )
    .unwrap()
}
