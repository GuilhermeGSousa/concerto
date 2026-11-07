use std::fmt;

use concerto_foundation::assets::{Asset, CreatableAsset, LoadableAsset, handle::AssetHandle};
use concerto_mesh::{
    JointName, JointPose, ReflectSkeletonSource, Skeleton, SkeletonError, SkeletonSource,
};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Mat4, Quat};
use serde::{Deserialize, Serialize};

use crate::clip::AnimationClip;

/// A skeleton described for retargeting: its named bone chains, its retarget root and motion root, and the pose it is aligned in.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(SkeletonSource)]
pub struct RetargetRig {
    pub skeleton: AssetHandle<Skeleton>,
    pub root: JointName,
    pub motion_root: Option<JointName>,
    pub chains: Vec<RetargetChain>,
    pub reference: ReferencePose,
    pub corrections: Vec<BoneCorrection>,
}

/// A named run of bones from `start` down to `end`, matched to the chain of the same name on another rig.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RetargetChain {
    pub name: Name,
    pub start: JointName,
    pub end: JointName,
}

/// The pose a rig is aligned in before its corrections.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub enum ReferencePose {
    /// The skeleton's rest pose.
    #[default]
    Rest,
    /// The skeleton's rest pose with every channel of `clip` sampled at `time`.
    ClipFrame {
        clip: AssetHandle<AnimationClip>,
        time: f32,
    },
}

/// A rotation applied to one bone of the reference pose, in the bone's own space.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct BoneCorrection {
    pub bone: JointName,
    pub rotation: Quat,
}

impl Default for BoneCorrection {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            rotation: Quat::IDENTITY,
        }
    }
}

impl BoneCorrection {
    /// A correction turning `bone` by `rotation`, in the bone's own space.
    pub fn new(bone: impl Into<JointName>, rotation: Quat) -> Self {
        Self {
            bone: bone.into(),
            rotation,
        }
    }
}

impl RetargetChain {
    /// A chain called `name` from `start` down to `end`.
    pub fn new(
        name: impl Into<Name>,
        start: impl Into<JointName>,
        end: impl Into<JointName>,
    ) -> Self {
        Self {
            name: name.into(),
            start: start.into(),
            end: end.into(),
        }
    }
}

impl RetargetRig {
    /// A rig with no skeleton, root, chains or corrections, aligned in the rest pose.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the skeleton the rig describes.
    pub fn with_skeleton(mut self, skeleton: AssetHandle<Skeleton>) -> Self {
        self.skeleton = skeleton;
        self
    }

    /// Sets the retarget root, the pelvis.
    pub fn with_root(mut self, root: impl Into<JointName>) -> Self {
        self.root = root.into();
        self
    }

    /// Sets the bone above the root that carries the character's ground motion.
    pub fn with_motion_root(mut self, motion_root: impl Into<JointName>) -> Self {
        self.motion_root = Some(motion_root.into());
        self
    }

    /// Sets the pose the rig is aligned in before its corrections.
    pub fn with_reference(mut self, reference: ReferencePose) -> Self {
        self.reference = reference;
        self
    }

    /// Adds one chain, after the ones already there.
    pub fn with_chain(mut self, chain: RetargetChain) -> Self {
        self.chains.push(chain);
        self
    }

    /// Adds one correction, after the ones already there.
    pub fn with_correction(mut self, correction: BoneCorrection) -> Self {
        self.corrections.push(correction);
        self
    }

    /// Resolves the rig against `skeleton`, the one `self.skeleton` names, reading the reference frame from `clip`, the one `ReferencePose::ClipFrame` names.
    pub fn resolve(
        &self,
        skeleton: &Skeleton,
        clip: Option<&AnimationClip>,
    ) -> Result<ResolvedRig, RigError> {
        skeleton.validate().map_err(RigError::Skeleton)?;

        if self.root.is_empty() {
            return Err(RigError::MissingRoot);
        }
        let root = find_bone(skeleton, self.root, BoneUse::Root)?;

        let motion_root = match self.motion_root {
            Some(name) => {
                let motion_root = find_bone(skeleton, name, BoneUse::MotionRoot)?;
                if !is_strict_ancestor(skeleton, motion_root, root) {
                    return Err(RigError::MotionRootNotAbove {
                        motion_root: name,
                        root: self.root,
                    });
                }
                Some(motion_root)
            }
            None => None,
        };

        let mut chains: Vec<ResolvedChain> = Vec::with_capacity(self.chains.len());
        for (index, chain) in self.chains.iter().enumerate() {
            if chain.name.is_empty() {
                return Err(RigError::UnnamedChain { index });
            }
            if chains.iter().any(|seen| seen.name == chain.name) {
                return Err(RigError::DuplicateChain { chain: chain.name });
            }
            let start = find_bone(skeleton, chain.start, BoneUse::ChainStart(chain.name))?;
            let end = find_bone(skeleton, chain.end, BoneUse::ChainEnd(chain.name))?;
            let mut bones = vec![end];
            let mut bone = end;
            while bone != start {
                match skeleton.parents[bone] {
                    Some(parent) => {
                        bone = usize::from(parent);
                        bones.push(bone);
                    }
                    None => {
                        return Err(RigError::ChainEndNotBelowStart { chain: chain.name });
                    }
                }
            }
            bones.reverse();
            chains.push(ResolvedChain {
                name: chain.name,
                bones,
            });
        }

        let mut reference_local = match &self.reference {
            ReferencePose::Rest => skeleton.rest_pose.to_vec(),
            ReferencePose::ClipFrame { time, .. } => {
                let clip = clip.ok_or(RigError::MissingClip)?;
                if !time.is_finite() {
                    return Err(RigError::InvalidTime { time: *time });
                }
                if !skeleton
                    .bone_ids
                    .iter()
                    .any(|id| clip.get_channels(id).is_some())
                {
                    return Err(RigError::ClipNotForSkeleton);
                }
                sample_frame(skeleton, clip, *time)
            }
        };

        for correction in &self.corrections {
            let bone = find_bone(skeleton, correction.bone, BoneUse::Correction)?;
            let pose = &mut reference_local[bone];
            pose.rotation = (pose.rotation * correction.rotation).normalize();
        }

        let reference_model = skeleton
            .forward_kinematics(&reference_local)
            .map_err(RigError::Skeleton)?;

        let root_height = reference_model[root].w_axis.y;
        if !root_height.is_finite() || root_height <= 1e-4 {
            return Err(RigError::RootHeight {
                height: root_height,
            });
        }

        Ok(ResolvedRig {
            root,
            motion_root,
            chains,
            reference_local,
            reference_model,
            root_height,
        })
    }
}

impl Asset for RetargetRig {
    const FORMAT_VERSION: u32 = 1;

    fn name() -> &'static str {
        "RetargetRig"
    }
}

impl LoadableAsset for RetargetRig {}

impl CreatableAsset for RetargetRig {
    fn new_asset() -> Self {
        Self::new()
    }
}

impl SkeletonSource for RetargetRig {
    fn skeleton(&self) -> Option<&AssetHandle<Skeleton>> {
        Some(&self.skeleton)
    }
}

/// A rig resolved against its skeleton: bone indices, and the reference pose in local and model space.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRig {
    pub root: usize,
    pub motion_root: Option<usize>,
    pub chains: Vec<ResolvedChain>,
    pub reference_local: Vec<JointPose>,
    pub reference_model: Vec<Mat4>,
    pub root_height: f32,
}

impl ResolvedRig {
    /// The chain called `name`, if the rig has one.
    pub fn chain(&self, name: Name) -> Option<&ResolvedChain> {
        self.chains.iter().find(|chain| chain.name == name)
    }
}

/// A chain's name and its bones, from its start down to its end.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedChain {
    pub name: Name,
    pub bones: Vec<usize>,
}

/// What a rig bone name was used as, for errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoneUse {
    Root,
    MotionRoot,
    ChainStart(Name),
    ChainEnd(Name),
    Correction,
}

impl fmt::Display for BoneUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoneUse::Root => write!(f, "the root"),
            BoneUse::MotionRoot => write!(f, "the motion root"),
            BoneUse::ChainStart(chain) => write!(f, "the start of chain `{chain}`"),
            BoneUse::ChainEnd(chain) => write!(f, "the end of chain `{chain}`"),
            BoneUse::Correction => write!(f, "a correction"),
        }
    }
}

/// Why a rig could not be resolved against a skeleton.
#[derive(Clone, Debug, PartialEq)]
pub enum RigError {
    MissingRoot,
    UnknownBone {
        bone: JointName,
        used_as: BoneUse,
    },
    MotionRootNotAbove {
        motion_root: JointName,
        root: JointName,
    },
    UnnamedChain {
        index: usize,
    },
    DuplicateChain {
        chain: Name,
    },
    ChainEndNotBelowStart {
        chain: Name,
    },
    MissingClip,
    InvalidTime {
        time: f32,
    },
    ClipNotForSkeleton,
    RootHeight {
        height: f32,
    },
    Skeleton(SkeletonError),
}

impl fmt::Display for RigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RigError::MissingRoot => write!(f, "the rig names no root bone"),
            RigError::UnknownBone { bone, used_as } => write!(
                f,
                "bone `{}`, used as {used_as}, is not in the skeleton",
                bone.0
            ),
            RigError::MotionRootNotAbove { motion_root, root } => write!(
                f,
                "motion root `{}` is not an ancestor of root `{}`",
                motion_root.0, root.0
            ),
            RigError::UnnamedChain { index } => write!(f, "chain {index} has no name"),
            RigError::DuplicateChain { chain } => {
                write!(f, "chain `{chain}` is named more than once")
            }
            RigError::ChainEndNotBelowStart { chain } => {
                write!(f, "the end of chain `{chain}` is not below its start")
            }
            RigError::MissingClip => {
                write!(
                    f,
                    "the reference pose is a clip frame, but no clip was given"
                )
            }
            RigError::InvalidTime { time } => {
                write!(f, "the reference frame's time {time} is not finite")
            }
            RigError::ClipNotForSkeleton => write!(
                f,
                "the reference clip has channels for no bone of the skeleton"
            ),
            RigError::RootHeight { height } => write!(
                f,
                "the root is {height} above the model origin in the reference pose, which is not above 0.1 mm"
            ),
            RigError::Skeleton(error) => write!(f, "the skeleton is invalid: {error}"),
        }
    }
}

impl std::error::Error for RigError {}

fn find_bone(skeleton: &Skeleton, bone: JointName, used_as: BoneUse) -> Result<usize, RigError> {
    if bone.is_empty() {
        return Err(RigError::UnknownBone { bone, used_as });
    }
    skeleton
        .bone_index(bone.0)
        .ok_or(RigError::UnknownBone { bone, used_as })
}

fn is_strict_ancestor(skeleton: &Skeleton, ancestor: usize, bone: usize) -> bool {
    let mut current = skeleton.parents[bone];
    while let Some(parent) = current {
        let parent = usize::from(parent);
        if parent == ancestor {
            return true;
        }
        current = skeleton.parents[parent];
    }
    false
}

fn sample_frame(skeleton: &Skeleton, clip: &AnimationClip, time: f32) -> Vec<JointPose> {
    let mut pose = skeleton.rest_pose.to_vec();
    for (joint, id) in pose.iter_mut().zip(skeleton.bone_ids.iter()) {
        if let Some(channels) = clip.get_channels(id) {
            for channel in channels {
                channel.sample_transform(time, joint);
            }
        }
    }
    pose
}

#[cfg(test)]
mod tests {
    use concerto_mesh::{JointName, JointPose, Skeleton, SkeletonError};
    use concerto_name::Name;
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{BoneCorrection, BoneUse, ReferencePose, RetargetChain, RetargetRig, RigError};
    use crate::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};

    const NAMES: [&str; 14] = [
        "root",
        "pelvis",
        "spine_a",
        "spine_b",
        "spine_c",
        "neck",
        "head",
        "clavicle_l",
        "upper_l",
        "lower_l",
        "hand_l",
        "thigh_l",
        "calf_l",
        "foot_l",
    ];
    const PARENTS: [Option<u16>; 14] = [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(4),
        Some(7),
        Some(8),
        Some(9),
        Some(1),
        Some(11),
        Some(12),
    ];
    const ROOT: usize = 0;
    const PELVIS: usize = 1;
    const SPINE_A: usize = 2;
    const SPINE_C: usize = 4;
    const HEAD: usize = 6;
    const UPPER_L: usize = 8;
    const HAND_L: usize = 10;

    fn rest(bone: usize) -> JointPose {
        match bone {
            ROOT => JointPose {
                translation: Vec3::ZERO,
                rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                scale: Vec3::ONE,
            },
            PELVIS => JointPose {
                translation: Vec3::new(0.0, 0.0, 1.0),
                rotation: Quat::from_rotation_z(0.2),
                scale: Vec3::ONE,
            },
            _ => JointPose {
                translation: Vec3::new(0.0, 0.1, 0.0),
                rotation: Quat::from_euler(
                    glam::EulerRot::XYZ,
                    0.1 * bone as f32,
                    0.05,
                    -0.07 * bone as f32,
                ),
                scale: Vec3::ONE,
            },
        }
    }

    fn skeleton() -> Skeleton {
        let count = NAMES.len();
        Skeleton::new(
            vec![Mat4::IDENTITY; count],
            PARENTS.to_vec(),
            NAMES.iter().map(|name| Name::new(name)).collect(),
            (0..count).map(rest).collect(),
            (0..count as u128)
                .map(|bone| Uuid::from_u128(100 + bone))
                .collect(),
        )
        .unwrap()
    }

    fn bone_id(bone: usize) -> Uuid {
        Uuid::from_u128(100 + bone as u128)
    }

    fn rig() -> RetargetRig {
        RetargetRig::new().with_root("pelvis")
    }

    fn resolve_error(rig: &RetargetRig) -> RigError {
        rig.resolve(&skeleton(), None).unwrap_err()
    }

    fn assert_quat(actual: Quat, expected: Quat) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5),
            "{actual:?} != {expected:?}"
        );
    }

    fn assert_vec3(actual: Vec3, expected: Vec3) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn resolves_the_root_motion_root_and_chains_to_bone_indices() {
        let rig = rig()
            .with_motion_root("root")
            .with_chain(RetargetChain::new("spine", "spine_a", "spine_c"))
            .with_chain(RetargetChain::new("head", "head", "head"));

        let resolved = rig.resolve(&skeleton(), None).unwrap();

        assert_eq!(resolved.root, PELVIS);
        assert_eq!(resolved.motion_root, Some(ROOT));
        assert_eq!(resolved.chains.len(), 2);
        assert_eq!(
            resolved.chain(Name::new("spine")).unwrap().bones,
            vec![SPINE_A, 3, SPINE_C]
        );
        assert_eq!(resolved.chain(Name::new("head")).unwrap().bones, vec![HEAD]);
        assert!(resolved.chain(Name::new("tail")).is_none());
    }

    #[test]
    fn the_rest_reference_is_the_rest_pose_and_its_forward_kinematics() {
        let skeleton = skeleton();

        let resolved = rig().resolve(&skeleton, None).unwrap();

        assert_eq!(resolved.reference_local, skeleton.rest_pose.to_vec());
        assert_eq!(
            resolved.reference_model,
            skeleton.forward_kinematics(&skeleton.rest_pose).unwrap()
        );
        assert!(
            (resolved.root_height - 1.0).abs() < 1e-5,
            "{}",
            resolved.root_height
        );
    }

    #[test]
    fn a_clip_frame_reference_samples_each_bone_and_skips_one_key_channels() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone_id(UPPER_L),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Rotation(vec![Quat::IDENTITY, Quat::from_rotation_z(1.0)]),
            ),
        );
        clip.add_channel(
            bone_id(PELVIS),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![
                    Vec3::new(0.0, 0.0, 1.0),
                    Vec3::new(0.0, 0.0, 2.0),
                ]),
            ),
        );
        clip.add_channel(
            bone_id(HEAD),
            AnimationChannel::new(
                vec![0.0],
                AnimationChanelOutput::Rotation(vec![Quat::from_rotation_y(1.0)]),
            ),
        );
        let rig = rig().with_reference(ReferencePose::ClipFrame {
            clip: Default::default(),
            time: 0.5,
        });

        let resolved = rig.resolve(&skeleton(), Some(&clip)).unwrap();

        assert_quat(
            resolved.reference_local[UPPER_L].rotation,
            Quat::from_rotation_z(0.5),
        );
        assert_vec3(
            resolved.reference_local[UPPER_L].translation,
            rest(UPPER_L).translation,
        );
        assert_vec3(
            resolved.reference_local[PELVIS].translation,
            Vec3::new(0.0, 0.0, 1.5),
        );
        assert_quat(
            resolved.reference_local[PELVIS].rotation,
            rest(PELVIS).rotation,
        );
        for bone in (0..NAMES.len()).filter(|bone| *bone != UPPER_L && *bone != PELVIS) {
            assert_eq!(resolved.reference_local[bone], rest(bone), "bone {bone}");
        }
        assert!(
            (resolved.root_height - 1.5).abs() < 1e-5,
            "{}",
            resolved.root_height
        );
    }

    #[test]
    fn a_correction_turns_its_bone_in_its_own_space_and_moves_the_bones_below() {
        let correction = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let skeleton = skeleton();
        let rig = rig().with_correction(BoneCorrection::new("upper_l", correction));

        let resolved = rig.resolve(&skeleton, None).unwrap();

        let expected_rotation = rest(UPPER_L).rotation * correction;
        assert_quat(
            resolved.reference_local[UPPER_L].rotation,
            expected_rotation,
        );
        let mut expected_pose = skeleton.rest_pose.to_vec();
        expected_pose[UPPER_L].rotation = expected_rotation;
        let expected_model = skeleton.forward_kinematics(&expected_pose).unwrap();
        let rest_model = skeleton.forward_kinematics(&skeleton.rest_pose).unwrap();
        let hand = resolved.reference_model[HAND_L].w_axis.truncate();
        assert_vec3(hand, expected_model[HAND_L].w_axis.truncate());
        assert!(
            hand.distance(rest_model[HAND_L].w_axis.truncate()) > 1e-3,
            "{hand:?}"
        );
    }

    #[test]
    fn two_corrections_on_one_bone_compose_in_list_order() {
        let first = Quat::from_rotation_x(0.4);
        let second = Quat::from_rotation_y(0.7);
        let rig = rig()
            .with_correction(BoneCorrection::new("upper_l", first))
            .with_correction(BoneCorrection::new("upper_l", second));

        let resolved = rig.resolve(&skeleton(), None).unwrap();

        assert_quat(
            resolved.reference_local[UPPER_L].rotation,
            rest(UPPER_L).rotation * first * second,
        );
    }

    #[test]
    fn an_empty_root_is_missing() {
        assert_eq!(resolve_error(&RetargetRig::new()), RigError::MissingRoot);
    }

    #[test]
    fn an_unknown_root_is_an_unknown_bone_used_as_the_root() {
        assert_eq!(
            resolve_error(&RetargetRig::new().with_root("hips")),
            RigError::UnknownBone {
                bone: JointName::from("hips"),
                used_as: BoneUse::Root,
            }
        );
    }

    #[test]
    fn an_unknown_motion_root_is_an_unknown_bone_used_as_the_motion_root() {
        assert_eq!(
            resolve_error(&rig().with_motion_root("world")),
            RigError::UnknownBone {
                bone: JointName::from("world"),
                used_as: BoneUse::MotionRoot,
            }
        );
    }

    #[test]
    fn a_motion_root_below_the_root_is_not_above_it() {
        assert_eq!(
            resolve_error(&rig().with_motion_root("spine_a")),
            RigError::MotionRootNotAbove {
                motion_root: JointName::from("spine_a"),
                root: JointName::from("pelvis"),
            }
        );
    }

    #[test]
    fn a_motion_root_equal_to_the_root_is_not_above_it() {
        assert_eq!(
            resolve_error(&rig().with_motion_root("pelvis")),
            RigError::MotionRootNotAbove {
                motion_root: JointName::from("pelvis"),
                root: JointName::from("pelvis"),
            }
        );
    }

    #[test]
    fn an_empty_motion_root_is_an_unknown_bone_used_as_the_motion_root() {
        assert_eq!(
            resolve_error(&rig().with_motion_root("")),
            RigError::UnknownBone {
                bone: JointName::default(),
                used_as: BoneUse::MotionRoot,
            }
        );
    }

    #[test]
    fn an_empty_chain_start_is_an_unknown_bone_used_as_its_start() {
        assert_eq!(
            resolve_error(&rig().with_chain(RetargetChain::new("spine", "", "spine_c"))),
            RigError::UnknownBone {
                bone: JointName::default(),
                used_as: BoneUse::ChainStart(Name::new("spine")),
            }
        );
    }

    #[test]
    fn an_empty_correction_bone_is_an_unknown_bone_used_as_a_correction() {
        assert_eq!(
            resolve_error(&rig().with_correction(BoneCorrection::default())),
            RigError::UnknownBone {
                bone: JointName::default(),
                used_as: BoneUse::Correction,
            }
        );
    }

    #[test]
    fn a_skeleton_with_a_short_rest_pose_is_invalid() {
        let mut skeleton = skeleton();
        let mut rest_pose = skeleton.rest_pose.to_vec();
        rest_pose.pop();
        skeleton.rest_pose = rest_pose.into_boxed_slice();

        assert_eq!(
            rig().resolve(&skeleton, None).unwrap_err(),
            RigError::Skeleton(SkeletonError::LengthMismatch {
                field: "rest_pose",
                expected: NAMES.len(),
                actual: NAMES.len() - 1,
            })
        );
    }

    #[test]
    fn a_skeleton_with_a_parent_after_its_child_is_invalid() {
        let mut skeleton = skeleton();
        skeleton.parents[PELVIS] = Some(5);

        assert_eq!(
            rig().resolve(&skeleton, None).unwrap_err(),
            RigError::Skeleton(SkeletonError::ParentNotBefore {
                bone: PELVIS,
                parent: 5,
            })
        );
    }

    #[test]
    fn an_unknown_chain_start_names_its_chain() {
        assert_eq!(
            resolve_error(&rig().with_chain(RetargetChain::new("spine", "spine_0", "spine_c"))),
            RigError::UnknownBone {
                bone: JointName::from("spine_0"),
                used_as: BoneUse::ChainStart(Name::new("spine")),
            }
        );
    }

    #[test]
    fn an_unknown_chain_end_names_its_chain() {
        assert_eq!(
            resolve_error(&rig().with_chain(RetargetChain::new("spine", "spine_a", "spine_d"))),
            RigError::UnknownBone {
                bone: JointName::from("spine_d"),
                used_as: BoneUse::ChainEnd(Name::new("spine")),
            }
        );
    }

    #[test]
    fn a_chain_end_above_its_start_is_not_below_it() {
        assert_eq!(
            resolve_error(&rig().with_chain(RetargetChain::new("spine", "spine_c", "spine_a"))),
            RigError::ChainEndNotBelowStart {
                chain: Name::new("spine"),
            }
        );
    }

    #[test]
    fn a_chain_end_on_a_sibling_branch_is_not_below_its_start() {
        assert_eq!(
            resolve_error(&rig().with_chain(RetargetChain::new("arm_l", "thigh_l", "hand_l"))),
            RigError::ChainEndNotBelowStart {
                chain: Name::new("arm_l"),
            }
        );
    }

    #[test]
    fn a_chain_with_no_name_is_unnamed() {
        assert_eq!(
            resolve_error(
                &rig()
                    .with_chain(RetargetChain::new("head", "head", "head"))
                    .with_chain(RetargetChain::new("", "neck", "neck"))
            ),
            RigError::UnnamedChain { index: 1 }
        );
    }

    #[test]
    fn a_chain_name_used_twice_is_a_duplicate() {
        assert_eq!(
            resolve_error(
                &rig()
                    .with_chain(RetargetChain::new("head", "head", "head"))
                    .with_chain(RetargetChain::new("head", "neck", "head"))
            ),
            RigError::DuplicateChain {
                chain: Name::new("head"),
            }
        );
    }

    #[test]
    fn a_clip_frame_without_a_clip_is_missing_its_clip() {
        let rig = rig().with_reference(ReferencePose::ClipFrame {
            clip: Default::default(),
            time: 0.0,
        });

        assert_eq!(resolve_error(&rig), RigError::MissingClip);
    }

    #[test]
    fn a_clip_frame_at_a_nan_time_is_invalid() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone_id(PELVIS),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![Vec3::Z, Vec3::Z]),
            ),
        );
        let rig = rig().with_reference(ReferencePose::ClipFrame {
            clip: Default::default(),
            time: f32::NAN,
        });

        let error = rig.resolve(&skeleton(), Some(&clip)).unwrap_err();

        assert!(
            matches!(error, RigError::InvalidTime { time } if time.is_nan()),
            "{error:?}"
        );
    }

    #[test]
    fn a_clip_with_channels_for_no_bone_of_the_skeleton_is_not_for_it() {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(999),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![Vec3::Z, Vec3::Z]),
            ),
        );
        let rig = rig().with_reference(ReferencePose::ClipFrame {
            clip: Default::default(),
            time: 0.5,
        });

        assert_eq!(
            rig.resolve(&skeleton(), Some(&clip)).unwrap_err(),
            RigError::ClipNotForSkeleton
        );
    }

    #[test]
    fn an_unknown_correction_bone_is_an_unknown_bone_used_as_a_correction() {
        assert_eq!(
            resolve_error(&rig().with_correction(BoneCorrection::new("tail", Quat::IDENTITY))),
            RigError::UnknownBone {
                bone: JointName::from("tail"),
                used_as: BoneUse::Correction,
            }
        );
    }

    #[test]
    fn a_root_at_the_model_origin_has_no_height() {
        assert_eq!(
            resolve_error(&RetargetRig::new().with_root("root")),
            RigError::RootHeight { height: 0.0 }
        );
    }
}
