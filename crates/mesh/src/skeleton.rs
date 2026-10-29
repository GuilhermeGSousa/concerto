use concerto_ecs::{
    component::scene::SceneEntityRef, reflect::ReflectSceneComponent, Component, Entity,
};
use concerto_foundation::assets::{handle::AssetHandle, Asset, LoadableAsset};
use concerto_reflect::Reflect;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Translation, rotation and scale of one bone relative to its parent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointPose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl JointPose {
    pub const IDENTITY: JointPose = JointPose {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// The matrix that applies scale, then rotation, then translation.
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Why a skeleton could not be built or evaluated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkeletonError {
    LengthMismatch {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    ParentNotBefore {
        bone: usize,
        parent: usize,
    },
    PoseLength {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for SkeletonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkeletonError::LengthMismatch {
                field,
                expected,
                actual,
            } => write!(
                f,
                "skeleton field `{field}` has {actual} entries, expected {expected}"
            ),
            SkeletonError::ParentNotBefore { bone, parent } => write!(
                f,
                "bone {bone} has parent {parent}, which does not come before it"
            ),
            SkeletonError::PoseLength { expected, actual } => {
                write!(f, "pose has {actual} joints, skeleton has {expected} bones")
            }
        }
    }
}

impl std::error::Error for SkeletonError {}

/// Bone hierarchy, names, rest pose and bind data; every slice has one entry per bone.
#[derive(Asset, Serialize, Deserialize)]
pub struct Skeleton {
    pub inverse_bindposes: Box<[Mat4]>,
    pub parents: Box<[Option<u16>]>,
    pub names: Box<[String]>,
    pub rest_pose: Box<[JointPose]>,
    pub bone_ids: Box<[Uuid]>,
}

impl Skeleton {
    /// Builds a skeleton, checking that the slices agree in length and that each parent precedes its child.
    pub fn new(
        inverse_bindposes: Vec<Mat4>,
        parents: Vec<Option<u16>>,
        names: Vec<String>,
        rest_pose: Vec<JointPose>,
        bone_ids: Vec<Uuid>,
    ) -> Result<Skeleton, SkeletonError> {
        let expected = inverse_bindposes.len();
        for (field, actual) in [
            ("parents", parents.len()),
            ("names", names.len()),
            ("rest_pose", rest_pose.len()),
            ("bone_ids", bone_ids.len()),
        ] {
            if actual != expected {
                return Err(SkeletonError::LengthMismatch {
                    field,
                    expected,
                    actual,
                });
            }
        }
        for (bone, parent) in parents.iter().enumerate() {
            if let Some(parent) = parent {
                if usize::from(*parent) >= bone {
                    return Err(SkeletonError::ParentNotBefore {
                        bone,
                        parent: usize::from(*parent),
                    });
                }
            }
        }
        Ok(Skeleton {
            inverse_bindposes: inverse_bindposes.into_boxed_slice(),
            parents: parents.into_boxed_slice(),
            names: names.into_boxed_slice(),
            rest_pose: rest_pose.into_boxed_slice(),
            bone_ids: bone_ids.into_boxed_slice(),
        })
    }

    pub fn bone_count(&self) -> usize {
        self.inverse_bindposes.len()
    }

    /// The index of the first bone with this name.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|bone| bone == name)
    }

    /// The global transform of every bone for the given local poses.
    pub fn forward_kinematics(&self, local: &[JointPose]) -> Result<Vec<Mat4>, SkeletonError> {
        if local.len() != self.bone_count() {
            return Err(SkeletonError::PoseLength {
                expected: self.bone_count(),
                actual: local.len(),
            });
        }
        let mut global: Vec<Mat4> = Vec::with_capacity(local.len());
        for (pose, parent) in local.iter().zip(self.parents.iter()) {
            let local_matrix = pose.to_mat4();
            let matrix = match parent {
                Some(parent) => global[usize::from(*parent)] * local_matrix,
                None => local_matrix,
            };
            global.push(matrix);
        }
        Ok(global)
    }
}

impl LoadableAsset for Skeleton {}

/// Runtime data built by the importer; scene-only, so the inspector does not edit it.
#[derive(Component, Clone, Reflect, Default)]
#[reflect(SceneComponent)]
pub struct SkeletonComponent {
    pub skeleton: AssetHandle<Skeleton>,
    pub bones: Vec<SceneEntityRef>,
    pub bone_ids: Vec<Uuid>,
    pub root: Option<SceneEntityRef>,
}

impl SkeletonComponent {
    pub fn new(
        skeleton: AssetHandle<Skeleton>,
        bones: Vec<SceneEntityRef>,
        bone_ids: Vec<Uuid>,
        root: Option<SceneEntityRef>,
    ) -> Self {
        Self {
            skeleton,
            bones,
            bone_ids,
            root,
        }
    }

    pub fn skeleton(&self) -> &AssetHandle<Skeleton> {
        &self.skeleton
    }

    pub fn bones(&self) -> &[SceneEntityRef] {
        &self.bones
    }

    pub fn bone_ids(&self) -> &[Uuid] {
        &self.bone_ids
    }

    pub fn root(&self) -> Option<Entity> {
        self.root.and_then(SceneEntityRef::entity)
    }
}
