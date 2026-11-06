use concerto_mesh::{JointName, Skeleton};
use glam::{Affine3A, Quat};

use crate::pose::JointPose;

pub(crate) struct ModelJoint {
    pub(crate) transform: Affine3A,
    pub(crate) rotation: Quat,
}

pub(crate) struct WorldToModel {
    pub(crate) transform: Affine3A,
    pub(crate) rotation: Quat,
}

impl WorldToModel {
    pub(crate) fn new(model_to_world: Affine3A) -> Self {
        let (_, rotation, _) = model_to_world.to_scale_rotation_translation();
        Self {
            transform: model_to_world.inverse(),
            rotation: rotation.inverse(),
        }
    }

    pub(crate) fn is_finite(&self) -> bool {
        self.transform.is_finite() && self.rotation.is_finite()
    }
}

pub(crate) fn parent(skeleton: &Skeleton, bone: usize) -> Option<usize> {
    skeleton
        .parents
        .get(bone)
        .copied()
        .flatten()
        .map(usize::from)
}

pub(crate) fn bone_under_first(skeleton: &Skeleton, name: JointName) -> Result<usize, String> {
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

pub(crate) fn model_joint(
    skeleton: &Skeleton,
    joints: &[JointPose],
    bone: usize,
) -> Option<ModelJoint> {
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
