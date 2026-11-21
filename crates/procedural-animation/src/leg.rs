use concerto_animation::{
    ik::{TwoBoneChain, solve_two_bone},
    node::model_space::{bone_under_first, model_joint, parent},
    pose::AnimationFrame,
};
use concerto_foundation::math::{Spring, SpringValue};
use concerto_mesh::{JointName, Skeleton};
use glam::{Quat, Vec3};

pub(crate) struct ResolvedLeg {
    pub(crate) hip: usize,
    pub(crate) knee: usize,
    pub(crate) foot: usize,
    pub(crate) toe: Option<usize>,
}

pub(crate) fn resolve_leg(
    skeleton: &Skeleton,
    foot: JointName,
    toe: Option<JointName>,
) -> Result<ResolvedLeg, String> {
    let foot_bone = bone_under_first(skeleton, foot)?;
    let knee =
        parent(skeleton, foot_bone).ok_or_else(|| format!("foot `{}` has no parent", &*foot))?;
    let hip =
        parent(skeleton, knee).ok_or_else(|| format!("foot `{}` has no grandparent", &*foot))?;
    let toe = toe.map(|toe| bone_under_first(skeleton, toe)).transpose()?;
    Ok(ResolvedLeg {
        hip,
        knee,
        foot: foot_bone,
        toe,
    })
}

pub(crate) fn elapsed_after(elapsed: f32, delta_time: f32) -> f32 {
    if delta_time.is_finite() && delta_time > 0.0 {
        elapsed + delta_time
    } else {
        elapsed
    }
}

pub(crate) fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

pub(crate) fn ease<T: SpringValue>(
    spring: &mut Spring<T>,
    target: T,
    halflife: f32,
    elapsed: f32,
) -> T {
    spring.set_halflife(halflife);
    if halflife > 0.0 {
        spring.update(target, elapsed)
    } else {
        spring.reset_to(target);
        target
    }
}

pub(crate) fn bend_leg(
    skeleton: &Skeleton,
    leg: &ResolvedLeg,
    goal_position: Vec3,
    goal_tilt: Quat,
    softness: f32,
    output: &mut AnimationFrame,
) {
    let joints = output.joints();
    let Some(hip) = model_joint(skeleton, joints, leg.hip) else {
        return;
    };
    let Some(knee) = model_joint(skeleton, joints, leg.knee) else {
        return;
    };
    let Some(foot) = model_joint(skeleton, joints, leg.foot) else {
        return;
    };
    let parent_rotation = match parent(skeleton, leg.hip) {
        Some(up) => match model_joint(skeleton, joints, up) {
            Some(joint) => joint.rotation,
            None => return,
        },
        None => Quat::IDENTITY,
    };
    let solution = solve_two_bone(
        &TwoBoneChain {
            root: hip.transform.translation.into(),
            mid: knee.transform.translation.into(),
            tip: foot.transform.translation.into(),
        },
        goal_position,
        None,
        softness,
    );
    let hip_rotation = solution.root * hip.rotation;
    let knee_rotation = solution.mid * knee.rotation;
    let foot_rotation = goal_tilt * foot.rotation;
    let locals = [
        LocalRotation {
            bone: leg.hip,
            rotation: parent_rotation.inverse() * hip_rotation,
        },
        LocalRotation {
            bone: leg.knee,
            rotation: hip_rotation.inverse() * knee_rotation,
        },
        LocalRotation {
            bone: leg.foot,
            rotation: knee_rotation.inverse() * foot_rotation,
        },
    ];
    for local in locals {
        if let Some(joint) = output.joint_mut(local.bone) {
            joint.rotation = local.rotation.normalize();
        }
    }
}

struct LocalRotation {
    bone: usize,
    rotation: Quat,
}
