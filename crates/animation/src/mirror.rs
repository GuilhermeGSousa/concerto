use std::{collections::HashMap, fmt};

use concerto_mesh::skeleton::{JointPose, Skeleton};
use glam::{Mat3, Quat, Vec3};

/// How to mirror a pose of one skeleton across a plane through its model-space origin.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseMirror {
    twins: Box<[u16]>,
    before: Box<[Mat3]>,
    after: Box<[Mat3]>,
}

impl PoseMirror {
    /// The mirror of `skeleton` across the plane whose normal is `axis`, in model space.
    pub fn new(skeleton: &Skeleton, axis: Vec3) -> Result<Self, MirrorError> {
        let n = axis.try_normalize().ok_or(MirrorError::InvalidAxis(axis))?;
        let reflection = Mat3::IDENTITY - 2.0 * Mat3::from_cols(n * n.x, n * n.y, n * n.z);

        let twins = twins(skeleton)?;
        for (bone, parent) in skeleton.parents.iter().enumerate() {
            let twin = usize::from(twins[bone]);
            if parent.map(|parent| twins[usize::from(parent)]) != skeleton.parents[twin] {
                return Err(MirrorError::TwinParents {
                    joint: skeleton.names[bone].clone(),
                });
            }
        }

        let mut rest: Vec<Quat> = Vec::with_capacity(skeleton.bone_count());
        for (pose, parent) in skeleton.rest_pose.iter().zip(skeleton.parents.iter()) {
            let rotation = match parent {
                Some(parent) => rest[usize::from(*parent)] * pose.rotation,
                None => pose.rotation,
            };
            rest.push(rotation.normalize());
        }
        let rest: Vec<Mat3> = rest.into_iter().map(Mat3::from_quat).collect();

        let after: Box<[Mat3]> = (0..skeleton.bone_count())
            .map(|bone| rest[usize::from(twins[bone])].transpose() * reflection * rest[bone])
            .collect();
        let before: Box<[Mat3]> = skeleton
            .parents
            .iter()
            .map(|parent| match parent {
                Some(parent) => after[usize::from(*parent)].transpose(),
                None => reflection,
            })
            .collect();

        let mut worst: Option<(usize, f32)> = None;
        for (bone, parent) in skeleton.parents.iter().enumerate() {
            if parent.is_none() {
                continue;
            }
            let own = skeleton.rest_pose[bone].translation;
            let twin = skeleton.rest_pose[usize::from(twins[bone])].translation;
            let distance = (before[bone] * twin).distance(own);
            if distance > 1e-3 + 1e-2 * own.length()
                && worst.is_none_or(|(_, largest)| distance > largest)
            {
                worst = Some((bone, distance));
            }
        }
        if let Some((bone, distance)) = worst {
            return Err(MirrorError::Asymmetric {
                joint: skeleton.names[bone].clone(),
                distance,
            });
        }

        Ok(PoseMirror {
            twins,
            before,
            after,
        })
    }

    /// The bone that `bone` takes its pose from: its twin, or itself.
    pub fn twin(&self, bone: usize) -> Option<usize> {
        self.twins.get(bone).map(|twin| usize::from(*twin))
    }

    /// Writes `source`, one local `JointPose` per bone in skeleton order, mirrored into `out`.
    pub fn mirror_into(
        &self,
        source: &[JointPose],
        out: &mut [JointPose],
    ) -> Result<(), MirrorError> {
        let expected = self.twins.len();
        for actual in [source.len(), out.len()] {
            if actual != expected {
                return Err(MirrorError::BoneCountMismatch { expected, actual });
            }
        }
        for (bone, target) in out.iter_mut().enumerate() {
            let pose = &source[usize::from(self.twins[bone])];
            let before = self.before[bone];
            let rotation = before * Mat3::from_quat(pose.rotation) * self.after[bone];
            *target = JointPose {
                translation: before * pose.translation,
                rotation: Quat::from_mat3(&rotation).normalize(),
                scale: pose.scale,
            };
        }
        Ok(())
    }
}

fn twins(skeleton: &Skeleton) -> Result<Box<[u16]>, MirrorError> {
    let indices: HashMap<&str, usize> = skeleton
        .names
        .iter()
        .enumerate()
        .rev()
        .map(|(bone, name)| (name.as_str(), bone))
        .collect();
    skeleton
        .names
        .iter()
        .enumerate()
        .map(|(bone, name)| {
            let twin_name = match (name.contains("Left"), name.contains("Right")) {
                (true, false) => name.replace("Left", "Right"),
                (false, true) => name.replace("Right", "Left"),
                _ => return Ok(bone as u16),
            };
            indices
                .get(twin_name.as_str())
                .map(|twin| *twin as u16)
                .ok_or(MirrorError::MissingTwin {
                    joint: name.clone(),
                    twin: twin_name,
                })
        })
        .collect()
}

/// Why a skeleton cannot be mirrored, or a pose does not fit its mirror.
#[derive(Clone, Debug, PartialEq)]
pub enum MirrorError {
    InvalidAxis(Vec3),
    MissingTwin { joint: String, twin: String },
    TwinParents { joint: String },
    Asymmetric { joint: String, distance: f32 },
    BoneCountMismatch { expected: usize, actual: usize },
}

impl fmt::Display for MirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirrorError::InvalidAxis(axis) => {
                write!(f, "mirror axis {axis} is zero or not finite")
            }
            MirrorError::MissingTwin { joint, twin } => {
                write!(f, "joint `{joint}` has no twin `{twin}` in the skeleton")
            }
            MirrorError::TwinParents { joint } => write!(
                f,
                "joint `{joint}` and its twin have parents that are not twins"
            ),
            MirrorError::Asymmetric { joint, distance } => write!(
                f,
                "rest pose is not symmetric across the mirror plane: joint `{joint}` is {distance} from its twin's reflection"
            ),
            MirrorError::BoneCountMismatch { expected, actual } => {
                write!(f, "pose has {actual} joints, skeleton has {expected} bones")
            }
        }
    }
}

impl std::error::Error for MirrorError {}
