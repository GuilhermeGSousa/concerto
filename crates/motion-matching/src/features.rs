use concerto_ecs::{reflect::ReflectComponent, Component};
use concerto_mesh::{JointName, Skeleton};
use concerto_reflect::{to_value, Reflect};
use glam::{Quat, Vec3};

use crate::{
    channel::{ChannelSource, FeatureChannel, FrameContext, ReflectFeatureChannel},
    definition::{StoredChannel, TrajectorySettings},
};

fn local_point(frame: &FrameContext, point: Vec3) -> Vec3 {
    frame.simulation.rotation.inverse() * (point - frame.simulation.translation)
}

fn local_vector(frame: &FrameContext, vector: Vec3) -> Vec3 {
    let rotation: Quat = frame.simulation.rotation;
    rotation.inverse() * vector
}

fn check_bones(bones: &[JointName], skeleton: &Skeleton) -> Result<(), String> {
    match bones
        .iter()
        .find(|bone| skeleton.bone_index(bone).is_none())
    {
        Some(bone) => Err(format!("the skeleton has no bone named {}", bone.0)),
        None => Ok(()),
    }
}

fn check_samples(samples: &[usize], trajectory: &TrajectorySettings) -> Result<(), String> {
    match samples
        .iter()
        .find(|sample| **sample >= trajectory.sample_count)
    {
        Some(sample) => Err(format!(
            "trajectory sample {sample} is past the {} samples the trajectory has",
            trajectory.sample_count
        )),
        None => Ok(()),
    }
}

/// The model-space positions of bones, in the simulation bone's frame; 3 values per bone.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, FeatureChannel)]
pub struct BonePositions {
    pub bones: Vec<JointName>,
    pub weight: f32,
}

impl Default for BonePositions {
    fn default() -> Self {
        Self {
            bones: Vec::new(),
            weight: 0.75,
        }
    }
}

impl FeatureChannel for BonePositions {
    fn dims(&self) -> usize {
        3 * self.bones.len()
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        for (bone, out) in self.bones.iter().zip(out.chunks_mut(3)) {
            let value = frame
                .skeleton
                .bone_index(bone)
                .and_then(|index| frame.model_pose.get(index))
                .map_or(Vec3::ZERO, |pose| local_point(frame, pose.translation));
            out.copy_from_slice(&value.to_array());
        }
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn validate(
        &self,
        skeleton: &Skeleton,
        _trajectory: &TrajectorySettings,
    ) -> Result<(), String> {
        check_bones(&self.bones, skeleton)
    }
}

/// The model-space velocities of bones, in the simulation bone's frame; 3 values per bone.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, FeatureChannel)]
pub struct BoneVelocities {
    pub bones: Vec<JointName>,
    pub weight: f32,
}

impl Default for BoneVelocities {
    fn default() -> Self {
        Self {
            bones: Vec::new(),
            weight: 1.0,
        }
    }
}

impl FeatureChannel for BoneVelocities {
    fn dims(&self) -> usize {
        3 * self.bones.len()
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        for (bone, out) in self.bones.iter().zip(out.chunks_mut(3)) {
            let value = frame
                .skeleton
                .bone_index(bone)
                .and_then(|index| frame.model_velocities.get(index))
                .map_or(Vec3::ZERO, |velocity| local_vector(frame, *velocity));
            out.copy_from_slice(&value.to_array());
        }
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn validate(
        &self,
        skeleton: &Skeleton,
        _trajectory: &TrajectorySettings,
    ) -> Result<(), String> {
        check_bones(&self.bones, skeleton)
    }
}

/// The ground-plane positions of trajectory samples, in the simulation bone's frame; x and z per sample.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, FeatureChannel)]
pub struct TrajectoryPositions {
    pub samples: Vec<usize>,
    pub weight: f32,
}

impl Default for TrajectoryPositions {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            weight: 1.0,
        }
    }
}

impl FeatureChannel for TrajectoryPositions {
    fn dims(&self) -> usize {
        2 * self.samples.len()
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        for (sample, out) in self.samples.iter().zip(out.chunks_mut(2)) {
            let value = frame
                .trajectory
                .get(*sample)
                .map_or(Vec3::ZERO, |sample| local_point(frame, sample.position));
            out[0] = value.x;
            out[1] = value.z;
        }
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn source(&self) -> ChannelSource {
        ChannelSource::Trajectory
    }

    fn validate(
        &self,
        _skeleton: &Skeleton,
        trajectory: &TrajectorySettings,
    ) -> Result<(), String> {
        check_samples(&self.samples, trajectory)
    }
}

/// The ground-plane facing of trajectory samples, in the simulation bone's frame; x and z per sample.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, FeatureChannel)]
pub struct TrajectoryDirections {
    pub samples: Vec<usize>,
    pub weight: f32,
}

impl Default for TrajectoryDirections {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            weight: 1.5,
        }
    }
}

impl FeatureChannel for TrajectoryDirections {
    fn dims(&self) -> usize {
        2 * self.samples.len()
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        for (sample, out) in self.samples.iter().zip(out.chunks_mut(2)) {
            let value = frame.trajectory.get(*sample).map_or(Vec3::ZERO, |sample| {
                local_vector(frame, sample.rotation * Vec3::Z)
            });
            out[0] = value.x;
            out[1] = value.z;
        }
    }

    fn weight(&self) -> f32 {
        self.weight
    }

    fn source(&self) -> ChannelSource {
        ChannelSource::Trajectory
    }

    fn validate(
        &self,
        _skeleton: &Skeleton,
        trajectory: &TrajectorySettings,
    ) -> Result<(), String> {
        check_samples(&self.samples, trajectory)
    }
}

fn stored<T: FeatureChannel>(type_name: &str, channel: &T) -> StoredChannel {
    StoredChannel {
        type_name: type_name.into(),
        value: to_value(channel).expect("a feature channel encodes"),
    }
}

fn names(bones: &[&str]) -> Vec<JointName> {
    bones.iter().map(|bone| (*bone).into()).collect()
}

/// The reference 27-value feature set: foot positions and velocities, hip velocity, and three trajectory positions and directions.
pub fn reference_channels() -> Vec<StoredChannel> {
    vec![
        stored(
            "BonePositions",
            &BonePositions {
                bones: names(&["LeftFoot", "RightFoot"]),
                weight: 0.75,
            },
        ),
        stored(
            "BoneVelocities",
            &BoneVelocities {
                bones: names(&["LeftFoot", "RightFoot"]),
                weight: 1.0,
            },
        ),
        stored(
            "BoneVelocities",
            &BoneVelocities {
                bones: names(&["Hips"]),
                weight: 1.0,
            },
        ),
        stored(
            "TrajectoryPositions",
            &TrajectoryPositions {
                samples: vec![0, 1, 2],
                weight: 1.0,
            },
        ),
        stored(
            "TrajectoryDirections",
            &TrajectoryDirections {
                samples: vec![0, 1, 2],
                weight: 1.5,
            },
        ),
    ]
}
