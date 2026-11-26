use std::fmt;

use concerto_mesh::Skeleton;
use concerto_name::Name;
use concerto_physics::ragdoll::{
    BodyPose, BuiltRagdoll, JointSettings, MotorSettings, PartSettings, PartShape,
    RagdollBuildError, RagdollSettings,
};
use glam::Mat4;

use crate::asset::{Ragdoll, RagdollShape, Segment, Side};

/// Why a ragdoll asset does not fit its skeleton or cannot be built.
#[derive(Clone, Debug, PartialEq)]
pub enum RagdollError {
    NoParts,
    /// A part names a bone the skeleton does not have.
    UnknownBone {
        bone: Name,
    },
    /// Two parts are on the same bone.
    DuplicateBone {
        bone: Name,
    },
    /// A part is under no other part, and neither is the first root.
    ///
    /// A ragdoll is one tree.
    SecondRoot {
        root: Name,
        bone: Name,
    },
    /// The physics backend refused the parts.
    Build(RagdollBuildError),
}

impl fmt::Display for RagdollError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoParts => f.write_str("the ragdoll has no parts; add one for the pelvis first"),
            Self::UnknownBone { bone } => write!(
                f,
                "a part is on bone `{bone}`, which the skeleton does not have; pick one of the skeleton's bones"
            ),
            Self::DuplicateBone { bone } => {
                write!(f, "two parts are on bone `{bone}`; remove one")
            }
            Self::SecondRoot { root, bone } => write!(
                f,
                "the part on `{bone}` is not under the root part on `{root}`; a ragdoll is one tree, so add a part on a bone above both or remove one"
            ),
            Self::Build(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RagdollError {}

/// The parts of one arm and one leg, each when the ragdoll has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limbs {
    pub upper_arm: Option<usize>,
    pub lower_arm: Option<usize>,
    pub hand: Option<usize>,
    pub thigh: Option<usize>,
    pub shin: Option<usize>,
    pub foot: Option<usize>,
}

/// Which part is which, from the segment and side each part of the asset names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landmarks {
    pub pelvis: Option<usize>,
    /// The spine parts, from the lowest up.
    pub spine: Vec<usize>,
    /// The highest spine part, or the pelvis when the ragdoll has no spine part.
    pub chest: Option<usize>,
    pub head: Option<usize>,
    pub left: Limbs,
    pub right: Limbs,
}

impl Landmarks {
    /// Both feet the ragdoll has.
    pub fn feet(&self) -> impl Iterator<Item = usize> {
        [self.left.foot, self.right.foot].into_iter().flatten()
    }

    /// The limbs of `side`; the centre has none.
    pub fn limbs(&self, side: Side) -> Option<&Limbs> {
        match side {
            Side::Left => Some(&self.left),
            Side::Right => Some(&self.right),
            Side::Centre => None,
        }
    }
}

/// A ragdoll asset fitted to its skeleton: the physics settings, built, and which bone each part follows.
#[derive(Clone)]
pub struct ResolvedRagdoll {
    built: BuiltRagdoll,
    bones: Vec<usize>,
    segments: Vec<Segment>,
    sides: Vec<Side>,
    landmarks: Landmarks,
    names: Vec<Name>,
    followed: Vec<Option<usize>>,
    rest: Vec<BodyPose>,
}

impl ResolvedRagdoll {
    /// The recipe ragdolls are created from.
    pub fn built(&self) -> &BuiltRagdoll {
        &self.built
    }

    /// The physics settings: one part per ragdoll part, parents first.
    pub fn settings(&self) -> &RagdollSettings {
        self.built.settings()
    }

    pub fn part_count(&self) -> usize {
        self.bones.len()
    }

    /// Which part is which.
    pub fn landmarks(&self) -> &Landmarks {
        &self.landmarks
    }

    /// What part of the body `part` is.
    pub fn segment(&self, part: usize) -> Segment {
        self.segments.get(part).copied().unwrap_or_default()
    }

    /// The side of the body `part` is on.
    pub fn side(&self, part: usize) -> Side {
        self.sides.get(part).copied().unwrap_or_default()
    }

    /// The index in the skeleton of the bone each part follows, in part order.
    pub fn bones(&self) -> &[usize] {
        &self.bones
    }

    /// The name of the bone `part` follows.
    pub fn bone_name(&self, part: usize) -> Option<Name> {
        self.names.get(part).copied()
    }

    /// The part on the bone called `bone`.
    pub fn part_of(&self, bone: &str) -> Option<usize> {
        self.names.iter().position(|name| &**name == bone)
    }

    /// For each bone of the skeleton, the part it moves with.
    ///
    /// That is the bone's own part, or that of its nearest ancestor that has
    /// one; `None` above the root part.
    pub fn followed(&self) -> &[Option<usize>] {
        &self.followed
    }

    /// Every bone's model-space rest pose, in skeleton order.
    pub fn rest(&self) -> &[BodyPose] {
        &self.rest
    }
}

impl Ragdoll {
    /// Fits the asset to `skeleton` and builds the physics recipe.
    ///
    /// Parts are ordered parents first, each joined to the part of its
    /// nearest ancestor bone that has one.
    ///
    /// # Errors
    ///
    /// Returns [`RagdollError::NoParts`] when the asset has no parts,
    /// [`RagdollError::UnknownBone`] when a part names a bone `skeleton` does
    /// not have, [`RagdollError::DuplicateBone`] when two parts are on one
    /// bone, [`RagdollError::SecondRoot`] when a part is under no other part
    /// and is not the root, and [`RagdollError::Build`] when the physics
    /// settings are refused.
    pub fn resolve(&self, skeleton: &Skeleton) -> Result<ResolvedRagdoll, RagdollError> {
        if self.parts.is_empty() {
            return Err(RagdollError::NoParts);
        }
        let mut sources = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            let bone = skeleton
                .bone_index(part.bone.0)
                .ok_or(RagdollError::UnknownBone { bone: part.bone.0 })?;
            sources.push(Source { bone, part });
        }
        sources.sort_by_key(|source| source.bone);
        if let Some(pair) = sources.windows(2).find(|pair| pair[0].bone == pair[1].bone) {
            return Err(RagdollError::DuplicateBone {
                bone: pair[0].part.bone.0,
            });
        }

        let rest = model_rest(skeleton);
        let mut followed: Vec<Option<usize>> = vec![None; skeleton.bone_count()];
        let mut next = 0;
        for bone in 0..skeleton.bone_count() {
            followed[bone] = if sources.get(next).is_some_and(|source| source.bone == bone) {
                next += 1;
                Some(next - 1)
            } else {
                skeleton.parents[bone].and_then(|parent| followed[parent as usize])
            };
        }
        let mut settings = RagdollSettings::new();
        for (index, source) in sources.iter().enumerate() {
            let parent = skeleton.parents[source.bone].and_then(|parent| followed[parent as usize]);
            if index > 0 && parent.is_none() {
                return Err(RagdollError::SecondRoot {
                    root: sources[0].part.bone.0,
                    bone: source.part.bone.0,
                });
            }

            let pose = rest[source.bone];
            let part = source.part;
            let joint = JointSettings::new(
                pose.rotation * part.joint.twist_axis,
                pose.rotation * part.joint.plane_axis,
            )
            .with_swing(part.joint.swing_normal, part.joint.swing_plane)
            .with_twist(part.joint.twist_min, part.joint.twist_max)
            .with_max_friction_torque(part.joint.friction)
            .with_motor(MotorSettings {
                frequency: part.joint.motor.frequency,
                damping: part.joint.motor.damping,
                torque_limit: part.joint.motor.torque_limit,
            });
            let mut built = PartSettings::new(pose, shape(part.shape))
                .with_shape_offset(BodyPose::new(part.offset, part.rotation.normalize()))
                .with_mass(part.mass)
                .with_friction(if part.segment == Segment::Foot {
                    SOLE_FRICTION
                } else {
                    BODY_FRICTION
                });
            if let Some(parent) = parent {
                built = built.with_parent(parent, joint);
            }
            settings = settings.with_part(built);
        }

        let mut landmarks = Landmarks::default();
        for (index, source) in sources.iter().enumerate() {
            let part = source.part;
            let limbs = match part.side {
                Side::Left => Some(&mut landmarks.left),
                Side::Right => Some(&mut landmarks.right),
                Side::Centre => None,
            };
            match (part.segment, limbs) {
                (Segment::Pelvis, _) => landmarks.pelvis = Some(index),
                (Segment::Spine, _) => landmarks.spine.push(index),
                (Segment::Head, _) => landmarks.head = Some(index),
                (Segment::UpperArm, Some(limbs)) => limbs.upper_arm = Some(index),
                (Segment::LowerArm, Some(limbs)) => limbs.lower_arm = Some(index),
                (Segment::Hand, Some(limbs)) => limbs.hand = Some(index),
                (Segment::Thigh, Some(limbs)) => limbs.thigh = Some(index),
                (Segment::Shin, Some(limbs)) => limbs.shin = Some(index),
                (Segment::Foot, Some(limbs)) => limbs.foot = Some(index),
                _ => {}
            }
        }
        landmarks.chest = landmarks.spine.last().copied().or(landmarks.pelvis);

        Ok(ResolvedRagdoll {
            built: settings.build().map_err(RagdollError::Build)?,
            bones: sources.iter().map(|source| source.bone).collect(),
            segments: sources.iter().map(|source| source.part.segment).collect(),
            sides: sources.iter().map(|source| source.part.side).collect(),
            landmarks,
            names: sources.iter().map(|source| source.part.bone.0).collect(),
            followed,
            rest,
        })
    }
}

const SOLE_FRICTION: f32 = 4.0;
const BODY_FRICTION: f32 = 0.5;

struct Source<'a> {
    bone: usize,
    part: &'a crate::asset::RagdollPart,
}

fn shape(shape: RagdollShape) -> PartShape {
    match shape {
        RagdollShape::Capsule {
            half_height,
            radius,
        } => PartShape::Capsule {
            half_height,
            radius,
        },
        RagdollShape::Box { half_extents } => PartShape::Box { half_extents },
        RagdollShape::Sphere { radius } => PartShape::Sphere { radius },
    }
}

fn model_rest(skeleton: &Skeleton) -> Vec<BodyPose> {
    let mut matrices: Vec<Mat4> = Vec::with_capacity(skeleton.bone_count());
    for (bone, local) in skeleton.rest_pose.iter().enumerate() {
        let local = local.to_mat4();
        matrices.push(match skeleton.parents[bone] {
            Some(parent) => matrices[parent as usize] * local,
            None => local,
        });
    }
    matrices
        .iter()
        .map(|matrix| {
            let (_, rotation, translation) = matrix.to_scale_rotation_translation();
            BodyPose::new(translation, rotation.normalize())
        })
        .collect()
}
