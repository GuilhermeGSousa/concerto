use std::{collections::HashSet, fmt};

use concerto_ecs::Res;
use concerto_foundation::assets::{
    Asset, CreatableAsset, LoadableAsset, asset_server::AssetLoadContext, asset_store::AssetStore,
    handle::AssetHandle,
};
use concerto_mesh::{
    JointName, JointPose, ReflectSkeletonSource, Skeleton, SkeletonError, SkeletonSource,
};
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Mat4, Quat, Vec3};
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    clip::AnimationClip,
    evaluation::{AnimationGraphContext, RootBone},
    graph::{AnimationGraph, AnimationGraphState},
    ik::{TwoBoneChain, solve_two_bone},
    node::{AnimationNode, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool, RootMotion, twist_about},
};

/// A skeleton described for retargeting: its named bone chains, its retarget root and motion root, and the pose it is aligned in.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(SkeletonSource, Default)]
pub struct RetargetRig {
    pub skeleton: AssetHandle<Skeleton>,
    pub root: JointName,
    pub motion_root: Option<JointName>,
    pub chains: Vec<RetargetChain>,
    pub reference: ReferencePose,
    pub corrections: Vec<BoneCorrection>,
    pub goals: Vec<RetargetGoal>,
}

/// A named run of bones from `start` down to `end`, matched to the chain of the same name on another rig.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RetargetChain {
    pub name: Name,
    pub start: JointName,
    pub end: JointName,
}

/// A named IK goal on a bone, matched to the goal of the same name on another rig; the bone is the tip of a two-bone chain made of it, its parent and its grandparent.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RetargetGoal {
    pub name: Name,
    pub bone: JointName,
}

impl RetargetGoal {
    /// A goal called `name` on `bone`.
    pub fn new(name: impl Into<Name>, bone: impl Into<JointName>) -> Self {
        Self {
            name: name.into(),
            bone: bone.into(),
        }
    }
}

/// How a retarget pairing solves one goal that both rigs name: `weight` 0 leaves the FK result and 1 reaches the goal; `softness` eases reach near full extension.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct RetargetGoalSettings {
    pub goal: Name,
    pub weight: f32,
    pub softness: f32,
}

impl Default for RetargetGoalSettings {
    fn default() -> Self {
        Self {
            goal: Name::default(),
            weight: 1.0,
            softness: 0.0,
        }
    }
}

impl RetargetGoalSettings {
    /// Settings for `goal` at weight 1 and softness 0.
    pub fn new(goal: impl Into<Name>) -> Self {
        Self {
            goal: goal.into(),
            ..Self::default()
        }
    }

    /// Sets how far toward the goal the tip moves, from 0 to 1.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Sets the distance below full extension where reach starts to ease.
    pub fn with_softness(mut self, softness: f32) -> Self {
        self.softness = softness;
        self
    }
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

    /// Adds one goal, after the ones already there.
    pub fn with_goal(mut self, goal: RetargetGoal) -> Self {
        self.goals.push(goal);
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

        let mut goals: Vec<ResolvedGoal> = Vec::with_capacity(self.goals.len());
        for (index, goal) in self.goals.iter().enumerate() {
            if goal.name.is_empty() {
                return Err(RigError::UnnamedGoal { index });
            }
            if goals.iter().any(|seen| seen.name == goal.name) {
                return Err(RigError::DuplicateGoal { goal: goal.name });
            }
            let tip = find_bone(skeleton, goal.bone, BoneUse::Goal(goal.name))?;
            let Some(mid) = skeleton.parents[tip].map(usize::from) else {
                return Err(RigError::GoalTooHigh { goal: goal.name });
            };
            let Some(goal_root) = skeleton.parents[mid].map(usize::from) else {
                return Err(RigError::GoalTooHigh { goal: goal.name });
            };
            let bends_root = [mid, goal_root].into_iter().any(|bone| {
                [Some(root), motion_root]
                    .into_iter()
                    .flatten()
                    .any(|pinned| bone == pinned || is_strict_ancestor(skeleton, bone, pinned))
            });
            if bends_root {
                return Err(RigError::GoalBendsRoot { goal: goal.name });
            }
            goals.push(ResolvedGoal {
                name: goal.name,
                tip,
                mid,
                root: goal_root,
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
                clip.sample_pose(skeleton, *time)
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
            goals,
            reference_local,
            reference_model,
            root_height,
        })
    }
}

impl Asset for RetargetRig {
    const FORMAT_VERSION: u32 = 2;

    fn name() -> &'static str {
        "RetargetRig"
    }

    fn migrate(version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        match version {
            1 => {
                let old: legacy::RetargetRigV1 = bincode::deserialize(payload)?;
                Ok(old.into())
            }
            _ => anyhow::bail!(
                "RetargetRig version {version} has no migration to version {}; re-create the rig in the editor",
                Self::FORMAT_VERSION
            ),
        }
    }
}

mod legacy {
    use concerto_foundation::assets::handle::AssetHandle;
    use concerto_mesh::{JointName, Skeleton};
    use serde::Deserialize;

    use super::{BoneCorrection, ReferencePose, RetargetChain, RetargetRig};

    #[derive(Deserialize)]
    pub(super) struct RetargetRigV1 {
        skeleton: AssetHandle<Skeleton>,
        root: JointName,
        motion_root: Option<JointName>,
        chains: Vec<RetargetChain>,
        reference: ReferencePose,
        corrections: Vec<BoneCorrection>,
    }

    impl From<RetargetRigV1> for RetargetRig {
        fn from(old: RetargetRigV1) -> Self {
            Self {
                skeleton: old.skeleton,
                root: old.root,
                motion_root: old.motion_root,
                chains: old.chains,
                reference: old.reference,
                corrections: old.corrections,
                goals: Vec::new(),
            }
        }
    }
}

impl LoadableAsset for RetargetRig {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        let server = context.asset_server();
        if !self.skeleton.id().is_nil() {
            self.skeleton = server.load(self.skeleton.id());
        }
        if let ReferencePose::ClipFrame { clip, .. } = &mut self.reference
            && !clip.id().is_nil()
        {
            *clip = server.load(clip.id());
        }
        Ok(())
    }
}

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
    pub goals: Vec<ResolvedGoal>,
    pub reference_local: Vec<JointPose>,
    pub reference_model: Vec<Mat4>,
    pub root_height: f32,
}

impl ResolvedRig {
    /// The chain called `name`, if the rig has one.
    pub fn chain(&self, name: Name) -> Option<&ResolvedChain> {
        self.chains.iter().find(|chain| chain.name == name)
    }

    /// The goal called `name`, if the rig has one.
    pub fn goal(&self, name: Name) -> Option<&ResolvedGoal> {
        self.goals.iter().find(|goal| goal.name == name)
    }
}

/// A chain's name and its bones, from its start down to its end.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedChain {
    pub name: Name,
    pub bones: Vec<usize>,
}

/// A goal's name and the bones it bends: `tip`, its parent `mid`, and the mid's parent `root`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGoal {
    pub name: Name,
    pub tip: usize,
    pub mid: usize,
    pub root: usize,
}

/// What a rig bone name was used as, for errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoneUse {
    Root,
    MotionRoot,
    ChainStart(Name),
    ChainEnd(Name),
    Correction,
    Goal(Name),
}

impl fmt::Display for BoneUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoneUse::Root => write!(f, "the root"),
            BoneUse::MotionRoot => write!(f, "the motion root"),
            BoneUse::ChainStart(chain) => write!(f, "the start of chain `{chain}`"),
            BoneUse::ChainEnd(chain) => write!(f, "the end of chain `{chain}`"),
            BoneUse::Correction => write!(f, "a correction"),
            BoneUse::Goal(goal) => write!(f, "the goal `{goal}`"),
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
    UnnamedGoal {
        index: usize,
    },
    DuplicateGoal {
        goal: Name,
    },
    GoalTooHigh {
        goal: Name,
    },
    GoalBendsRoot {
        goal: Name,
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
            RigError::UnnamedGoal { index } => write!(f, "goal {index} has no name"),
            RigError::DuplicateGoal { goal } => {
                write!(f, "goal `{goal}` is named more than once")
            }
            RigError::GoalTooHigh { goal } => write!(
                f,
                "the bone of goal `{goal}` has no parent and grandparent to bend"
            ),
            RigError::GoalBendsRoot { goal } => {
                write!(f, "goal `{goal}` would bend the root or the motion root")
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
                "the root is {height} m above the model origin in the reference pose, which is not above 0.1 mm"
            ),
            RigError::Skeleton(error) => write!(f, "the skeleton is invalid: {error}"),
        }
    }
}

impl std::error::Error for RigError {}

/// How to play a pose of one skeleton on another, built from the two skeletons' retarget rigs.
#[derive(Clone, Debug)]
pub struct PoseRetarget {
    source_parents: Box<[Option<u16>]>,
    source_reference_inverse: Box<[Quat]>,
    source_root: usize,
    source_root_reference: Vec3,
    source_motion_root: Option<usize>,
    source_motion_root_reference: Vec3,
    target_parents: Box<[Option<u16>]>,
    target_reference_local: Box<[JointPose]>,
    target_reference_rotation: Box<[Quat]>,
    target_root: usize,
    target_root_reference: Vec3,
    target_motion_root_reference: Vec3,
    rules: Box<[BoneRule]>,
    goals: Box<[GoalPair]>,
    scale: f32,
    source_rotation: Box<[Quat]>,
    source_model: Box<[Mat4]>,
    target_rotation: Box<[Quat]>,
    target_model: Box<[Mat4]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BoneRule {
    Reference,
    Chain(ChainSample),
    MotionRoot,
    Root,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ChainSample {
    lower: usize,
    upper: usize,
    alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GoalPair {
    name: Name,
    source_tip: usize,
    root: usize,
    mid: usize,
    tip: usize,
    offset: Vec3,
    weight: f32,
    softness: f32,
}

impl PoseRetarget {
    /// Pairs `source` and `target`, each resolved against the skeleton given beside it.
    pub fn new(
        source: &ResolvedRig,
        source_skeleton: &Skeleton,
        target: &ResolvedRig,
        target_skeleton: &Skeleton,
    ) -> Result<PoseRetarget, RetargetError> {
        if !rig_fits(source, source_skeleton) {
            return Err(RetargetError::RigNotForSkeleton {
                side: RetargetSide::Source,
            });
        }
        if !rig_fits(target, target_skeleton) {
            return Err(RetargetError::RigNotForSkeleton {
                side: RetargetSide::Target,
            });
        }
        if !target
            .chains
            .iter()
            .any(|chain| source.chain(chain.name).is_some())
        {
            return Err(RetargetError::NoSharedChain);
        }

        let source_reference = model_rotations(&source_skeleton.parents, &source.reference_local);
        let target_reference = model_rotations(&target_skeleton.parents, &target.reference_local);

        let mut rules = vec![BoneRule::Reference; target_skeleton.bone_count()];
        for target_chain in &target.chains {
            let Some(source_chain) = source.chain(target_chain.name) else {
                continue;
            };
            let source_parameters = chain_parameters(&source_chain.bones, &source.reference_model);
            let target_parameters = chain_parameters(&target_chain.bones, &target.reference_model);
            for (bone, t) in target_chain.bones.iter().zip(target_parameters) {
                rules[*bone] =
                    BoneRule::Chain(chain_sample(&source_chain.bones, &source_parameters, t));
            }
        }
        if let Some(motion_root) = target.motion_root {
            rules[motion_root] = BoneRule::MotionRoot;
        }
        rules[target.root] = BoneRule::Root;

        let scale = target.root_height / source.root_height;
        let source_root_reference = source.reference_model[source.root].w_axis.truncate();
        let target_root_reference = target.reference_model[target.root].w_axis.truncate();
        let goals = target
            .goals
            .iter()
            .filter_map(|goal| {
                let source_goal = source.goal(goal.name)?;
                let source_offset = source.reference_model[source_goal.tip].w_axis.truncate()
                    - source_root_reference;
                Some(GoalPair {
                    name: goal.name,
                    source_tip: source_goal.tip,
                    root: goal.root,
                    mid: goal.mid,
                    tip: goal.tip,
                    offset: target.reference_model[goal.tip].w_axis.truncate()
                        - (target_root_reference + source_offset * scale),
                    weight: 1.0,
                    softness: 0.0,
                })
            })
            .collect();

        Ok(PoseRetarget {
            source_parents: source_skeleton.parents.clone(),
            source_reference_inverse: source_reference
                .iter()
                .map(|rotation| rotation.inverse())
                .collect(),
            source_root: source.root,
            source_root_reference,
            source_motion_root: source.motion_root,
            source_motion_root_reference: reference_position(
                source.motion_root,
                &source.reference_model,
            ),
            target_parents: target_skeleton.parents.clone(),
            target_reference_local: target.reference_local.clone().into_boxed_slice(),
            target_reference_rotation: target_reference.into_boxed_slice(),
            target_root: target.root,
            target_root_reference,
            target_motion_root_reference: reference_position(
                target.motion_root,
                &target.reference_model,
            ),
            rules: rules.into_boxed_slice(),
            goals,
            scale,
            source_rotation: vec![Quat::IDENTITY; source_skeleton.bone_count()].into_boxed_slice(),
            source_model: vec![Mat4::IDENTITY; source_skeleton.bone_count()].into_boxed_slice(),
            target_rotation: vec![Quat::IDENTITY; target_skeleton.bone_count()].into_boxed_slice(),
            target_model: vec![Mat4::IDENTITY; target_skeleton.bone_count()].into_boxed_slice(),
        })
    }

    /// Writes `source`, a pose of the source skeleton in bone order, retargeted onto the target skeleton into `out`.
    pub fn retarget_into(
        &mut self,
        source: &[JointPose],
        out: &mut [JointPose],
    ) -> Result<(), RetargetError> {
        if source.len() != self.source_parents.len() {
            return Err(RetargetError::PoseLength {
                side: RetargetSide::Source,
                expected: self.source_parents.len(),
                actual: source.len(),
            });
        }
        if out.len() != self.target_parents.len() {
            return Err(RetargetError::PoseLength {
                side: RetargetSide::Target,
                expected: self.target_parents.len(),
                actual: out.len(),
            });
        }

        for (bone, pose) in source.iter().enumerate() {
            match self.source_parents[bone] {
                Some(parent) => {
                    let parent = usize::from(parent);
                    self.source_rotation[bone] =
                        (self.source_rotation[parent] * pose.rotation).normalize();
                    self.source_model[bone] = self.source_model[parent] * pose.to_mat4();
                }
                None => {
                    self.source_rotation[bone] = pose.rotation.normalize();
                    self.source_model[bone] = pose.to_mat4();
                }
            }
        }

        for (bone, target) in out.iter_mut().enumerate() {
            let parent = self.target_parents[bone].map(usize::from);
            let mut pose = self.target_reference_local[bone].clone();
            match self.rules[bone] {
                BoneRule::Reference => {}
                BoneRule::Chain(sample) => {
                    let delta = self
                        .source_delta(sample.lower)
                        .slerp(self.source_delta(sample.upper), sample.alpha);
                    pose.rotation =
                        self.local_rotation(parent, delta * self.target_reference_rotation[bone]);
                }
                BoneRule::MotionRoot => match self.source_motion_root {
                    Some(motion_root) => {
                        let desired =
                            self.source_delta(motion_root) * self.target_reference_rotation[bone];
                        let offset =
                            self.source_position(motion_root) - self.source_motion_root_reference;
                        let position = self.target_motion_root_reference + offset * self.scale;
                        self.place(parent, &mut pose, desired, position);
                    }
                    None => {
                        let desired = twist_about(self.source_delta(self.source_root), Vec3::Y)
                            * self.target_reference_rotation[bone];
                        let offset = (self.source_position(self.source_root)
                            - self.source_root_reference)
                            * self.scale;
                        let position =
                            self.target_motion_root_reference + Vec3::new(offset.x, 0.0, offset.z);
                        self.place(parent, &mut pose, desired, position);
                    }
                },
                BoneRule::Root => {
                    let desired =
                        self.source_delta(self.source_root) * self.target_reference_rotation[bone];
                    let offset =
                        self.source_position(self.source_root) - self.source_root_reference;
                    let position = self.target_root_reference + offset * self.scale;
                    self.place(parent, &mut pose, desired, position);
                }
            }
            self.store_target(bone, &pose);
            *target = pose;
        }

        for index in 0..self.goals.len() {
            let pair = self.goals[index];
            self.solve_goal(pair, out);
        }
        Ok(())
    }

    /// Sets the weight and softness of the goal `settings` names; fails, changing nothing, when the two rigs do not both name it.
    pub fn set_goal(&mut self, settings: &RetargetGoalSettings) -> Result<(), RetargetError> {
        let pair = self
            .goals
            .iter_mut()
            .find(|pair| pair.name == settings.goal)
            .ok_or(RetargetError::UnknownGoal {
                goal: settings.goal,
            })?;
        pair.weight = settings.weight;
        pair.softness = settings.softness;
        Ok(())
    }

    /// The scale from source to target root motion: the target root's reference height over the source's.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    fn solve_goal(&mut self, pair: GoalPair, out: &mut [JointPose]) {
        let weight = if pair.weight.is_nan() {
            0.0
        } else {
            pair.weight.clamp(0.0, 1.0)
        };
        if weight <= 0.0 {
            return;
        }
        let full = self.target_position(self.target_root)
            + (self.source_position(pair.source_tip) - self.source_position(self.source_root))
                * self.scale
            + twist_about(self.source_delta(self.source_root), Vec3::Y) * pair.offset;
        let tip = self.target_position(pair.tip);
        let chain = TwoBoneChain {
            root: self.target_position(pair.root),
            mid: self.target_position(pair.mid),
            tip,
        };
        let solution = solve_two_bone(&chain, tip.lerp(full, weight), None, pair.softness);
        let root_rotation = (solution.root * self.target_rotation[pair.root]).normalize();
        let mid_rotation = (solution.mid * self.target_rotation[pair.mid]).normalize();
        let tip_rotation = self.target_rotation[pair.tip];
        out[pair.root].rotation = self.local_rotation(
            self.target_parents[pair.root].map(usize::from),
            root_rotation,
        );
        out[pair.mid].rotation = (root_rotation.inverse() * mid_rotation).normalize();
        out[pair.tip].rotation = (mid_rotation.inverse() * tip_rotation).normalize();
        for (bone, pose) in out.iter().enumerate().skip(pair.root) {
            self.store_target(bone, pose);
        }
    }

    fn store_target(&mut self, bone: usize, pose: &JointPose) {
        match self.target_parents[bone].map(usize::from) {
            Some(parent) => {
                self.target_rotation[bone] =
                    (self.target_rotation[parent] * pose.rotation).normalize();
                self.target_model[bone] = self.target_model[parent] * pose.to_mat4();
            }
            None => {
                self.target_rotation[bone] = pose.rotation.normalize();
                self.target_model[bone] = pose.to_mat4();
            }
        }
    }

    fn target_position(&self, bone: usize) -> Vec3 {
        self.target_model[bone].w_axis.truncate()
    }

    fn source_delta(&self, bone: usize) -> Quat {
        self.source_rotation[bone] * self.source_reference_inverse[bone]
    }

    fn source_position(&self, bone: usize) -> Vec3 {
        self.source_model[bone].w_axis.truncate()
    }

    fn local_rotation(&self, parent: Option<usize>, desired: Quat) -> Quat {
        match parent {
            Some(parent) => (self.target_rotation[parent].inverse() * desired).normalize(),
            None => desired.normalize(),
        }
    }

    fn place(&self, parent: Option<usize>, pose: &mut JointPose, desired: Quat, position: Vec3) {
        pose.rotation = self.local_rotation(parent, desired);
        pose.translation = match parent {
            Some(parent) => self.target_model[parent]
                .inverse()
                .transform_point3(position),
            None => position,
        };
    }
}

/// Which side of a retarget a skeleton, rig or pose is on, for errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetargetSide {
    Source,
    Target,
}

impl fmt::Display for RetargetSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetargetSide::Source => write!(f, "source"),
            RetargetSide::Target => write!(f, "target"),
        }
    }
}

/// Why a retarget could not be built or applied.
#[derive(Clone, Debug, PartialEq)]
pub enum RetargetError {
    RigNotForSkeleton {
        side: RetargetSide,
    },
    NoSharedChain,
    PoseLength {
        side: RetargetSide,
        expected: usize,
        actual: usize,
    },
    UnknownGoal {
        goal: Name,
    },
}

impl fmt::Display for RetargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetargetError::RigNotForSkeleton { side } => write!(
                f,
                "the {side} rig does not fit the {side} skeleton it was given"
            ),
            RetargetError::NoSharedChain => {
                write!(f, "the source and target rigs share no chain name")
            }
            RetargetError::PoseLength {
                side,
                expected,
                actual,
            } => write!(
                f,
                "the {side} pose has {actual} bones, but the {side} skeleton has {expected}"
            ),
            RetargetError::UnknownGoal { goal } => {
                write!(
                    f,
                    "the source and target rigs do not both name goal `{goal}`"
                )
            }
        }
    }
}

impl std::error::Error for RetargetError {}

/// Plays `graph` on the source rig's skeleton, retargeted onto the player's, with nodes that read the world (foot placement, IK, look-at) kept outside it; it fails, until the player's graph changes, on a nested retarget node or an inner node whose `CreateInput` is not an update input.
#[derive(Clone, Default, Reflect, Serialize, Deserialize)]
#[reflect(AnimationNode)]
pub struct RetargetNode {
    pub graph: AssetHandle<AnimationGraph>,
    pub source: AssetHandle<RetargetRig>,
    pub target: AssetHandle<RetargetRig>,
    pub goals: Vec<RetargetGoalSettings>,
}

impl RetargetNode {
    /// A node playing `graph` on the skeleton of `source`, retargeted through `target` onto the player's skeleton.
    pub fn new(
        graph: AssetHandle<AnimationGraph>,
        source: AssetHandle<RetargetRig>,
        target: AssetHandle<RetargetRig>,
    ) -> Self {
        Self {
            graph,
            source,
            target,
            goals: Vec::new(),
        }
    }

    /// Adds the settings of one goal that both rigs name; a later entry for the same goal wins.
    pub fn with_goal(mut self, settings: RetargetGoalSettings) -> Self {
        self.goals.push(settings);
        self
    }

    fn start(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        skeletons: &AssetStore<Skeleton>,
        rigs: &AssetStore<RetargetRig>,
        clips: &AssetStore<AnimationClip>,
        context: &AnimationGraphContext<'_>,
    ) -> Option<Stage> {
        let player = context.skeleton()?;
        let source_rig = rigs.get(&self.source)?;
        let target_rig = rigs.get(&self.target)?;
        let source_skeleton = skeletons.get(&source_rig.skeleton)?;
        let target_skeleton = skeletons.get(&target_rig.skeleton)?;
        let source_clip = reference_clip(source_rig, clips)?;
        let target_clip = reference_clip(target_rig, clips)?;
        let inner_graph = graphs.get(&self.graph)?;
        if !inner_graph.is_live() {
            return None;
        }

        let source = match source_rig.resolve(source_skeleton, source_clip) {
            Ok(source) => source,
            Err(error) => return Some(failed(format!("the source rig: {error}"), None)),
        };
        let target = match target_rig.resolve(target_skeleton, target_clip) {
            Ok(target) => target,
            Err(error) => return Some(failed(format!("the target rig: {error}"), None)),
        };
        if target_skeleton.bone_ids != player.bone_ids {
            return Some(failed(
                "the target rig's skeleton is not the player's skeleton",
                None,
            ));
        }
        if holds_retarget_node(inner_graph, graphs) {
            return Some(failed(
                "its inner graph holds a retarget node",
                Some(target.reference_local.into_boxed_slice()),
            ));
        }
        let retarget = match PoseRetarget::new(&source, source_skeleton, &target, target_skeleton) {
            Ok(mut retarget) => {
                for settings in &self.goals {
                    if let Err(error) = retarget.set_goal(settings) {
                        warn!("RetargetNode ignores its settings for a goal: {error}");
                    }
                }
                retarget
            }
            Err(error) => {
                return Some(failed(
                    error,
                    Some(target.reference_local.into_boxed_slice()),
                ));
            }
        };

        let source_root = source_skeleton
            .parents
            .iter()
            .position(Option::is_none)
            .unwrap_or(0);
        let mut running = Running {
            inner: AnimationGraphState::default(),
            retarget,
            pool: FramePool::new(source_skeleton.bone_count()),
            source_skeleton: source_rig.skeleton.clone(),
            source_root,
            source_root_reference: source.reference_local[source_root].clone(),
            player_bone_ids: player.bone_ids.clone(),
            finished: false,
            warned: false,
        };
        running.pool.set_rest(&source_skeleton.rest_pose);
        let inner_context = running.inner_context(context, source_skeleton);
        running
            .inner
            .initialize(self.graph.clone(), graphs, &inner_context);
        if let Some(node) = running.inner.stateless_node(graphs) {
            return Some(failed(
                format!(
                    "its inner node {node} has no state; its CreateInput must be an update input"
                ),
                Some(target.reference_local.into_boxed_slice()),
            ));
        }
        Some(Stage::Ready(Box::new(running)))
    }
}

/// A [`RetargetNode`]'s progress: waiting for its assets, playing, or failed.
pub struct RetargetNodeState {
    stage: Stage,
    reset_pending: bool,
}

impl RetargetNodeState {
    pub(crate) fn inner(&self) -> Option<&AnimationGraphState> {
        match &self.stage {
            Stage::Ready(running) => Some(&running.inner),
            Stage::Pending | Stage::Failed { .. } => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn inner_mut(&mut self) -> Option<&mut AnimationGraphState> {
        match &mut self.stage {
            Stage::Ready(running) => Some(&mut running.inner),
            Stage::Pending | Stage::Failed { .. } => None,
        }
    }
}

enum Stage {
    Pending,
    Ready(Box<Running>),
    Failed { fallback: Option<Box<[JointPose]>> },
}

struct Running {
    inner: AnimationGraphState,
    retarget: PoseRetarget,
    pool: FramePool,
    source_skeleton: AssetHandle<Skeleton>,
    source_root: usize,
    source_root_reference: JointPose,
    player_bone_ids: Box<[Uuid]>,
    finished: bool,
    warned: bool,
}

impl Running {
    fn inner_context<'a>(
        &self,
        context: &AnimationGraphContext<'a>,
        skeleton: &'a Skeleton,
    ) -> AnimationGraphContext<'a> {
        AnimationGraphContext {
            blackboard: context.blackboard,
            root: context.root().map(|root| RootBone {
                index: self.source_root,
                rest: self.source_root_reference.clone(),
                up: root.up,
            }),
            skeleton: Some(skeleton),
            model_to_world: context.model_to_world,
            inputs: context.inputs,
        }
    }

    fn play(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        context: &AnimationGraphContext<'_>,
        source: &Skeleton,
        output: &mut AnimationFrame,
    ) {
        let inner_context = self.inner_context(context, source);
        let mut frame = self.pool.acquire();
        if self.inner.evaluate(
            graphs,
            &inner_context,
            &source.bone_ids,
            &mut self.pool,
            &mut frame,
        ) {
            match self
                .retarget
                .retarget_into(frame.joints(), output.pose_mut().joints_mut())
            {
                Ok(()) => {
                    let motion = frame.root_motion();
                    output.set_root_motion(RootMotion {
                        translation: motion.translation * self.retarget.scale(),
                        rotation: motion.rotation,
                    });
                }
                Err(error) => {
                    if !self.warned {
                        warn!("RetargetNode does not retarget its inner pose: {error}");
                        self.warned = true;
                    }
                }
            }
            for event in frame.events() {
                output.push_event(*event);
            }
        }
        self.pool.release(frame);
    }
}

fn failed(reason: impl fmt::Display, fallback: Option<Box<[JointPose]>>) -> Stage {
    warn!("RetargetNode does not play: {reason}");
    Stage::Failed { fallback }
}

fn holds_retarget_node(graph: &AnimationGraph, graphs: &AssetStore<AnimationGraph>) -> bool {
    let mut seen = HashSet::new();
    let mut pending = vec![graph];
    while let Some(graph) = pending.pop() {
        for node in graph.live_nodes() {
            if node.as_reflect().downcast_ref::<RetargetNode>().is_some() {
                return true;
            }
            node.inner_graphs(&mut |handle| {
                if seen.insert(handle.id())
                    && let Some(state_graph) = graphs.get(handle)
                {
                    pending.push(state_graph);
                }
            });
        }
    }
    false
}

fn reference_clip<'a>(
    rig: &RetargetRig,
    clips: &'a AssetStore<AnimationClip>,
) -> Option<Option<&'a AnimationClip>> {
    match &rig.reference {
        ReferencePose::Rest => Some(None),
        ReferencePose::ClipFrame { clip, .. } => clips.get(clip).map(Some),
    }
}

impl AnimationNode for RetargetNode {
    type State = RetargetNodeState;
    type CreateInput = ();
    type UpdateInput = (
        Res<'static, AssetStore<AnimationGraph>>,
        Res<'static, AssetStore<Skeleton>>,
        Res<'static, AssetStore<RetargetRig>>,
        Res<'static, AssetStore<AnimationClip>>,
    );
    type EvaluateInput = (
        Res<'static, AssetStore<AnimationGraph>>,
        Res<'static, AssetStore<Skeleton>>,
    );

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> RetargetNodeState {
        RetargetNodeState {
            stage: Stage::Pending,
            reset_pending: false,
        }
    }

    fn update(
        &self,
        state: &mut RetargetNodeState,
        delta_time: f32,
        (graphs, skeletons, rigs, clips): &(
            Res<AssetStore<AnimationGraph>>,
            Res<AssetStore<Skeleton>>,
            Res<AssetStore<RetargetRig>>,
            Res<AssetStore<AnimationClip>>,
        ),
        context: &AnimationGraphContext<'_>,
    ) {
        if matches!(state.stage, Stage::Pending)
            && let Some(stage) = self.start(graphs, skeletons, rigs, clips, context)
        {
            state.stage = stage;
        }
        let Stage::Ready(running) = &mut state.stage else {
            return;
        };
        if context
            .skeleton()
            .is_some_and(|player| player.bone_ids != running.player_bone_ids)
        {
            state.stage = failed("the player's skeleton changed", None);
            return;
        }
        let Some(source) = skeletons.get(&running.source_skeleton) else {
            return;
        };
        if state.reset_pending {
            running.inner.reset(graphs);
            state.reset_pending = false;
        }
        let inner_context = running.inner_context(context, source);
        running.inner.update(delta_time, graphs, &inner_context);
        running.finished = running.inner.is_finished(graphs);
    }

    fn evaluate(
        &self,
        state: &mut RetargetNodeState,
        (graphs, skeletons): &(Res<AssetStore<AnimationGraph>>, Res<AssetStore<Skeleton>>),
        context: &AnimationGraphContext<'_>,
        _bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedFrame],
        _pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        match &mut state.stage {
            Stage::Pending => {}
            Stage::Ready(running) => {
                if let Some(source) = skeletons.get(&running.source_skeleton) {
                    running.play(graphs, context, source, output);
                }
            }
            Stage::Failed { fallback } => {
                if let Some(fallback) = fallback {
                    for (joint, reference) in output
                        .pose_mut()
                        .joints_mut()
                        .iter_mut()
                        .zip(fallback.iter())
                    {
                        *joint = reference.clone();
                    }
                }
            }
        }
    }

    fn reset(&self, state: &mut RetargetNodeState) {
        state.reset_pending = true;
        if let Stage::Ready(running) = &mut state.stage {
            running.finished = false;
        }
    }

    fn is_finished(&self, state: &RetargetNodeState) -> bool {
        match &state.stage {
            Stage::Pending => false,
            Stage::Ready(running) => running.finished,
            Stage::Failed { .. } => true,
        }
    }

    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {
        visit(&self.graph);
    }

    fn inner_states(&self, state: &RetargetNodeState, visit: &mut dyn FnMut(&AnimationGraphState)) {
        if let Some(inner) = state.inner() {
            visit(inner);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut RetargetNodeState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    ) {
        if let Some(inner) = state.inner_mut() {
            visit(inner);
        }
    }
}

fn rig_fits(rig: &ResolvedRig, skeleton: &Skeleton) -> bool {
    let count = skeleton.bone_count();
    skeleton.validate().is_ok()
        && rig.reference_local.len() == count
        && rig.reference_model.len() == count
        && rig.root < count
        && rig
            .chains
            .iter()
            .all(|chain| !chain.bones.is_empty() && chain.bones.iter().all(|bone| *bone < count))
        && rig.motion_root.is_none_or(|motion_root| {
            motion_root < count && is_strict_ancestor(skeleton, motion_root, rig.root)
        })
        && rig.goals.iter().all(|goal| {
            goal.tip < count
                && goal.mid < count
                && goal.root < count
                && skeleton.parents[goal.tip].map(usize::from) == Some(goal.mid)
                && skeleton.parents[goal.mid].map(usize::from) == Some(goal.root)
        })
        && rig.root_height.is_finite()
        && rig.root_height > 1e-4
}

fn model_rotations(parents: &[Option<u16>], local: &[JointPose]) -> Vec<Quat> {
    let mut rotations: Vec<Quat> = Vec::with_capacity(local.len());
    for (pose, parent) in local.iter().zip(parents.iter()) {
        let rotation = match parent {
            Some(parent) => rotations[usize::from(*parent)] * pose.rotation,
            None => pose.rotation,
        };
        rotations.push(rotation.normalize());
    }
    rotations
}

fn reference_position(bone: Option<usize>, reference_model: &[Mat4]) -> Vec3 {
    bone.map_or(Vec3::ZERO, |bone| reference_model[bone].w_axis.truncate())
}

fn chain_parameters(bones: &[usize], reference_model: &[Mat4]) -> Vec<f32> {
    if bones.len() < 2 {
        return vec![0.0; bones.len()];
    }
    let last = bones.len() - 1;
    let lengths: Vec<f32> = bones
        .windows(2)
        .map(|pair| {
            reference_model[pair[1]]
                .w_axis
                .truncate()
                .distance(reference_model[pair[0]].w_axis.truncate())
        })
        .collect();
    let total: f32 = lengths.iter().sum();
    if total < 1e-6 {
        return (0..=last).map(|i| i as f32 / last as f32).collect();
    }
    let mut parameters: Vec<f32> = Vec::with_capacity(bones.len());
    parameters.push(0.0);
    let mut along: f32 = 0.0;
    for length in &lengths[..last - 1] {
        along += *length;
        parameters.push(along / total);
    }
    parameters.push(1.0);
    parameters
}

fn chain_sample(bones: &[usize], parameters: &[f32], t: f32) -> ChainSample {
    let lower = parameters
        .iter()
        .rposition(|parameter| *parameter <= t)
        .unwrap_or(0);
    let at = |index: usize| ChainSample {
        lower: bones[index],
        upper: bones[index],
        alpha: 0.0,
    };
    if lower + 1 >= bones.len() {
        return at(lower);
    }
    let alpha = (t - parameters[lower]) / (parameters[lower + 1] - parameters[lower]);
    if alpha <= 0.0 {
        return at(lower);
    }
    ChainSample {
        lower: bones[lower],
        upper: bones[lower + 1],
        alpha,
    }
}

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

#[cfg(test)]
mod tests {
    use std::{
        sync::{Mutex, Once},
        thread::ThreadId,
    };

    use concerto_ecs::{
        World,
        resource::{Res, Resource},
        system::input::{SystemInputData, register_input},
    };
    use concerto_foundation::assets::{
        Asset, AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use concerto_mesh::{JointName, JointPose, Skeleton, SkeletonError};
    use concerto_name::{Name, name};
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{
        BoneCorrection, BoneUse, PoseRetarget, ReferencePose, ResolvedGoal, ResolvedRig,
        RetargetChain, RetargetError, RetargetGoal, RetargetGoalSettings, RetargetNode,
        RetargetRig, RetargetSide, RigError,
    };
    use crate::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::{AnimationGraphContext, RootBone},
        event::AnimationEvent,
        graph::{AnimationGraph, made_live},
        ik::{TwoBoneChain, solve_two_bone},
        node::{
            AnimationClipNode, AnimationNode, AnimationPlayMode, CreateNodeInputs,
            ReflectAnimationNode, state_machine::AnimationStateMachine,
        },
        player::AnimationPlayer,
        plugin::register_animation_types,
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool},
        test_walks::Walks,
    };

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
    const THIGH_L: usize = 11;
    const CALF_L: usize = 12;
    const FOOT_L: usize = 13;

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
    fn loading_a_rig_loads_its_skeleton_and_its_reference_clip() {
        let mut server = AssetServer::new();
        let rigs = AssetStore::<RetargetRig>::new();
        let skeletons = AssetStore::<Skeleton>::new();
        let clips = AssetStore::<AnimationClip>::new();
        server.register_asset(&rigs);
        server.register_asset(&skeletons);
        server.register_asset(&clips);
        let skeleton = AssetId::new();
        let clip = AssetId::new();

        server.add(
            rig()
                .with_skeleton(AssetHandle::weak(skeleton))
                .with_reference(ReferencePose::ClipFrame {
                    clip: AssetHandle::weak(clip),
                    time: 0.0,
                }),
        );

        assert!(server.is_loading_or_loaded(skeleton));
        assert!(server.is_loading_or_loaded(clip));
    }

    #[test]
    fn a_rig_resolves_its_root_motion_root_chains_and_goals_to_bone_indices() {
        let rig = rig()
            .with_motion_root("root")
            .with_chain(RetargetChain::new("spine", "spine_a", "spine_c"))
            .with_chain(RetargetChain::new("head", "head", "head"))
            .with_goal(RetargetGoal::new("foot_l", "foot_l"))
            .with_goal(RetargetGoal::new("hand_l", "hand_l"));

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
        assert_eq!(
            resolved.goals,
            vec![
                ResolvedGoal {
                    name: Name::new("foot_l"),
                    tip: FOOT_L,
                    mid: CALF_L,
                    root: THIGH_L,
                },
                ResolvedGoal {
                    name: Name::new("hand_l"),
                    tip: HAND_L,
                    mid: 9,
                    root: UPPER_L,
                },
            ]
        );
        assert_eq!(resolved.goal(Name::new("hand_l")).unwrap().tip, HAND_L);
        assert!(resolved.goal(Name::new("hand_r")).is_none());
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
    fn corrections_turn_their_bone_in_its_own_space_in_list_order_and_move_the_bones_below() {
        let first = Quat::from_rotation_x(0.4);
        let second = Quat::from_rotation_y(0.7);
        let skeleton = skeleton();
        let rig = rig()
            .with_correction(BoneCorrection::new("upper_l", first))
            .with_correction(BoneCorrection::new("upper_l", second));

        let resolved = rig.resolve(&skeleton, None).unwrap();

        let expected_rotation = rest(UPPER_L).rotation * first * second;
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

    struct RigRefusal {
        name: &'static str,
        rig: RetargetRig,
        skeleton: Skeleton,
        clip: Option<AnimationClip>,
        error: RigError,
    }

    fn refusal(name: &'static str, rig: RetargetRig, error: RigError) -> RigRefusal {
        RigRefusal {
            name,
            rig,
            skeleton: skeleton(),
            clip: None,
            error,
        }
    }

    fn unknown(bone: &str, used_as: BoneUse) -> RigError {
        RigError::UnknownBone {
            bone: JointName::from(bone),
            used_as,
        }
    }

    fn clip_frame_at(time: f32) -> RetargetRig {
        rig().with_reference(ReferencePose::ClipFrame {
            clip: Default::default(),
            time,
        })
    }

    fn translating(bone: Uuid) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            bone,
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![Vec3::Z, Vec3::Z]),
            ),
        );
        clip
    }

    #[test]
    fn a_rig_that_does_not_fit_its_skeleton_is_refused_naming_why() {
        let spine = || Name::new("spine");
        let foot = || Name::new("foot_l");
        let mut short_rest = skeleton();
        let mut rest_pose = short_rest.rest_pose.to_vec();
        rest_pose.pop();
        short_rest.rest_pose = rest_pose.into_boxed_slice();
        let mut parent_after_child = skeleton();
        parent_after_child.parents[PELVIS] = Some(5);
        let cases = [
            refusal("no root", RetargetRig::new(), RigError::MissingRoot),
            refusal(
                "unknown root",
                RetargetRig::new().with_root("hips"),
                unknown("hips", BoneUse::Root),
            ),
            refusal(
                "unknown motion root",
                rig().with_motion_root("world"),
                unknown("world", BoneUse::MotionRoot),
            ),
            refusal(
                "empty motion root",
                rig().with_motion_root(""),
                unknown("", BoneUse::MotionRoot),
            ),
            refusal(
                "motion root below the root",
                rig().with_motion_root("spine_a"),
                RigError::MotionRootNotAbove {
                    motion_root: JointName::from("spine_a"),
                    root: JointName::from("pelvis"),
                },
            ),
            refusal(
                "motion root equal to the root",
                rig().with_motion_root("pelvis"),
                RigError::MotionRootNotAbove {
                    motion_root: JointName::from("pelvis"),
                    root: JointName::from("pelvis"),
                },
            ),
            refusal(
                "empty chain start",
                rig().with_chain(RetargetChain::new("spine", "", "spine_c")),
                unknown("", BoneUse::ChainStart(spine())),
            ),
            refusal(
                "unknown chain start",
                rig().with_chain(RetargetChain::new("spine", "spine_0", "spine_c")),
                unknown("spine_0", BoneUse::ChainStart(spine())),
            ),
            refusal(
                "unknown chain end",
                rig().with_chain(RetargetChain::new("spine", "spine_a", "spine_d")),
                unknown("spine_d", BoneUse::ChainEnd(spine())),
            ),
            refusal(
                "chain end above its start",
                rig().with_chain(RetargetChain::new("spine", "spine_c", "spine_a")),
                RigError::ChainEndNotBelowStart { chain: spine() },
            ),
            refusal(
                "chain end on a sibling branch",
                rig().with_chain(RetargetChain::new("arm_l", "thigh_l", "hand_l")),
                RigError::ChainEndNotBelowStart {
                    chain: Name::new("arm_l"),
                },
            ),
            refusal(
                "unnamed chain",
                rig()
                    .with_chain(RetargetChain::new("head", "head", "head"))
                    .with_chain(RetargetChain::new("", "neck", "neck")),
                RigError::UnnamedChain { index: 1 },
            ),
            refusal(
                "chain named twice",
                rig()
                    .with_chain(RetargetChain::new("head", "head", "head"))
                    .with_chain(RetargetChain::new("head", "neck", "head")),
                RigError::DuplicateChain {
                    chain: Name::new("head"),
                },
            ),
            refusal(
                "empty correction bone",
                rig().with_correction(BoneCorrection::default()),
                unknown("", BoneUse::Correction),
            ),
            refusal(
                "unknown correction bone",
                rig().with_correction(BoneCorrection::new("tail", Quat::IDENTITY)),
                unknown("tail", BoneUse::Correction),
            ),
            refusal(
                "unnamed goal",
                rig()
                    .with_goal(RetargetGoal::new("foot_l", "foot_l"))
                    .with_goal(RetargetGoal::new("", "hand_l")),
                RigError::UnnamedGoal { index: 1 },
            ),
            refusal(
                "goal named twice",
                rig()
                    .with_goal(RetargetGoal::new("foot_l", "foot_l"))
                    .with_goal(RetargetGoal::new("foot_l", "hand_l")),
                RigError::DuplicateGoal { goal: foot() },
            ),
            refusal(
                "unknown goal bone",
                rig().with_goal(RetargetGoal::new("foot_l", "ball_l")),
                unknown("ball_l", BoneUse::Goal(foot())),
            ),
            refusal(
                "empty goal bone",
                rig().with_goal(RetargetGoal::new("foot_l", "")),
                unknown("", BoneUse::Goal(foot())),
            ),
            refusal(
                "goal bone with no grandparent",
                rig().with_goal(RetargetGoal::new("top", "pelvis")),
                RigError::GoalTooHigh {
                    goal: Name::new("top"),
                },
            ),
            refusal(
                "goal whose root is the rig root",
                rig().with_goal(RetargetGoal::new("knee_l", "calf_l")),
                RigError::GoalBendsRoot {
                    goal: Name::new("knee_l"),
                },
            ),
            refusal(
                "clip frame without a clip",
                clip_frame_at(0.0),
                RigError::MissingClip,
            ),
            RigRefusal {
                clip: Some(translating(bone_id(PELVIS))),
                ..refusal(
                    "clip frame at a NaN time",
                    clip_frame_at(f32::NAN),
                    RigError::InvalidTime { time: f32::NAN },
                )
            },
            RigRefusal {
                clip: Some(translating(Uuid::from_u128(999))),
                ..refusal(
                    "clip with channels for no bone of the skeleton",
                    clip_frame_at(0.5),
                    RigError::ClipNotForSkeleton,
                )
            },
            refusal(
                "root at the model origin",
                RetargetRig::new().with_root("root"),
                RigError::RootHeight { height: 0.0 },
            ),
            RigRefusal {
                skeleton: short_rest,
                ..refusal(
                    "skeleton with a short rest pose",
                    rig(),
                    RigError::Skeleton(SkeletonError::LengthMismatch {
                        field: "rest_pose",
                        expected: NAMES.len(),
                        actual: NAMES.len() - 1,
                    }),
                )
            },
            RigRefusal {
                skeleton: parent_after_child,
                ..refusal(
                    "skeleton with a parent after its child",
                    rig(),
                    RigError::Skeleton(SkeletonError::ParentNotBefore {
                        bone: PELVIS,
                        parent: 5,
                    }),
                )
            },
        ];

        for case in cases {
            let error = case
                .rig
                .resolve(&case.skeleton, case.clip.as_ref())
                .unwrap_err();
            assert_eq!(
                format!("{error:?}"),
                format!("{:?}", case.error),
                "{}",
                case.name
            );
        }
    }

    struct FixtureBone {
        name: &'static str,
        parent: Option<&'static str>,
        rotation: Quat,
        position: Vec3,
    }

    fn fixture_bone(
        name: &'static str,
        parent: Option<&'static str>,
        rotation: Quat,
        position: Vec3,
    ) -> FixtureBone {
        FixtureBone {
            name,
            parent,
            rotation,
            position,
        }
    }

    fn fixture_skeleton(bones: &[FixtureBone], first_id: u128) -> Skeleton {
        let parent_index = |bone: &FixtureBone| {
            bone.parent
                .map(|parent| bones.iter().position(|other| other.name == parent).unwrap())
        };
        let rest_pose = bones
            .iter()
            .map(|bone| match parent_index(bone) {
                Some(parent) => {
                    let inverse = bones[parent].rotation.inverse();
                    JointPose {
                        translation: inverse * (bone.position - bones[parent].position),
                        rotation: (inverse * bone.rotation).normalize(),
                        scale: Vec3::ONE,
                    }
                }
                None => JointPose {
                    translation: bone.position,
                    rotation: bone.rotation,
                    scale: Vec3::ONE,
                },
            })
            .collect();
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones
                .iter()
                .map(|bone| parent_index(bone).map(|parent| parent as u16))
                .collect(),
            bones.iter().map(|bone| Name::new(bone.name)).collect(),
            rest_pose,
            (0..bones.len() as u128)
                .map(|bone| Uuid::from_u128(first_id + bone))
                .collect(),
        )
        .unwrap()
    }

    fn source_rotation(index: usize) -> Quat {
        Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.05 + 0.1 * index as f32, -0.2)
    }

    fn target_rotation(index: usize) -> Quat {
        Quat::from_euler(glam::EulerRot::YXZ, 0.25 * index as f32, 0.4, -0.1)
    }

    fn source_skeleton() -> Skeleton {
        source_skeleton_turned(&source_rotation)
    }

    fn source_skeleton_turned(rotation: &dyn Fn(usize) -> Quat) -> Skeleton {
        let bones = [
            fixture_bone("hips", None, rotation(0), Vec3::new(0.0, 1.0, 0.0)),
            fixture_bone(
                "spine_1",
                Some("hips"),
                rotation(1),
                Vec3::new(0.0, 1.1, 0.0),
            ),
            fixture_bone(
                "spine_2",
                Some("spine_1"),
                rotation(2),
                Vec3::new(0.0, 1.3, 0.0),
            ),
            fixture_bone(
                "spine_3",
                Some("spine_2"),
                rotation(3),
                Vec3::new(0.0, 1.5, 0.0),
            ),
            fixture_bone(
                "neck_1",
                Some("spine_3"),
                rotation(4),
                Vec3::new(0.0, 1.6, 0.0),
            ),
            fixture_bone(
                "neck_2",
                Some("neck_1"),
                rotation(5),
                Vec3::new(0.0, 1.7, 0.0),
            ),
            fixture_bone(
                "head",
                Some("neck_2"),
                rotation(6),
                Vec3::new(0.0, 1.8, 0.0),
            ),
            fixture_bone(
                "upper_l",
                Some("spine_3"),
                rotation(7),
                Vec3::new(0.2, 1.5, 0.0),
            ),
            fixture_bone(
                "lower_l",
                Some("upper_l"),
                rotation(8),
                Vec3::new(0.5, 1.5, 0.0),
            ),
            fixture_bone(
                "hand_l",
                Some("lower_l"),
                rotation(9),
                Vec3::new(0.8, 1.5, 0.0),
            ),
            fixture_bone(
                "fin_l",
                Some("hand_l"),
                rotation(10),
                Vec3::new(0.9, 1.5, 0.0),
            ),
            fixture_bone(
                "thigh_l",
                Some("hips"),
                rotation(11),
                Vec3::new(0.1, 0.95, 0.0),
            ),
            fixture_bone(
                "shin_l",
                Some("thigh_l"),
                rotation(12),
                Vec3::new(0.1, 0.5, 0.0),
            ),
            fixture_bone(
                "foot_l",
                Some("shin_l"),
                rotation(13),
                Vec3::new(0.1, 0.05, 0.0),
            ),
        ];
        fixture_skeleton(&bones, 1000)
    }

    fn target_skeleton() -> Skeleton {
        target_skeleton_turned(&target_rotation)
    }

    fn target_skeleton_turned(rotation: &dyn Fn(usize) -> Quat) -> Skeleton {
        let bones = [
            fixture_bone(
                "root",
                None,
                Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                Vec3::ZERO,
            ),
            fixture_bone(
                "pelvis",
                Some("root"),
                rotation(1),
                Vec3::new(0.0, 0.5, 0.0),
            ),
            fixture_bone(
                "spine_01",
                Some("pelvis"),
                rotation(2),
                Vec3::new(0.0, 0.55, 0.0),
            ),
            fixture_bone(
                "spine_02",
                Some("spine_01"),
                rotation(3),
                Vec3::new(0.0, 0.6, 0.0),
            ),
            fixture_bone(
                "spine_03",
                Some("spine_02"),
                rotation(4),
                Vec3::new(0.0, 0.7, 0.0),
            ),
            fixture_bone(
                "spine_04",
                Some("spine_03"),
                rotation(5),
                Vec3::new(0.0, 0.75, 0.0),
            ),
            fixture_bone(
                "neck_01",
                Some("spine_04"),
                rotation(6),
                Vec3::new(0.0, 0.8, 0.0),
            ),
            fixture_bone(
                "head",
                Some("neck_01"),
                rotation(7),
                Vec3::new(0.0, 0.9, 0.0),
            ),
            fixture_bone(
                "clavicle_l",
                Some("spine_04"),
                rotation(8),
                Vec3::new(0.05, 0.75, 0.0),
            ),
            fixture_bone(
                "upperarm_l",
                Some("clavicle_l"),
                rotation(9),
                Vec3::new(0.1, 0.75, 0.0),
            ),
            fixture_bone(
                "lowerarm_l",
                Some("upperarm_l"),
                rotation(10),
                Vec3::new(0.25, 0.75, 0.0),
            ),
            fixture_bone(
                "hand_l",
                Some("lowerarm_l"),
                rotation(11),
                Vec3::new(0.4, 0.75, 0.0),
            ),
            fixture_bone(
                "thigh_l",
                Some("pelvis"),
                rotation(12),
                Vec3::new(0.05, 0.47, 0.0),
            ),
            fixture_bone(
                "calf_l",
                Some("thigh_l"),
                rotation(13),
                Vec3::new(0.05, 0.25, 0.0),
            ),
            fixture_bone(
                "foot_l",
                Some("calf_l"),
                rotation(14),
                Vec3::new(0.05, 0.03, 0.0),
            ),
            fixture_bone(
                "tail",
                Some("pelvis"),
                rotation(15),
                Vec3::new(0.0, 0.45, -0.1),
            ),
        ];
        fixture_skeleton(&bones, 2000)
    }

    fn source_rig_asset() -> RetargetRig {
        RetargetRig::new()
            .with_root("hips")
            .with_chain(RetargetChain::new("spine", "spine_1", "spine_3"))
            .with_chain(RetargetChain::new("neck", "neck_1", "neck_2"))
            .with_chain(RetargetChain::new("head", "head", "head"))
            .with_chain(RetargetChain::new("arm_l", "upper_l", "hand_l"))
            .with_chain(RetargetChain::new("leg_l", "thigh_l", "foot_l"))
            .with_chain(RetargetChain::new("fin_l", "fin_l", "fin_l"))
    }

    fn source_rig() -> ResolvedRig {
        source_rig_asset()
            .resolve(&source_skeleton(), None)
            .unwrap()
    }

    fn doubled_target_skeleton() -> Skeleton {
        let mut skeleton = target_skeleton();
        for joint in skeleton.rest_pose.iter_mut() {
            joint.translation *= 2.0;
        }
        skeleton
    }

    fn target_rig() -> ResolvedRig {
        target_rig_on(&target_skeleton())
    }

    fn target_rig_asset() -> RetargetRig {
        RetargetRig::new()
            .with_motion_root("root")
            .with_root("pelvis")
            .with_chain(RetargetChain::new("spine", "spine_01", "spine_04"))
            .with_chain(RetargetChain::new("neck", "neck_01", "neck_01"))
            .with_chain(RetargetChain::new("head", "head", "head"))
            .with_chain(RetargetChain::new("arm_l", "upperarm_l", "hand_l"))
            .with_chain(RetargetChain::new("leg_l", "thigh_l", "foot_l"))
            .with_chain(RetargetChain::new("tail", "tail", "tail"))
    }

    fn target_rig_on(skeleton: &Skeleton) -> ResolvedRig {
        target_rig_asset().resolve(skeleton, None).unwrap()
    }

    fn bone(skeleton: &Skeleton, name: &str) -> usize {
        skeleton.bone_index(Name::new(name)).unwrap()
    }

    fn model_rotations(skeleton: &Skeleton, pose: &[JointPose]) -> Vec<Quat> {
        let mut rotations: Vec<Quat> = Vec::with_capacity(pose.len());
        for (joint, parent) in pose.iter().zip(skeleton.parents.iter()) {
            let rotation = match parent {
                Some(parent) => rotations[usize::from(*parent)] * joint.rotation,
                None => joint.rotation,
            };
            rotations.push(rotation.normalize());
        }
        rotations
    }

    fn posed(skeleton: &Skeleton, deltas: &[Quat]) -> Vec<JointPose> {
        let desired: Vec<Quat> = model_rotations(skeleton, &skeleton.rest_pose)
            .iter()
            .zip(deltas.iter())
            .map(|(rest, delta)| *delta * *rest)
            .collect();
        skeleton
            .rest_pose
            .iter()
            .zip(skeleton.parents.iter())
            .enumerate()
            .map(|(bone, (joint, parent))| JointPose {
                rotation: match parent {
                    Some(parent) => {
                        (desired[usize::from(*parent)].inverse() * desired[bone]).normalize()
                    }
                    None => desired[bone],
                },
                ..joint.clone()
            })
            .collect()
    }

    fn model_deltas(skeleton: &Skeleton, pose: &[JointPose]) -> Vec<Quat> {
        model_rotations(skeleton, pose)
            .iter()
            .zip(model_rotations(skeleton, &skeleton.rest_pose).iter())
            .map(|(rotation, rest)| *rotation * rest.inverse())
            .collect()
    }

    fn model_position(model: &[Mat4], bone: usize) -> Vec3 {
        model[bone].w_axis.truncate()
    }

    fn source_to_target() -> PoseRetarget {
        PoseRetarget::new(
            &source_rig(),
            &source_skeleton(),
            &target_rig(),
            &target_skeleton(),
        )
        .unwrap()
    }

    fn retarget(
        pose_retarget: &mut PoseRetarget,
        source: &[JointPose],
        count: usize,
    ) -> Vec<JointPose> {
        let mut out = vec![JointPose::IDENTITY; count];
        pose_retarget.retarget_into(source, &mut out).unwrap();
        out
    }

    fn assert_pose(actual: &JointPose, expected: &JointPose) {
        assert_vec3(actual.translation, expected.translation);
        assert_quat(actual.rotation, expected.rotation);
        assert_vec3(actual.scale, expected.scale);
    }

    fn about_x(degrees: f32) -> Quat {
        Quat::from_rotation_x(degrees.to_radians())
    }

    fn turn_then_tilt() -> Quat {
        about_x(10.0) * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    }

    struct RigOn {
        rig: RetargetRig,
        skeleton: Skeleton,
    }

    impl RigOn {
        fn resolved(&self) -> ResolvedRig {
            self.rig.resolve(&self.skeleton, None).unwrap()
        }

        fn onto(&self, target: &RigOn) -> PoseRetarget {
            PoseRetarget::new(
                &self.resolved(),
                &self.skeleton,
                &target.resolved(),
                &target.skeleton,
            )
            .unwrap()
        }
    }

    fn any_turn() -> impl Strategy<Value = Quat> {
        prop::array::uniform3(-3.1f32..3.1)
            .prop_map(|[x, y, z]| Quat::from_euler(glam::EulerRot::XYZ, x, y, z))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn a_source_at_its_reference_gives_the_target_reference_both_ways(
            source_turns in prop::collection::vec(any_turn(), 16),
            target_turns in prop::collection::vec(any_turn(), 16),
            goals in any::<bool>(),
        ) {
            let source_turn = |index: usize| source_turns[index];
            let target_turn = |index: usize| target_turns[index];
            let rigs = if goals {
                [
                    RigOn {
                        rig: with_feet(goal_source_rig_asset()),
                        skeleton: goal_source_skeleton_turned(&source_turn),
                    },
                    RigOn {
                        rig: with_feet(goal_target_rig_asset()),
                        skeleton: goal_target_skeleton_turned(&target_turn),
                    },
                ]
            } else {
                [
                    RigOn {
                        rig: source_rig_asset(),
                        skeleton: source_skeleton_turned(&source_turn),
                    },
                    RigOn {
                        rig: target_rig_asset(),
                        skeleton: target_skeleton_turned(&target_turn),
                    },
                ]
            };

            for [from, onto] in [[&rigs[0], &rigs[1]], [&rigs[1], &rigs[0]]] {
                let out = retarget(
                    &mut from.onto(onto),
                    &from.skeleton.rest_pose,
                    onto.skeleton.bone_count(),
                );
                for (actual, expected) in out.iter().zip(onto.skeleton.rest_pose.iter()) {
                    assert_pose(actual, expected);
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Way {
        SourceToTarget,
        TargetToSource,
        TargetToDoubled,
    }

    impl Way {
        fn rigs(self) -> [RigOn; 2] {
            let source = || RigOn {
                rig: source_rig_asset(),
                skeleton: source_skeleton(),
            };
            let target = || RigOn {
                rig: target_rig_asset(),
                skeleton: target_skeleton(),
            };
            match self {
                Way::SourceToTarget => [source(), target()],
                Way::TargetToSource => [target(), source()],
                Way::TargetToDoubled => [
                    target(),
                    RigOn {
                        rig: target_rig_asset(),
                        skeleton: doubled_target_skeleton(),
                    },
                ],
            }
        }
    }

    struct Turn {
        bone: &'static str,
        delta: Quat,
    }

    fn turn(bone: &'static str, delta: Quat) -> Turn {
        Turn { bone, delta }
    }

    struct ChainCase {
        name: &'static str,
        way: Way,
        source: Vec<Turn>,
        target: Vec<Turn>,
    }

    #[test]
    fn a_target_chain_turns_as_the_source_chain_sampled_along_its_length() {
        let cases = [
            ChainCase {
                name: "equal chains",
                way: Way::SourceToTarget,
                source: vec![
                    turn(
                        "upper_l",
                        Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.4, 0.6),
                    ),
                    turn(
                        "lower_l",
                        Quat::from_euler(glam::EulerRot::XYZ, -0.3, 0.5, 1.1),
                    ),
                ],
                target: vec![
                    turn(
                        "upperarm_l",
                        Quat::from_euler(glam::EulerRot::XYZ, 0.2, -0.4, 0.6),
                    ),
                    turn(
                        "lowerarm_l",
                        Quat::from_euler(glam::EulerRot::XYZ, -0.3, 0.5, 1.1),
                    ),
                    turn("hand_l", Quat::IDENTITY),
                ],
            },
            ChainCase {
                name: "a longer target chain",
                way: Way::SourceToTarget,
                source: vec![
                    turn("spine_1", about_x(20.0)),
                    turn("spine_2", about_x(40.0)),
                    turn("spine_3", about_x(60.0)),
                ],
                target: vec![
                    turn("spine_01", about_x(20.0)),
                    turn("spine_02", about_x(30.0)),
                    turn("spine_03", about_x(50.0)),
                    turn("spine_04", about_x(60.0)),
                ],
            },
            ChainCase {
                name: "a shorter target chain",
                way: Way::TargetToSource,
                source: vec![
                    turn("spine_01", about_x(20.0)),
                    turn("spine_02", about_x(30.0)),
                    turn("spine_03", about_x(50.0)),
                    turn("spine_04", about_x(60.0)),
                ],
                target: vec![
                    turn("spine_1", about_x(20.0)),
                    turn("spine_2", about_x(40.0)),
                    turn("spine_3", about_x(60.0)),
                ],
            },
            ChainCase {
                name: "a single bone target chain takes the source start",
                way: Way::SourceToTarget,
                source: vec![
                    turn("neck_1", about_x(25.0)),
                    turn("neck_2", Quat::from_rotation_z(-0.5)),
                    turn("head", Quat::from_rotation_y(0.7)),
                ],
                target: vec![
                    turn("neck_01", about_x(25.0)),
                    turn("head", Quat::from_rotation_y(0.7)),
                ],
            },
            ChainCase {
                name: "the root",
                way: Way::SourceToTarget,
                source: vec![turn("hips", Quat::from_rotation_y(0.7))],
                target: vec![turn("pelvis", Quat::from_rotation_y(0.7))],
            },
        ];

        for case in cases {
            let [from, onto] = case.way.rigs();
            let mut deltas = vec![Quat::IDENTITY; from.skeleton.bone_count()];
            for turn in &case.source {
                deltas[bone(&from.skeleton, turn.bone)] = turn.delta;
            }

            let out = retarget(
                &mut from.onto(&onto),
                &posed(&from.skeleton, &deltas),
                onto.skeleton.bone_count(),
            );

            let out_deltas = model_deltas(&onto.skeleton, &out);
            for turn in &case.target {
                let actual = out_deltas[bone(&onto.skeleton, turn.bone)];
                assert!(
                    actual.abs_diff_eq(turn.delta, 1e-5) || actual.abs_diff_eq(-turn.delta, 1e-5),
                    "{}: {} turned {actual:?}, not {:?}",
                    case.name,
                    turn.bone,
                    turn.delta
                );
            }
        }
    }

    #[test]
    fn unpaired_bones_and_chains_keep_the_reference() {
        let source_skeleton = source_skeleton();
        let target_skeleton = target_skeleton();
        let deltas: Vec<Quat> = (0..source_skeleton.bone_count())
            .map(|bone| {
                Quat::from_euler(
                    glam::EulerRot::XYZ,
                    0.1 + 0.05 * bone as f32,
                    -0.2 + 0.03 * bone as f32,
                    0.15,
                )
            })
            .collect();
        let source = posed(&source_skeleton, &deltas);
        let mut retarget_pose = source_to_target();

        let out = retarget(&mut retarget_pose, &source, target_skeleton.bone_count());

        for name in ["clavicle_l", "tail"] {
            let index = bone(&target_skeleton, name);
            assert_pose(&out[index], &target_skeleton.rest_pose[index]);
        }
        let pelvis = bone(&target_skeleton, "pelvis");
        let root = bone(&target_skeleton, "root");
        for (index, (actual, expected)) in
            out.iter().zip(target_skeleton.rest_pose.iter()).enumerate()
        {
            if index == pelvis || index == root {
                continue;
            }
            assert_vec3(actual.translation, expected.translation);
            assert_vec3(actual.scale, expected.scale);
        }

        let mut turned = source;
        let fin = bone(&source_skeleton, "fin_l");
        turned[fin].rotation = Quat::from_rotation_z(1.0) * turned[fin].rotation;
        assert_eq!(
            retarget(&mut retarget_pose, &turned, target_skeleton.bone_count()),
            out
        );
    }

    struct Moved {
        bone: &'static str,
        by: Vec3,
        turned: Option<Quat>,
    }

    struct RootCase {
        name: &'static str,
        way: Way,
        source: fn(&Skeleton) -> Vec<JointPose>,
        moved: Vec<Moved>,
    }

    #[test]
    fn the_root_and_motion_root_carry_the_scaled_source_root_motion() {
        let cases = [
            RootCase {
                name: "the root moves by the scaled source root offset",
                way: Way::SourceToTarget,
                source: |skeleton| {
                    let mut pose = skeleton.rest_pose.to_vec();
                    pose[bone(skeleton, "hips")].translation += Vec3::new(0.2, -0.1, 0.4);
                    pose
                },
                moved: vec![Moved {
                    bone: "pelvis",
                    by: Vec3::new(0.1, -0.05, 0.2),
                    turned: None,
                }],
            },
            RootCase {
                name: "a generated motion root takes the ground part of the root motion",
                way: Way::SourceToTarget,
                source: |skeleton| {
                    let hips = bone(skeleton, "hips");
                    let mut deltas = vec![Quat::IDENTITY; skeleton.bone_count()];
                    deltas[hips] = turn_then_tilt();
                    let mut pose = posed(skeleton, &deltas);
                    pose[hips].translation += Vec3::new(1.0, -0.1, 2.0);
                    pose
                },
                moved: vec![
                    Moved {
                        bone: "root",
                        by: Vec3::new(0.5, 0.0, 1.0),
                        turned: Some(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                    },
                    Moved {
                        bone: "pelvis",
                        by: Vec3::new(0.5, -0.05, 1.0),
                        turned: Some(turn_then_tilt()),
                    },
                ],
            },
            RootCase {
                name: "a source motion root is copied onto the target motion root",
                way: Way::TargetToDoubled,
                source: |skeleton| {
                    let mut pose = skeleton.rest_pose.to_vec();
                    let root = bone(skeleton, "root");
                    pose[root].rotation = turn_then_tilt() * pose[root].rotation;
                    pose[root].translation += Vec3::new(1.0, 0.2, 0.5);
                    let pelvis = bone(skeleton, "pelvis");
                    pose[pelvis].rotation = Quat::from_rotation_y(0.6) * pose[pelvis].rotation;
                    pose
                },
                moved: vec![Moved {
                    bone: "root",
                    by: Vec3::new(2.0, 0.4, 1.0),
                    turned: Some(turn_then_tilt()),
                }],
            },
            RootCase {
                name: "a motion root onto a target without one moves the root",
                way: Way::TargetToSource,
                source: |skeleton| {
                    let mut pose = skeleton.rest_pose.to_vec();
                    pose[bone(skeleton, "root")].translation += Vec3::new(1.0, 0.0, 0.0);
                    pose
                },
                moved: vec![Moved {
                    bone: "hips",
                    by: Vec3::new(2.0, 0.0, 0.0),
                    turned: None,
                }],
            },
        ];

        for case in cases {
            let [from, onto] = case.way.rigs();

            let out = retarget(
                &mut from.onto(&onto),
                &(case.source)(&from.skeleton),
                onto.skeleton.bone_count(),
            );

            let rest_model = onto
                .skeleton
                .forward_kinematics(&onto.skeleton.rest_pose)
                .unwrap();
            let model = onto.skeleton.forward_kinematics(&out).unwrap();
            let deltas = model_deltas(&onto.skeleton, &out);
            for moved in &case.moved {
                let index = bone(&onto.skeleton, moved.bone);
                let position = model_position(&model, index);
                let expected = model_position(&rest_model, index) + moved.by;
                assert!(
                    position.abs_diff_eq(expected, 1e-5),
                    "{}: {} at {position:?}, not {expected:?}",
                    case.name,
                    moved.bone
                );
                if let Some(turned) = moved.turned {
                    assert!(
                        deltas[index].abs_diff_eq(turned, 1e-5)
                            || deltas[index].abs_diff_eq(-turned, 1e-5),
                        "{}: {} turned {:?}, not {turned:?}",
                        case.name,
                        moved.bone,
                        deltas[index]
                    );
                }
            }
        }
    }

    struct LengthCase {
        name: &'static str,
        source: usize,
        out: usize,
        error: RetargetError,
    }

    #[test]
    fn a_pose_of_the_wrong_length_is_refused_and_nothing_is_written() {
        let source_count = source_skeleton().bone_count();
        let target_count = target_skeleton().bone_count();
        let cases = [
            LengthCase {
                name: "a short source pose",
                source: source_count - 1,
                out: target_count,
                error: RetargetError::PoseLength {
                    side: RetargetSide::Source,
                    expected: source_count,
                    actual: source_count - 1,
                },
            },
            LengthCase {
                name: "a short output",
                source: source_count,
                out: target_count - 1,
                error: RetargetError::PoseLength {
                    side: RetargetSide::Target,
                    expected: target_count,
                    actual: target_count - 1,
                },
            },
        ];

        for case in cases {
            let mut out = vec![JointPose::IDENTITY; case.out];

            let result = source_to_target()
                .retarget_into(&source_skeleton().rest_pose[..case.source], &mut out);

            assert_eq!(result, Err(case.error), "{}", case.name);
            assert_eq!(out, vec![JointPose::IDENTITY; case.out], "{}", case.name);
        }
    }

    struct PairRefusal {
        name: &'static str,
        source: ResolvedRig,
        source_skeleton: Skeleton,
        target: ResolvedRig,
        target_skeleton: Skeleton,
        side: Option<RetargetSide>,
    }

    fn pair_refusal(name: &'static str, side: RetargetSide) -> PairRefusal {
        PairRefusal {
            name,
            source: source_rig(),
            source_skeleton: source_skeleton(),
            target: target_rig(),
            target_skeleton: target_skeleton(),
            side: Some(side),
        }
    }

    fn feet_on(rig: RetargetRig, skeleton: &Skeleton) -> ResolvedRig {
        with_feet(rig).resolve(skeleton, None).unwrap()
    }

    #[test]
    fn a_pose_retarget_refuses_rigs_that_do_not_fit_their_skeletons_or_share_no_chain() {
        let mut root_past_the_last_bone = target_rig();
        root_past_the_last_bone.root = target_skeleton().bone_count();
        let mut empty_chain = source_rig();
        empty_chain.chains[0].bones.clear();
        let mut goal_tip_past_the_last_bone =
            feet_on(goal_target_rig_asset(), &goal_target_skeleton());
        goal_tip_past_the_last_bone.goals[0].tip = goal_target_skeleton().bone_count();
        let mut goal_mid_off_the_tip = feet_on(goal_source_rig_asset(), &goal_source_skeleton());
        goal_mid_off_the_tip.goals[0].mid = goal_mid_off_the_tip.goals[0].root;
        let goal_rigs = |name, side| PairRefusal {
            source: feet_on(goal_source_rig_asset(), &goal_source_skeleton()),
            source_skeleton: goal_source_skeleton(),
            target: feet_on(goal_target_rig_asset(), &goal_target_skeleton()),
            target_skeleton: goal_target_skeleton(),
            ..pair_refusal(name, side)
        };
        let cases = [
            PairRefusal {
                source: RetargetRig::new()
                    .with_root("hips")
                    .with_chain(RetargetChain::new("fin_l", "fin_l", "fin_l"))
                    .resolve(&source_skeleton(), None)
                    .unwrap(),
                side: None,
                ..pair_refusal("rigs sharing no chain name", RetargetSide::Source)
            },
            PairRefusal {
                source_skeleton: target_skeleton(),
                ..pair_refusal(
                    "a source rig beside the target skeleton",
                    RetargetSide::Source,
                )
            },
            PairRefusal {
                target_skeleton: source_skeleton(),
                ..pair_refusal(
                    "a target rig beside the source skeleton",
                    RetargetSide::Target,
                )
            },
            PairRefusal {
                target: root_past_the_last_bone,
                ..pair_refusal("a root past the last bone", RetargetSide::Target)
            },
            PairRefusal {
                source: empty_chain,
                ..pair_refusal("an empty chain", RetargetSide::Source)
            },
            PairRefusal {
                target: goal_tip_past_the_last_bone,
                ..goal_rigs("a goal tip past the last bone", RetargetSide::Target)
            },
            PairRefusal {
                source: goal_mid_off_the_tip,
                ..goal_rigs(
                    "a goal mid that is not the tip's parent",
                    RetargetSide::Source,
                )
            },
        ];

        for case in cases {
            let error = PoseRetarget::new(
                &case.source,
                &case.source_skeleton,
                &case.target,
                &case.target_skeleton,
            )
            .unwrap_err();

            let expected = match case.side {
                Some(side) => RetargetError::RigNotForSkeleton { side },
                None => RetargetError::NoSharedChain,
            };
            assert_eq!(error, expected, "{}", case.name);
        }
    }

    fn goal_source_skeleton() -> Skeleton {
        goal_source_skeleton_turned(&source_rotation)
    }

    fn goal_source_skeleton_turned(rotation: &dyn Fn(usize) -> Quat) -> Skeleton {
        let bones = [
            fixture_bone("hips", None, rotation(0), Vec3::new(0.0, 1.0, 0.0)),
            fixture_bone("spine", Some("hips"), rotation(1), Vec3::new(0.0, 1.2, 0.0)),
            fixture_bone(
                "thigh_l",
                Some("hips"),
                rotation(2),
                Vec3::new(0.1, 0.95, 0.0),
            ),
            fixture_bone(
                "shin_l",
                Some("thigh_l"),
                rotation(3),
                Vec3::new(0.1, 0.5, 0.08),
            ),
            fixture_bone(
                "foot_l",
                Some("shin_l"),
                rotation(4),
                Vec3::new(0.1, 0.05, 0.0),
            ),
            fixture_bone(
                "toe_l",
                Some("foot_l"),
                rotation(5),
                Vec3::new(0.1, 0.0, 0.12),
            ),
            fixture_bone(
                "thigh_r",
                Some("hips"),
                rotation(6),
                Vec3::new(-0.1, 0.95, 0.0),
            ),
            fixture_bone(
                "shin_r",
                Some("thigh_r"),
                rotation(7),
                Vec3::new(-0.1, 0.5, 0.08),
            ),
            fixture_bone(
                "foot_r",
                Some("shin_r"),
                rotation(8),
                Vec3::new(-0.1, 0.05, 0.0),
            ),
        ];
        fixture_skeleton(&bones, 3000)
    }

    fn goal_target_skeleton() -> Skeleton {
        goal_target_skeleton_turned(&target_rotation)
    }

    fn goal_target_skeleton_turned(rotation: &dyn Fn(usize) -> Quat) -> Skeleton {
        let bones = [
            fixture_bone("pelvis", None, rotation(1), Vec3::new(0.0, 0.5, 0.0)),
            fixture_bone(
                "spine_01",
                Some("pelvis"),
                rotation(2),
                Vec3::new(0.0, 0.6, 0.0),
            ),
            fixture_bone(
                "thigh_l",
                Some("pelvis"),
                rotation(3),
                Vec3::new(0.06, 0.48, 0.0),
            ),
            fixture_bone(
                "calf_l",
                Some("thigh_l"),
                rotation(4),
                Vec3::new(0.06, 0.25, 0.05),
            ),
            fixture_bone(
                "foot_l",
                Some("calf_l"),
                rotation(5),
                Vec3::new(0.06, 0.02, 0.0),
            ),
            fixture_bone(
                "toe_l",
                Some("foot_l"),
                rotation(6),
                Vec3::new(0.06, 0.0, 0.07),
            ),
            fixture_bone(
                "thigh_r",
                Some("pelvis"),
                rotation(7),
                Vec3::new(-0.06, 0.48, 0.0),
            ),
            fixture_bone(
                "calf_r",
                Some("thigh_r"),
                rotation(8),
                Vec3::new(-0.06, 0.25, 0.05),
            ),
            fixture_bone(
                "foot_r",
                Some("calf_r"),
                rotation(9),
                Vec3::new(-0.06, 0.02, 0.0),
            ),
        ];
        fixture_skeleton(&bones, 4000)
    }

    fn goal_source_rig_asset() -> RetargetRig {
        RetargetRig::new()
            .with_root("hips")
            .with_chain(RetargetChain::new("spine", "spine", "spine"))
            .with_chain(RetargetChain::new("leg_l", "thigh_l", "foot_l"))
            .with_chain(RetargetChain::new("toe_l", "toe_l", "toe_l"))
            .with_chain(RetargetChain::new("leg_r", "thigh_r", "foot_r"))
    }

    fn goal_target_rig_asset() -> RetargetRig {
        RetargetRig::new()
            .with_root("pelvis")
            .with_chain(RetargetChain::new("spine", "spine_01", "spine_01"))
            .with_chain(RetargetChain::new("leg_l", "thigh_l", "foot_l"))
            .with_chain(RetargetChain::new("toe_l", "toe_l", "toe_l"))
            .with_chain(RetargetChain::new("leg_r", "thigh_r", "foot_r"))
    }

    fn with_feet(rig: RetargetRig) -> RetargetRig {
        rig.with_goal(RetargetGoal::new("foot_l", "foot_l"))
            .with_goal(RetargetGoal::new("foot_r", "foot_r"))
    }

    fn goal_retarget(source: RetargetRig, target: RetargetRig) -> PoseRetarget {
        let source_skeleton = goal_source_skeleton();
        let target_skeleton = goal_target_skeleton();
        PoseRetarget::new(
            &source.resolve(&source_skeleton, None).unwrap(),
            &source_skeleton,
            &target.resolve(&target_skeleton, None).unwrap(),
            &target_skeleton,
        )
        .unwrap()
    }

    fn feet_retarget() -> PoseRetarget {
        goal_retarget(
            with_feet(goal_source_rig_asset()),
            with_feet(goal_target_rig_asset()),
        )
    }

    fn plain_retarget() -> PoseRetarget {
        goal_retarget(goal_source_rig_asset(), goal_target_rig_asset())
    }

    fn reach(skeleton: &Skeleton, deltas: &mut [Quat], bones: [&str; 3], shift: Vec3, goal: Vec3) {
        let rest_model = skeleton.forward_kinematics(&skeleton.rest_pose).unwrap();
        let [root, mid, tip] = bones.map(|name| bone(skeleton, name));
        let at = |bone: usize| model_position(&rest_model, bone) + shift;
        let solution = solve_two_bone(
            &TwoBoneChain {
                root: at(root),
                mid: at(mid),
                tip: at(tip),
            },
            goal,
            None,
            0.0,
        );
        deltas[root] = solution.root;
        deltas[mid] = solution.mid;
    }

    const CROUCH: f32 = 0.1;

    fn crouch_deltas(skeleton: &Skeleton) -> Vec<Quat> {
        let rest_model = skeleton.forward_kinematics(&skeleton.rest_pose).unwrap();
        let mut deltas = vec![Quat::IDENTITY; skeleton.bone_count()];
        for bones in [
            ["thigh_l", "shin_l", "foot_l"],
            ["thigh_r", "shin_r", "foot_r"],
        ] {
            let foot = model_position(&rest_model, bone(skeleton, bones[2]));
            reach(skeleton, &mut deltas, bones, Vec3::NEG_Y * CROUCH, foot);
        }
        deltas
    }

    fn crouched(skeleton: &Skeleton, deltas: &[Quat]) -> Vec<JointPose> {
        let mut pose = posed(skeleton, deltas);
        pose[bone(skeleton, "hips")].translation.y -= CROUCH;
        pose
    }

    fn crouched_source() -> Vec<JointPose> {
        let skeleton = goal_source_skeleton();
        crouched(&skeleton, &crouch_deltas(&skeleton))
    }

    struct GoalSide<'a> {
        skeleton: &'a Skeleton,
        pose: &'a [JointPose],
        root: &'a str,
        tip: &'a str,
    }

    impl GoalSide<'_> {
        fn tip_from_root(&self, pose: &[JointPose]) -> Vec3 {
            let model = self.skeleton.forward_kinematics(pose).unwrap();
            model_position(&model, bone(self.skeleton, self.tip))
                - model_position(&model, bone(self.skeleton, self.root))
        }
    }

    fn full_goal(source: &GoalSide, target: &GoalSide, scale: f32) -> Vec3 {
        let offset = target.tip_from_root(&target.skeleton.rest_pose)
            - source.tip_from_root(&source.skeleton.rest_pose) * scale;
        let target_model = target.skeleton.forward_kinematics(target.pose).unwrap();
        model_position(&target_model, bone(target.skeleton, target.root))
            + source.tip_from_root(source.pose) * scale
            + offset
    }

    fn assert_near(actual: Vec3, expected: Vec3, tolerance: f32) {
        assert!(
            actual.abs_diff_eq(expected, tolerance),
            "{actual:?} != {expected:?}"
        );
    }

    proptest! {
        #[test]
        fn a_source_turned_about_up_turns_the_solved_target_limb_with_it(angle in -3.1f32..3.1) {
            let source_skeleton = goal_source_skeleton();
            let target = goal_target_skeleton();
            let hips = bone(&source_skeleton, "hips");
            let turn = Quat::from_rotation_y(angle);
            let source = crouched_source();
            let mut turned = source.clone();
            turned[hips].rotation = turn * turned[hips].rotation;
            turned[hips].translation = turn * turned[hips].translation;

            let out = retarget(&mut feet_retarget(), &source, target.bone_count());
            let turned_out = retarget(&mut feet_retarget(), &turned, target.bone_count());

            let side = |pose: &[JointPose]| {
                GoalSide {
                    skeleton: &target,
                    pose,
                    root: "pelvis",
                    tip: "foot_l",
                }
                .tip_from_root(pose)
            };
            assert_near(side(&turned_out), turn * side(&out), 1e-4);
        }

        #[test]
        fn a_goal_weight_counts_as_clamped_to_0_and_1_and_nan_as_0(
            weight in prop_oneof![Just(f32::NAN), -2.0f32..3.0],
        ) {
            let count = goal_target_skeleton().bone_count();
            let source = crouched_source();
            let weighted = |weight: f32| {
                let mut pose_retarget = feet_retarget();
                for goal in ["foot_l", "foot_r"] {
                    pose_retarget
                        .set_goal(&RetargetGoalSettings::new(goal).with_weight(weight))
                        .unwrap();
                }
                retarget(&mut pose_retarget, &source, count)
            };
            let clamped = if weight.is_nan() { 0.0 } else { weight.clamp(0.0, 1.0) };

            prop_assert_eq!(weighted(weight), weighted(clamped));
            if clamped == 0.0 {
                prop_assert_eq!(weighted(weight), retarget(&mut plain_retarget(), &source, count));
            }
        }
    }

    #[test]
    fn a_source_ankle_at_its_reference_height_puts_the_target_ankle_at_its_own() {
        let target = goal_target_skeleton();
        let source = crouched_source();
        let foot = bone(&target, "foot_l");
        let reference =
            model_position(&target.forward_kinematics(&target.rest_pose).unwrap(), foot);

        let fk = retarget(&mut plain_retarget(), &source, target.bone_count());
        let solved = retarget(&mut feet_retarget(), &source, target.bone_count());

        let fk_foot = model_position(&target.forward_kinematics(&fk).unwrap(), foot);
        assert!(
            (fk_foot.y - reference.y).abs() > 1e-3,
            "the FK ankle {fk_foot:?} is already at {reference:?}"
        );
        assert_near(
            model_position(&target.forward_kinematics(&solved).unwrap(), foot),
            reference,
            1e-4,
        );
    }

    #[test]
    fn a_source_tip_moved_from_its_reference_moves_the_target_tip_by_the_scaled_distance() {
        let source_skeleton = goal_source_skeleton();
        let target = goal_target_skeleton();
        let moved = Vec3::new(0.03, 0.0, 0.05);
        let source_rest = source_skeleton
            .forward_kinematics(&source_skeleton.rest_pose)
            .unwrap();
        let mut deltas = vec![Quat::IDENTITY; source_skeleton.bone_count()];
        reach(
            &source_skeleton,
            &mut deltas,
            ["thigh_l", "shin_l", "foot_l"],
            Vec3::ZERO,
            model_position(&source_rest, bone(&source_skeleton, "foot_l")) + moved,
        );
        let source = posed(&source_skeleton, &deltas);
        let mut pose_retarget = feet_retarget();

        let out = retarget(&mut pose_retarget, &source, target.bone_count());

        let foot = bone(&target, "foot_l");
        let pelvis = bone(&target, "pelvis");
        let rest = target.forward_kinematics(&target.rest_pose).unwrap();
        let model = target.forward_kinematics(&out).unwrap();
        assert_near(
            model_position(&model, foot) - model_position(&model, pelvis),
            model_position(&rest, foot) - model_position(&rest, pelvis)
                + moved * pose_retarget.scale(),
            1e-4,
        );
    }

    #[test]
    fn a_goal_with_softness_eases_its_reach() {
        let count = goal_target_skeleton().bone_count();
        let source = crouched_source();
        let mut soft = feet_retarget();
        soft.set_goal(&RetargetGoalSettings::new("foot_l").with_softness(1.0))
            .unwrap();

        assert_ne!(
            retarget(&mut soft, &source, count),
            retarget(&mut feet_retarget(), &source, count)
        );
    }

    #[test]
    fn a_goal_at_half_weight_moves_the_tip_halfway_from_the_fk_tip_to_its_goal() {
        let source_skeleton = goal_source_skeleton();
        let target = goal_target_skeleton();
        let source = crouched_source();
        let mut pose_retarget = feet_retarget();
        pose_retarget
            .set_goal(&RetargetGoalSettings::new("foot_l").with_weight(0.5))
            .unwrap();

        let fk = retarget(&mut plain_retarget(), &source, target.bone_count());
        let out = retarget(&mut pose_retarget, &source, target.bone_count());

        let foot = bone(&target, "foot_l");
        let full = full_goal(
            &GoalSide {
                skeleton: &source_skeleton,
                pose: &source,
                root: "hips",
                tip: "foot_l",
            },
            &GoalSide {
                skeleton: &target,
                pose: &out,
                root: "pelvis",
                tip: "foot_l",
            },
            pose_retarget.scale(),
        );
        let fk_foot = model_position(&target.forward_kinematics(&fk).unwrap(), foot);
        assert!(fk_foot.distance(full) > 1e-3, "{fk_foot:?} is at {full:?}");
        assert_near(
            model_position(&target.forward_kinematics(&out).unwrap(), foot),
            fk_foot.lerp(full, 0.5),
            1e-4,
        );
    }

    #[test]
    fn the_tip_keeps_its_fk_rotation_and_carries_the_bones_below_it() {
        let target = goal_target_skeleton();
        let source = crouched_source();

        let fk = retarget(&mut plain_retarget(), &source, target.bone_count());
        let solved = retarget(&mut feet_retarget(), &source, target.bone_count());

        let foot = bone(&target, "foot_l");
        let toe = bone(&target, "toe_l");
        let fk_rotations = model_rotations(&target, &fk);
        let solved_rotations = model_rotations(&target, &solved);
        assert_quat(solved_rotations[foot], fk_rotations[foot]);
        assert_quat(solved_rotations[toe], fk_rotations[toe]);
        let fk_model = target.forward_kinematics(&fk).unwrap();
        let solved_model = target.forward_kinematics(&solved).unwrap();
        assert!(
            model_position(&solved_model, foot).distance(model_position(&fk_model, foot)) > 1e-3
        );
        assert_near(
            model_position(&solved_model, toe) - model_position(&solved_model, foot),
            model_position(&fk_model, toe) - model_position(&fk_model, foot),
            1e-5,
        );
    }

    #[test]
    fn a_goal_out_of_reach_straightens_the_limb_toward_it() {
        let from = goal_target_skeleton();
        let onto = goal_source_skeleton();
        let from_rest = from.forward_kinematics(&from.rest_pose).unwrap();
        let [thigh, calf, foot] = ["thigh_l", "calf_l", "foot_l"].map(|name| bone(&from, name));
        let mut deltas = vec![Quat::IDENTITY; from.bone_count()];
        for [upper, lower] in [[thigh, calf], [calf, foot]] {
            deltas[upper] = Quat::from_rotation_arc(
                (model_position(&from_rest, lower) - model_position(&from_rest, upper)).normalize(),
                Vec3::NEG_Y,
            );
        }
        let source = posed(&from, &deltas);
        let mut pose_retarget = PoseRetarget::new(
            &with_feet(goal_target_rig_asset())
                .resolve(&from, None)
                .unwrap(),
            &from,
            &with_feet(goal_source_rig_asset())
                .resolve(&onto, None)
                .unwrap(),
            &onto,
        )
        .unwrap();

        let out = retarget(&mut pose_retarget, &source, onto.bone_count());

        assert!(
            out.iter()
                .all(|pose| pose.rotation.is_finite() && pose.translation.is_finite())
        );
        let [thigh, shin, foot] = ["thigh_l", "shin_l", "foot_l"].map(|name| bone(&onto, name));
        let rest = onto.forward_kinematics(&onto.rest_pose).unwrap();
        let model = onto.forward_kinematics(&out).unwrap();
        let length = |model: &[Mat4], from: usize, to: usize| {
            model_position(model, to).distance(model_position(model, from))
        };
        let upper = length(&rest, thigh, shin);
        let lower = length(&rest, shin, foot);
        assert!((length(&model, thigh, shin) - upper).abs() < 1e-4);
        assert!((length(&model, shin, foot) - lower).abs() < 1e-4);
        assert!((length(&model, thigh, foot) - (upper + lower)).abs() < 1e-4);
        let full = full_goal(
            &GoalSide {
                skeleton: &from,
                pose: &source,
                root: "pelvis",
                tip: "foot_l",
            },
            &GoalSide {
                skeleton: &onto,
                pose: &out,
                root: "hips",
                tip: "foot_l",
            },
            pose_retarget.scale(),
        );
        let thigh_position = model_position(&model, thigh);
        assert!(full.distance(thigh_position) > upper + lower + 1e-3);
        assert_near(
            (model_position(&model, foot) - thigh_position).normalize(),
            (full - thigh_position).normalize(),
            1e-3,
        );
    }

    #[test]
    fn a_goal_not_on_both_rigs_is_not_solved_and_its_settings_are_refused() {
        let count = goal_target_skeleton().bone_count();
        let source = crouched_source();
        let mut one_sided =
            goal_retarget(goal_source_rig_asset(), with_feet(goal_target_rig_asset()));
        assert_eq!(
            retarget(&mut one_sided, &source, count),
            retarget(&mut plain_retarget(), &source, count)
        );
        let one_foot = || {
            goal_retarget(
                goal_source_rig_asset().with_goal(RetargetGoal::new("foot_l", "foot_l")),
                with_feet(goal_target_rig_asset()),
            )
        };
        let mut pose_retarget = one_foot();

        for goal in ["foot_r", "hand_l"] {
            assert_eq!(
                pose_retarget.set_goal(&RetargetGoalSettings::new(goal).with_weight(0.0)),
                Err(RetargetError::UnknownGoal {
                    goal: Name::new(goal),
                })
            );
        }

        assert_eq!(
            retarget(&mut pose_retarget, &source, count),
            retarget(&mut one_foot(), &source, count)
        );
    }

    #[test]
    fn goals_solve_in_order_and_a_later_goal_sees_an_earlier_ones_result() {
        let source_skeleton = goal_source_skeleton();
        let target = goal_target_skeleton();
        let mut deltas = crouch_deltas(&source_skeleton);
        deltas[bone(&source_skeleton, "foot_l")] = about_x(25.0);
        let source = crouched(&source_skeleton, &deltas);
        let toe_goal = RetargetGoal::new("toe_l", "toe_l");
        let mut pose_retarget = goal_retarget(
            with_feet(goal_source_rig_asset()).with_goal(toe_goal.clone()),
            with_feet(goal_target_rig_asset()).with_goal(toe_goal),
        );

        let feet_only = retarget(&mut feet_retarget(), &source, target.bone_count());
        let out = retarget(&mut pose_retarget, &source, target.bone_count());

        let toe = bone(&target, "toe_l");
        let full = full_goal(
            &GoalSide {
                skeleton: &source_skeleton,
                pose: &source,
                root: "hips",
                tip: "toe_l",
            },
            &GoalSide {
                skeleton: &target,
                pose: &out,
                root: "pelvis",
                tip: "toe_l",
            },
            pose_retarget.scale(),
        );
        let feet_only_toe = model_position(&target.forward_kinematics(&feet_only).unwrap(), toe);
        assert!(
            feet_only_toe.distance(full) > 1e-3,
            "{feet_only_toe:?} is at {full:?}"
        );
        assert_near(
            model_position(&target.forward_kinematics(&out).unwrap(), toe),
            full,
            1e-4,
        );
    }

    struct Warning {
        thread: ThreadId,
        message: String,
    }

    static WARNINGS: Mutex<Vec<Warning>> = Mutex::new(Vec::new());

    struct WarningCapture;

    impl log::Log for WarningCapture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record) {
            if record.level() == log::Level::Warn
                && record.target() == "concerto_animation::retarget"
            {
                WARNINGS.lock().unwrap().push(Warning {
                    thread: std::thread::current().id(),
                    message: record.args().to_string(),
                });
            }
        }

        fn flush(&self) {}
    }

    fn warning_messages() -> Vec<String> {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            log::set_logger(&WarningCapture).unwrap();
            log::set_max_level(log::LevelFilter::Warn);
        });
        let me = std::thread::current().id();
        WARNINGS
            .lock()
            .unwrap()
            .iter()
            .filter(|warning| warning.thread == me)
            .map(|warning| warning.message.clone())
            .collect()
    }

    fn warnings() -> usize {
        warning_messages().len()
    }

    const SOURCE_SKELETON: &str = "retarget_node_source_skeleton";
    const TARGET_SKELETON: &str = "retarget_node_target_skeleton";
    const SOURCE_RIG: &str = "retarget_node_source_rig";
    const TARGET_RIG: &str = "retarget_node_target_rig";
    const HELD_CLIP: &str = "retarget_node_held_clip";
    const INNER_GRAPH: &str = "retarget_node_inner_graph";
    const OUTER_GRAPH: &str = "retarget_node_outer_graph";
    const INNER_CLIP: &str = "retarget_node_inner_clip";
    const NESTED_GRAPH: &str = "retarget_node_nested_graph";

    fn handle<A: Asset>(path: &str) -> AssetHandle<A> {
        AssetHandle::weak(AssetId::from_path(path))
    }

    fn held_pose() -> Vec<JointPose> {
        let skeleton = source_skeleton();
        let deltas: Vec<Quat> = (0..skeleton.bone_count())
            .map(|bone| {
                Quat::from_euler(
                    glam::EulerRot::XYZ,
                    0.1 * bone as f32,
                    -0.05 * bone as f32,
                    0.15,
                )
            })
            .collect();
        posed(&skeleton, &deltas)
    }

    fn held_clip() -> AnimationClip {
        let mut clip = AnimationClip::default();
        for (id, joint) in source_skeleton().bone_ids.iter().zip(held_pose()) {
            clip.add_channel(
                *id,
                AnimationChannel::new(
                    vec![0.0, 1.0],
                    AnimationChanelOutput::Rotation(vec![joint.rotation; 2]),
                ),
            );
        }
        clip
    }

    fn node_target_rig() -> RetargetRig {
        target_rig_asset().with_skeleton(handle(TARGET_SKELETON))
    }

    fn retargeted_held_pose() -> Vec<JointPose> {
        retarget(
            &mut source_to_target(),
            &held_pose(),
            target_skeleton().bone_count(),
        )
    }

    fn evaluate_inputs<'a>(
        context: &'a AnimationGraphContext<'a>,
    ) -> &'a SystemInputData<'a, 'a, <RetargetNode as AnimationNode>::EvaluateInput> {
        context
            .inputs
            .get::<<RetargetNode as AnimationNode>::EvaluateInput>()
            .unwrap()
    }

    fn assert_poses(actual: &[JointPose], expected: &[JointPose]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert_pose(actual, expected);
        }
    }

    struct Played {
        walks: Walks,
        player: AnimationPlayer,
        player_skeleton: Skeleton,
    }

    impl Played {
        fn new(player_skeleton: Skeleton) -> Self {
            Self::registering(player_skeleton, |_| {})
        }

        fn registering(player_skeleton: Skeleton, register: impl FnOnce(&mut World)) -> Self {
            Self::built(player_skeleton, nested_retarget_node(), register)
        }

        fn with_node(player_skeleton: Skeleton, node: RetargetNode) -> Self {
            Self::built(player_skeleton, node, |_| {})
        }

        fn built(
            player_skeleton: Skeleton,
            node: RetargetNode,
            register: impl FnOnce(&mut World),
        ) -> Self {
            let mut walks = Walks::with(AssetStore::new(), AssetStore::new(), register);
            walks
                .skeletons_mut()
                .insert(AssetId::from_path(SOURCE_SKELETON), source_skeleton());
            walks
                .skeletons_mut()
                .insert(AssetId::from_path(TARGET_SKELETON), target_skeleton());
            walks
                .clips_mut()
                .insert(AssetId::from_path(HELD_CLIP), held_clip());
            let outer = AnimationGraph::from_node(node);
            walks
                .graphs_mut()
                .insert(AssetId::from_path(OUTER_GRAPH), made_live(outer));
            let mut player = AnimationPlayer::new(player_skeleton.bone_count());
            walks.initialize(&mut player, handle(OUTER_GRAPH));
            Self {
                walks,
                player,
                player_skeleton,
            }
        }

        fn loaded(player_skeleton: Skeleton, target: RetargetRig) -> Self {
            let mut played = Self::new(player_skeleton);
            played.load_rigs(target);
            played.load_inner_graph();
            played
        }

        fn playing(source: RetargetRig, clip: AnimationClip, inner: AnimationClipNode) -> Self {
            let mut played = Self::new(target_skeleton());
            played
                .walks
                .clips_mut()
                .insert(AssetId::from_path(INNER_CLIP), clip);
            played.load_rig_pair(source, node_target_rig());
            played.load_inner(inner);
            played
        }

        fn load_rigs(&mut self, target: RetargetRig) {
            self.load_rig_pair(node_source_rig(), target);
        }

        fn load_rig_pair(&mut self, source: RetargetRig, target: RetargetRig) {
            self.walks
                .rigs_mut()
                .insert(AssetId::from_path(SOURCE_RIG), source);
            self.walks
                .rigs_mut()
                .insert(AssetId::from_path(TARGET_RIG), target);
        }

        fn load_inner_graph(&mut self) {
            self.load_inner(AnimationClipNode::new(handle(HELD_CLIP)));
        }

        fn load_inner(&mut self, inner: AnimationClipNode) {
            self.insert_inner(made_live(AnimationGraph::from_node(inner)));
        }

        fn insert_inner(&mut self, graph: AnimationGraph) {
            self.walks
                .graphs_mut()
                .insert(AssetId::from_path(INNER_GRAPH), graph);
        }

        fn visits<N: AnimationNode>(&mut self) -> usize {
            let mut visits = 0;
            self.player
                .for_each_state_mut::<N>(self.walks.graphs(), |_, _, _| visits += 1);
            visits
        }

        fn shared_visits<N: AnimationNode>(&self) -> usize {
            let mut visits = 0;
            self.player
                .for_each_state::<N>(self.walks.graphs(), |_, _, _| visits += 1);
            visits
        }

        fn advance(&mut self) {
            self.advance_by(0.1);
        }

        fn advance_by(&mut self, delta_time: f32) {
            self.walks
                .advance_with(&mut self.player, delta_time, &self.player_skeleton);
        }

        fn pose(&mut self) -> Vec<JointPose> {
            self.frame(None).joints().to_vec()
        }

        fn frame(&mut self, root: Option<RootBone>) -> AnimationFrame {
            let skeleton = &self.player_skeleton;
            let walks = &self.walks;
            let mut frame = None;
            self.player.for_each_state_mut::<RetargetNode>(
                walks.graphs(),
                |node, state, blackboard| {
                    let mut pool = FramePool::new(skeleton.bone_count());
                    pool.set_rest(&skeleton.rest_pose);
                    let mut output = pool.acquire();
                    walks.evaluate_with(blackboard, root.clone(), Some(skeleton), |context| {
                        node.evaluate(
                            state,
                            evaluate_inputs(context),
                            context,
                            &skeleton.bone_ids,
                            &[],
                            &mut pool,
                            &mut output,
                        )
                    });
                    frame = Some(output);
                },
            );
            frame.expect("the player plays a retarget node")
        }

        fn is_finished(&mut self) -> bool {
            let mut finished = None;
            self.player
                .for_each_state_mut::<RetargetNode>(self.walks.graphs(), |node, state, _| {
                    finished = Some(node.is_finished(state))
                });
            finished.expect("the player plays a retarget node")
        }

        fn reset(&mut self) {
            self.player
                .for_each_state_mut::<RetargetNode>(self.walks.graphs(), |node, state, _| {
                    node.reset(state)
                });
        }
    }

    fn node_source_rig() -> RetargetRig {
        source_rig_asset().with_skeleton(handle(SOURCE_SKELETON))
    }

    fn inner_clip() -> AnimationClipNode {
        AnimationClipNode::new(handle(INNER_CLIP))
    }

    fn turning_clip() -> AnimationClip {
        let skeleton = source_skeleton();
        let spine = bone(&skeleton, "spine_2");
        let rest = skeleton.rest_pose[spine].rotation;
        let mut clip = AnimationClip::default();
        clip.add_channel(
            skeleton.bone_ids[spine],
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Rotation(vec![rest, about_x(60.0) * rest]),
            ),
        );
        clip
    }

    fn retargeted_turning_pose(time: f32) -> Vec<JointPose> {
        retarget(
            &mut source_to_target(),
            &turning_clip().sample_pose(&source_skeleton(), time),
            target_skeleton().bone_count(),
        )
    }

    #[test]
    fn a_retarget_node_holds_the_rest_pose_until_its_rigs_and_inner_graph_load() {
        let rest = target_skeleton().rest_pose.to_vec();
        let mut played = Played::new(target_skeleton());

        played.advance();
        assert_poses(&played.pose(), &rest);

        played.load_rigs(node_target_rig());
        played.advance();
        assert_poses(&played.pose(), &rest);

        played.load_inner_graph();
        played.advance();
        assert_poses(&played.pose(), &retargeted_held_pose());
    }

    struct NodeFailure {
        name: &'static str,
        played: Played,
        pose: Vec<JointPose>,
        tolerance: f32,
        finished: bool,
        warning: &'static str,
    }

    fn holding_a_retarget_node(through_a_state_machine: bool) -> Played {
        let mut played = Played::new(target_skeleton());
        played.load_rigs(node_target_rig());
        let nested = made_live(AnimationGraph::from_node(nested_retarget_node()));
        if through_a_state_machine {
            played
                .walks
                .graphs_mut()
                .insert(AssetId::from_path(NESTED_GRAPH), nested);
            played.insert_inner(made_live(AnimationGraph::from_node(
                AnimationStateMachine::from_initial_state(
                    name!("nested"),
                    handle(NESTED_GRAPH),
                    |_| {},
                )
                .build(),
            )));
        } else {
            played.insert_inner(nested);
        }
        played
    }

    fn holding_a_created_once_node(through_a_state_machine: bool) -> Played {
        let mut played = played_with_create_only();
        if through_a_state_machine {
            played
                .walks
                .graphs_mut()
                .insert(AssetId::from_path(NESTED_GRAPH), created_once_graph());
            played.insert_inner(made_live(AnimationGraph::from_node(
                AnimationStateMachine::from_initial_state(
                    name!("created"),
                    handle(NESTED_GRAPH),
                    |_| {},
                )
                .build(),
            )));
        } else {
            played.insert_inner(created_once_graph());
        }
        played
    }

    fn with_the_player_skeleton_changed() -> Played {
        let mut played = Played::loaded(target_skeleton(), node_target_rig());
        played.advance();
        assert_poses(&played.pose(), &retargeted_held_pose());
        played.player_skeleton = renamed_target_skeleton();
        played
    }

    fn failure(name: &'static str, played: Played, warning: &'static str) -> NodeFailure {
        NodeFailure {
            name,
            played,
            pose: node_target_reference(),
            tolerance: 1e-5,
            finished: true,
            warning,
        }
    }

    #[test]
    fn a_retarget_node_that_cannot_honour_its_assets_warns_once_naming_why_and_holds_a_pose() {
        let no_shared_chain = RetargetRig::new()
            .with_skeleton(handle(TARGET_SKELETON))
            .with_motion_root("root")
            .with_root("pelvis")
            .with_chain(RetargetChain::new("tail", "tail", "tail"))
            .with_correction(BoneCorrection::new("spine_02", about_x(30.0)));
        let cases = [
            NodeFailure {
                pose: source_skeleton().rest_pose.to_vec(),
                ..failure(
                    "a target rig on another skeleton than the player",
                    Played::loaded(source_skeleton(), node_target_rig()),
                    "not the player's skeleton",
                )
            },
            NodeFailure {
                pose: no_shared_chain
                    .resolve(&target_skeleton(), None)
                    .unwrap()
                    .reference_local,
                ..failure(
                    "rigs sharing no chain",
                    Played::loaded(target_skeleton(), no_shared_chain),
                    "share no chain",
                )
            },
            failure(
                "an inner graph holding a retarget node",
                holding_a_retarget_node(false),
                "holds a retarget node",
            ),
            failure(
                "an inner state machine state holding a retarget node",
                holding_a_retarget_node(true),
                "holds a retarget node",
            ),
            failure(
                "an inner node whose create input is not an update input",
                holding_a_created_once_node(false),
                "CreatedOnce",
            ),
            failure(
                "an inner state machine state node whose create input is not an update input",
                holding_a_created_once_node(true),
                "CreatedOnce",
            ),
            NodeFailure {
                pose: target_skeleton().rest_pose.to_vec(),
                ..failure(
                    "the player skeleton changed while playing",
                    with_the_player_skeleton_changed(),
                    "the player's skeleton changed",
                )
            },
            NodeFailure {
                pose: feet_held_pose(),
                tolerance: NEAR_FULL_REACH,
                finished: false,
                ..failure(
                    "settings naming an unknown goal",
                    played_with_goal_settings(RetargetGoalSettings::new("hand_l").with_weight(0.0)),
                    "`hand_l`",
                )
            },
        ];

        for mut case in cases {
            let before = warnings();

            for _ in 0..3 {
                case.played.advance();
                assert_poses_within(case.name, &case.played.pose(), &case.pose, case.tolerance);
            }

            let messages = warning_messages().split_off(before);
            assert_eq!(messages.len(), 1, "{}: {messages:?}", case.name);
            assert!(
                messages[0].contains(case.warning),
                "{}: {}",
                case.name,
                messages[0]
            );
            assert_eq!(case.played.is_finished(), case.finished, "{}", case.name);
        }
    }

    #[test]
    fn an_outer_root_nets_the_inner_pose_against_the_source_reference_and_scales_its_root_motion() {
        let source_skeleton = source_skeleton();
        let hips = bone(&source_skeleton, "hips");
        let hips_rest = source_skeleton.rest_pose[hips].rotation;
        let source = node_source_rig().with_correction(BoneCorrection::new(
            "hips",
            hips_rest.inverse() * Quat::from_rotation_y(0.5) * hips_rest,
        ));
        let mut moving = AnimationClip::default();
        moving.add_channel(
            source_skeleton.bone_ids[hips],
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::Translation(vec![
                    Vec3::new(0.0, 1.0, 0.0),
                    Vec3::new(2.0, 1.0, 4.0),
                ]),
            ),
        );
        let mut played = Played::playing(source, moving, inner_clip());
        let target = target_skeleton();
        let outer_root = RootBone {
            index: 0,
            rest: target.rest_pose[0].clone(),
            up: Vec3::Y,
        };

        played.advance();
        let frame = played.frame(Some(outer_root));

        assert_poses(frame.joints(), &target.rest_pose);
        assert_vec3(
            frame.root_motion().translation,
            Vec3::new(0.2, 0.0, 0.4) * source_to_target().scale(),
        );
        assert_quat(frame.root_motion().rotation, Quat::IDENTITY);
    }

    #[test]
    fn a_clip_event_fired_in_the_inner_graph_reaches_the_output_frame() {
        let mut clip = held_clip();
        clip.add_event(
            0.05,
            AnimationEvent {
                name: Name::new("step"),
            },
        );
        let mut played = Played::playing(node_source_rig(), clip, inner_clip());

        played.advance();

        assert_eq!(
            played.frame(None).events(),
            [FiredEvent {
                clip: AssetId::from_path(INNER_CLIP),
                key: 0,
                time: 0.05,
                weight: 1.0,
                mirrored: false,
            }]
        );
    }

    #[test]
    fn a_retarget_node_finishes_with_its_inner_one_shot_clip_and_not_while_pending_or_after_a_reset()
     {
        let mut played = Played::new(target_skeleton());
        played.load_rigs(node_target_rig());

        played.advance();
        assert!(!played.is_finished());

        played.load_inner(
            AnimationClipNode::new(handle(HELD_CLIP)).with_play_mode(AnimationPlayMode::PlayOnce),
        );
        played.advance_by(0.5);
        assert!(!played.is_finished());

        played.advance_by(0.75);
        assert!(played.is_finished());

        played.reset();
        assert!(!played.is_finished());
    }

    #[test]
    fn reset_restarts_the_inner_graph_clip() {
        let mut played = Played::playing(node_source_rig(), turning_clip(), inner_clip());
        played.advance();
        played.advance_by(0.4);

        played.reset();
        played.advance();

        assert_poses(&played.pose(), &retargeted_turning_pose(0.1));
    }

    #[test]
    fn the_inner_clip_reads_its_play_rate_from_the_player_blackboard() {
        let mut played = Played::playing(
            node_source_rig(),
            turning_clip(),
            inner_clip().with_play_rate_param(Name::new("speed")),
        );
        played.player.set_float_param(Name::new("speed"), 3.0);

        played.advance();

        assert_poses(&played.pose(), &retargeted_turning_pose(0.1 * 3.0));
    }

    fn nested_retarget_node() -> RetargetNode {
        RetargetNode::new(handle(INNER_GRAPH), handle(SOURCE_RIG), handle(TARGET_RIG))
    }

    const NEAR_FULL_REACH: f32 = 1e-3;

    fn assert_poses_within(
        case: &str,
        actual: &[JointPose],
        expected: &[JointPose],
        tolerance: f32,
    ) {
        assert_eq!(actual.len(), expected.len(), "{case}");
        for (actual, expected) in actual.iter().zip(expected.iter()) {
            assert!(
                actual.rotation.abs_diff_eq(expected.rotation, tolerance)
                    || actual.rotation.abs_diff_eq(-expected.rotation, tolerance),
                "{case}: {:?} != {:?}",
                actual.rotation,
                expected.rotation
            );
            assert_vec3(actual.translation, expected.translation);
        }
    }

    fn node_with_feet(rig: RetargetRig) -> RetargetRig {
        rig.with_goal(RetargetGoal::new("foot_l", "foot_l"))
    }

    fn played_with_goal_settings(settings: RetargetGoalSettings) -> Played {
        let mut played = Played::with_node(
            target_skeleton(),
            nested_retarget_node().with_goal(settings),
        );
        played.load_rig_pair(
            node_with_feet(node_source_rig()),
            node_with_feet(node_target_rig()),
        );
        played.load_inner_graph();
        played
    }

    fn feet_held_pose() -> Vec<JointPose> {
        retarget(
            &mut PoseRetarget::new(
                &node_with_feet(source_rig_asset())
                    .resolve(&source_skeleton(), None)
                    .unwrap(),
                &source_skeleton(),
                &node_with_feet(target_rig_asset())
                    .resolve(&target_skeleton(), None)
                    .unwrap(),
                &target_skeleton(),
            )
            .unwrap(),
            &held_pose(),
            target_skeleton().bone_count(),
        )
    }

    #[test]
    fn a_retarget_node_applies_its_last_setting_for_each_goal() {
        assert_ne!(feet_held_pose(), retargeted_held_pose());
        let cases = [
            [
                RetargetGoalSettings::new("foot_l"),
                RetargetGoalSettings::new("foot_l").with_weight(0.0),
            ],
            [
                RetargetGoalSettings::new("foot_l").with_weight(0.0),
                RetargetGoalSettings::new("foot_l"),
            ],
        ];

        for [first, last] in cases {
            let at_weight_0 = last.weight == 0.0;
            let mut played = Played::with_node(
                target_skeleton(),
                nested_retarget_node().with_goal(first).with_goal(last),
            );
            played.load_rig_pair(
                node_with_feet(node_source_rig()),
                node_with_feet(node_target_rig()),
            );
            played.load_inner_graph();

            played.advance();

            if at_weight_0 {
                assert_poses(&played.pose(), &retargeted_held_pose());
            } else {
                assert_poses_within(
                    "a last setting at full weight",
                    &played.pose(),
                    &feet_held_pose(),
                    NEAR_FULL_REACH,
                );
            }
        }
    }

    fn node_target_reference() -> Vec<JointPose> {
        node_target_rig()
            .resolve(&target_skeleton(), None)
            .unwrap()
            .reference_local
    }

    #[test]
    fn for_each_state_and_for_each_state_mut_visit_a_node_inside_the_inner_graph() {
        let mut played = Played::playing(node_source_rig(), turning_clip(), inner_clip());
        assert_eq!(played.visits::<AnimationClipNode>(), 0);
        assert_eq!(played.shared_visits::<AnimationClipNode>(), 0);

        played.advance();

        assert_eq!(played.visits::<AnimationClipNode>(), 1);
        assert_eq!(played.shared_visits::<AnimationClipNode>(), 1);
    }

    #[derive(Resource)]
    struct CreateOnly;

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct CreatedOnce {}

    impl AnimationNode for CreatedOnce {
        type State = ();
        type CreateInput = Res<'static, CreateOnly>;
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &Res<CreateOnly>, _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            _: &mut AnimationFrame,
        ) {
        }

        fn reset(&self, _: &mut ()) {}
    }

    fn played_with_create_only() -> Played {
        let mut played = Played::registering(target_skeleton(), |world| {
            register_input::<CreateNodeInputs, Res<'static, CreateOnly>>(world);
            world.insert_resource(CreateOnly);
        });
        played.load_rigs(node_target_rig());
        played
    }

    fn created_once_graph() -> AnimationGraph {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<CreatedOnce>();
        let mut graph = AnimationGraph::from_node(CreatedOnce {});
        graph.make_live(&registry, None).unwrap();
        graph
    }

    fn renamed_target_skeleton() -> Skeleton {
        let mut skeleton = target_skeleton();
        skeleton.bone_ids = (0..skeleton.bone_count() as u128)
            .map(|bone| Uuid::from_u128(3000 + bone))
            .collect();
        skeleton
    }

    fn short_target_skeleton() -> Skeleton {
        let skeleton = target_skeleton();
        let count = 3;
        Skeleton::new(
            skeleton.inverse_bindposes[..count].to_vec(),
            skeleton.parents[..count].to_vec(),
            skeleton.names[..count].to_vec(),
            skeleton.rest_pose[..count].to_vec(),
            skeleton.bone_ids[..count].to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn an_output_shorter_than_the_target_skeleton_keeps_its_rest_pose_and_passes_events_with_one_warning()
     {
        let mut clip = turning_clip();
        clip.add_event(
            0.05,
            AnimationEvent {
                name: Name::new("step"),
            },
        );
        let mut played = Played::playing(node_source_rig(), clip, inner_clip());
        played.advance();
        let rest = short_target_skeleton().rest_pose.to_vec();
        let before = warnings();

        played.player_skeleton = short_target_skeleton();
        for _ in 0..3 {
            let frame = played.frame(None);
            assert_poses(frame.joints(), &rest);
            assert_vec3(frame.root_motion().translation, Vec3::ZERO);
            assert_quat(frame.root_motion().rotation, Quat::IDENTITY);
            assert_eq!(frame.events().len(), 1);
            assert_eq!(frame.events()[0].clip, AssetId::from_path(INNER_CLIP));
        }

        assert_eq!(warnings() - before, 1);
    }

    const REFERENCE_CLIP: &str = "retarget_node_reference_clip";

    fn source_rest_clip() -> AnimationClip {
        let skeleton = source_skeleton();
        let mut clip = AnimationClip::default();
        for (id, joint) in skeleton.bone_ids.iter().zip(skeleton.rest_pose.iter()) {
            clip.add_channel(
                *id,
                AnimationChannel::new(
                    vec![0.0, 1.0],
                    AnimationChanelOutput::Rotation(vec![joint.rotation; 2]),
                ),
            );
        }
        clip
    }

    #[test]
    fn a_retarget_node_stays_pending_until_its_reference_clip_loads() {
        let rest = target_skeleton().rest_pose.to_vec();
        let mut played = Played::new(target_skeleton());
        played.load_rig_pair(
            node_source_rig().with_reference(ReferencePose::ClipFrame {
                clip: handle(REFERENCE_CLIP),
                time: 0.0,
            }),
            node_target_rig(),
        );
        played.load_inner_graph();
        let before = warnings();

        for _ in 0..2 {
            played.advance();
            assert_poses(&played.pose(), &rest);
            assert!(!played.is_finished());
        }
        assert_eq!(warnings(), before);

        played
            .walks
            .clips_mut()
            .insert(AssetId::from_path(REFERENCE_CLIP), source_rest_clip());
        played.advance();

        assert_poses(&played.pose(), &retargeted_held_pose());
    }
}
