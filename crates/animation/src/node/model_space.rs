use concerto_mesh::{JointName, Skeleton};
use glam::{Affine3A, Quat};

use crate::pose::JointPose;

/// A joint's transform and rotation in the skeleton's model space.
pub struct ModelJoint {
    /// The joint's model-space transform.
    pub transform: Affine3A,
    /// The joint's model-space rotation.
    pub rotation: Quat,
}

/// The inverse of a model-to-world transform, with its rotation split out.
pub struct WorldToModel {
    /// The world-to-model transform.
    pub transform: Affine3A,
    /// The world-to-model rotation.
    pub rotation: Quat,
}

impl WorldToModel {
    /// Inverts `model_to_world`.
    pub fn new(model_to_world: Affine3A) -> Self {
        let (_, rotation, _) = model_to_world.to_scale_rotation_translation();
        Self {
            transform: model_to_world.inverse(),
            rotation: rotation.inverse(),
        }
    }

    /// Whether the transform and rotation hold only finite values.
    pub fn is_finite(&self) -> bool {
        self.transform.is_finite() && self.rotation.is_finite()
    }
}

/// The parent of `bone`, or `None` for a root or an unknown bone.
pub fn parent(skeleton: &Skeleton, bone: usize) -> Option<usize> {
    skeleton
        .parents
        .get(bone)
        .copied()
        .flatten()
        .map(usize::from)
}

/// The index of the bone named `name`, if it descends from bone 0.
pub fn bone_under_first(skeleton: &Skeleton, name: JointName) -> Result<usize, String> {
    let bone = skeleton
        .bone_index(name.0)
        .ok_or_else(|| format!("the skeleton has no bone `{}`", &*name))?;
    let mut top = bone;
    while let Some(up) = parent(skeleton, top) {
        top = up;
    }
    if top == 0 {
        Ok(bone)
    } else {
        Err(format!("bone `{}` does not descend from bone 0", &*name))
    }
}

/// The model-space transform of `bone` in the local pose `joints`.
pub fn model_joint(skeleton: &Skeleton, joints: &[JointPose], bone: usize) -> Option<ModelJoint> {
    let joint = joints.get(bone)?;
    let local =
        Affine3A::from_scale_rotation_translation(joint.scale, joint.rotation, joint.translation);
    Some(match parent(skeleton, bone) {
        Some(up) => {
            let parent = model_joint(skeleton, joints, up)?;
            ModelJoint {
                transform: parent.transform * local,
                rotation: parent.rotation * joint.rotation,
            }
        }
        None => ModelJoint {
            transform: local,
            rotation: joint.rotation,
        },
    })
}
