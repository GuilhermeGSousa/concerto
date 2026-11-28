use std::fmt;

use concerto_animation::retarget::{ResolvedChain, ResolvedRig};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::{Mesh, Skeleton};
use concerto_name::Name;
use glam::{Mat4, Quat, Vec3};

use crate::asset::{
    PartStrength, Ragdoll, RagdollJoint, RagdollPart, RagdollShape, Segment, Side, StrengthProfile,
};

/// The names a retarget rig gives the chains a humanoid ragdoll is generated from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HumanoidChains {
    /// The spine's chain, above the pelvis.
    pub spine: Name,
    /// The neck's chain, whose last bone is the head.
    pub neck: Name,
    /// The left arm's chain, from upper arm to hand.
    pub left_arm: Name,
    /// The right arm's chain, from upper arm to hand.
    pub right_arm: Name,
    /// The left leg's chain, from thigh to foot.
    pub left_leg: Name,
    /// The right leg's chain, from thigh to foot.
    pub right_leg: Name,
}

impl Default for HumanoidChains {
    fn default() -> Self {
        Self {
            spine: "spine".into(),
            neck: "neck".into(),
            left_arm: "arm_l".into(),
            right_arm: "arm_r".into(),
            left_leg: "leg_l".into(),
            right_leg: "leg_r".into(),
        }
    }
}

/// What a humanoid ragdoll is generated with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HumanoidSettings {
    /// The character's mass in kg, split over the parts by body-segment fractions.
    pub total_mass: f32,
    /// The chain names to read the rig by.
    pub chains: HumanoidChains,
}

impl Default for HumanoidSettings {
    fn default() -> Self {
        Self {
            total_mass: 75.0,
            chains: HumanoidChains::default(),
        }
    }
}

impl HumanoidSettings {
    /// Sets the character's mass in kg.
    pub fn with_total_mass(mut self, total_mass: f32) -> Self {
        self.total_mass = total_mass;
        self
    }

    /// Sets the chain names to read the rig by.
    pub fn with_chains(mut self, chains: HumanoidChains) -> Self {
        self.chains = chains;
        self
    }
}

/// Why a humanoid ragdoll could not be generated from a rig.
#[derive(Clone, Debug, PartialEq)]
pub enum GenerateError {
    /// The rig has no chain of this name.
    MissingChain {
        /// The chain's name.
        chain: Name,
    },
    /// The chain has fewer bones than the limb needs.
    ShortChain {
        /// The chain's name.
        chain: Name,
        /// How many bones the chain has.
        bones: usize,
        /// How many bones the limb needs.
        needs: usize,
    },
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingChain { chain } => write!(
                f,
                "the retarget rig has no chain called `{chain}`; add it to the rig, or name the rig's chain for it in the settings"
            ),
            Self::ShortChain {
                chain,
                bones,
                needs,
            } => write!(
                f,
                "chain `{chain}` has {bones} bones and a limb needs {needs}; start it higher or end it lower"
            ),
        }
    }
}

impl std::error::Error for GenerateError {}

const TORSO_ABOVE_PELVIS: f32 = 0.355;

/// The generated profile with the pelvis and legs kinematic and everything above powered: a body that takes a hit without needing balance.
pub const FLINCH: &str = "flinch";
/// The generated profile with every part powered: a body on its own feet.
pub const BRACED: &str = "braced";
/// The generated profile with every part limp.
pub const LIMP: &str = "limp";

impl Segment {
    /// The share of the body's mass in one segment of this kind; the spine's share is split over its parts.
    pub fn mass_fraction(self) -> f32 {
        match self {
            Segment::Pelvis => 0.142,
            Segment::Spine => TORSO_ABOVE_PELVIS,
            Segment::Head => 0.081,
            Segment::UpperArm => 0.028,
            Segment::LowerArm => 0.016,
            Segment::Hand => 0.006,
            Segment::Thigh => 0.1,
            Segment::Shin => 0.0465,
            Segment::Foot => 0.0145,
            Segment::Other => 0.0,
        }
    }

    /// The most torque the muscles holding a segment of this kind apply in the generated profiles, in N·m.
    pub fn muscle_torque(self) -> f32 {
        match self {
            Segment::Pelvis => 400.0,
            Segment::Spine => 400.0,
            Segment::Head => 80.0,
            Segment::UpperArm => 120.0,
            Segment::LowerArm => 70.0,
            Segment::Hand => 20.0,
            Segment::Thigh => 300.0,
            Segment::Shin => 250.0,
            Segment::Foot => 150.0,
            Segment::Other => 50.0,
        }
    }

    /// The stiffness of the muscles holding a segment of this kind in the generated profiles, in N·m/rad.
    ///
    /// It is enough that the segment and what hangs from it sag only a little
    /// under their own weight.
    pub fn muscle_stiffness(self) -> f32 {
        match self {
            Segment::Pelvis | Segment::Spine => 3000.0,
            Segment::Thigh => 2500.0,
            Segment::Shin => 1500.0,
            Segment::Foot => 600.0,
            Segment::Head => 300.0,
            Segment::UpperArm => 500.0,
            Segment::LowerArm => 200.0,
            Segment::Hand => 40.0,
            Segment::Other => 100.0,
        }
    }

    fn above_the_pelvis(self) -> bool {
        matches!(
            self,
            Segment::Spine | Segment::Head | Segment::UpperArm | Segment::LowerArm | Segment::Hand
        )
    }

    fn thickness(self) -> f32 {
        match self {
            Segment::Pelvis => 0.45,
            Segment::Spine => 0.8,
            Segment::Head => 0.5,
            Segment::UpperArm => 0.16,
            Segment::LowerArm => 0.13,
            Segment::Hand => 0.35,
            Segment::Thigh => 0.17,
            Segment::Shin => 0.12,
            Segment::Foot => 0.3,
            Segment::Other => 0.2,
        }
    }
}

struct Frame {
    up: Vec3,
    forward: Vec3,
    left: Vec3,
}

struct Planned {
    bone: usize,
    segment: Segment,
    side: Side,
    start: Vec3,
    end: Vec3,
    mass: f32,
    joint_model: JointModel,
    max_radius: f32,
}

enum JointModel {
    Ball {
        swing_normal: f32,
        swing_plane: f32,
        twist: f32,
    },
    Hinge {
        axis: Vec3,
        min: f32,
        max: f32,
        swing: f32,
    },
}

fn position(matrix: &Mat4) -> Vec3 {
    matrix.w_axis.truncate()
}

fn rotation(matrix: &Mat4) -> Quat {
    matrix.to_scale_rotation_translation().1.normalize()
}

fn chain(rig: &ResolvedRig, name: Name, needs: usize) -> Result<&ResolvedChain, GenerateError> {
    let chain = rig
        .chain(name)
        .ok_or(GenerateError::MissingChain { chain: name })?;
    if chain.bones.len() < needs {
        return Err(GenerateError::ShortChain {
            chain: name,
            bones: chain.bones.len(),
            needs,
        });
    }
    Ok(chain)
}

fn hinge_axis(bone_direction: Vec3, bend_towards: Vec3, fallback: Vec3) -> Vec3 {
    bone_direction
        .cross(bend_towards)
        .try_normalize()
        .unwrap_or(fallback)
}

fn part_owners(skeleton: &Skeleton, planned: &[Planned]) -> Vec<Option<usize>> {
    let mut owners: Vec<Option<usize>> = vec![None; skeleton.bone_count()];
    for bone in 0..skeleton.bone_count() {
        owners[bone] = planned
            .iter()
            .position(|plan| plan.bone == bone)
            .or_else(|| skeleton.parents[bone].and_then(|parent| owners[parent as usize]));
    }
    owners
}

fn skin_points(
    skeleton: &Skeleton,
    model: &[Mat4],
    meshes: &[&Mesh],
    owners: &[Option<usize>],
) -> Vec<Vec<Vec3>> {
    let parts = owners.iter().flatten().max().map_or(0, |last| last + 1);
    let mut points = vec![Vec::new(); parts.max(1)];
    for mesh in meshes {
        for vertex in &mesh.vertices {
            let mut dominant = 0;
            for slot in 1..vertex.bone_weights.len() {
                if vertex.bone_weights[slot] > vertex.bone_weights[dominant] {
                    dominant = slot;
                }
            }
            if vertex.bone_weights[dominant] <= 0.0 {
                continue;
            }
            let bone = vertex.bone_indices[dominant] as usize;
            let Some(Some(owner)) = owners.get(bone) else {
                continue;
            };
            let in_bone =
                skeleton.inverse_bindposes[bone].transform_point3(Vec3::from(vertex.pos_coords));
            points[*owner].push(model[bone].transform_point3(in_bone));
        }
    }
    points
}

const MIN_SKIN_POINTS: usize = 8;
const FRICTION_PER_KG: f32 = 0.5;

fn skinned_radius(points: &[Vec3], start: Vec3, end: Vec3) -> Option<f32> {
    let axis = end - start;
    let length_squared = axis.length_squared();
    if length_squared < 1e-8 {
        return None;
    }
    let mut total = 0.0;
    let mut count = 0;
    for point in points {
        let along = (*point - start).dot(axis) / length_squared;
        if !(0.0..=1.0).contains(&along) {
            continue;
        }
        total += (*point - (start + axis * along)).length();
        count += 1;
    }
    (count >= MIN_SKIN_POINTS).then(|| total / count as f32)
}

enum ShapeAxis {
    Along(Vec3),
    Forward(Vec3),
}

struct Sole {
    centre: Vec3,
    forward: Vec3,
    half_extents: Vec3,
}

const HEEL: f32 = 0.06;
const SOLE_UNDER_TOE: f32 = 0.02;

impl Sole {
    fn under(plan: &Planned, frame: &Frame, radius: f32) -> Option<Self> {
        let reach = plan.end - plan.start;
        let flat = reach - frame.up * reach.dot(frame.up);
        let forward = flat.try_normalize()?;
        let top = plan.start.dot(frame.up);
        let toe = top + reach.dot(frame.up) / 1.5;
        let bottom = top.min(toe) - SOLE_UNDER_TOE;
        let length = flat.length() + HEEL;
        let height = (top - bottom).max(0.02);
        Some(Self {
            centre: plan.start + forward * (length / 2.0 - HEEL)
                - frame.up * (top - (top + bottom) / 2.0),
            forward,
            half_extents: Vec3::new(radius.max(0.03), height / 2.0, length / 2.0),
        })
    }
}

fn part(
    skeleton: &Skeleton,
    model: &[Mat4],
    plan: &Planned,
    skin: &[Vec3],
    frame: &Frame,
) -> RagdollPart {
    let bone_position = position(&model[plan.bone]);
    let to_bone = rotation(&model[plan.bone]).inverse();
    let vector = plan.end - plan.start;
    let length = vector.length().max(0.02);
    let direction = vector.try_normalize().unwrap_or(frame.up);
    let radius = skinned_radius(skin, plan.start, plan.end)
        .unwrap_or(length * plan.segment.thickness())
        .clamp(0.02, plan.max_radius.max(0.02));
    let centre = (plan.start + plan.end) / 2.0;
    let shape = if plan.segment == Segment::Head {
        RagdollShape::Sphere {
            radius: radius.min(length / 2.0).max(0.05),
        }
    } else {
        RagdollShape::Capsule {
            half_height: (length / 2.0 - radius).max(0.005),
            radius: radius.min(length / 2.0).max(0.02),
        }
    };
    let sole = Sole::under(plan, frame, radius);
    let (shape, centre, direction_of_shape) = match (plan.segment, sole) {
        (Segment::Foot, Some(sole)) => (
            RagdollShape::Box {
                half_extents: sole.half_extents,
            },
            sole.centre,
            ShapeAxis::Forward(sole.forward),
        ),
        _ => (shape, centre, ShapeAxis::Along(direction)),
    };
    let joint = match plan.joint_model {
        JointModel::Ball {
            swing_normal,
            swing_plane,
            twist,
        } => {
            let plane = (frame.forward - direction * frame.forward.dot(direction))
                .try_normalize()
                .unwrap_or_else(|| direction.any_orthonormal_vector());
            RagdollJoint::new(to_bone * direction, to_bone * plane)
                .with_swing(swing_normal, swing_plane)
                .with_twist(-twist, twist)
        }
        JointModel::Hinge {
            axis,
            min,
            max,
            swing,
        } => {
            let plane = (direction - axis * direction.dot(axis))
                .try_normalize()
                .unwrap_or_else(|| axis.any_orthonormal_vector());
            RagdollJoint::new(to_bone * axis, to_bone * plane)
                .with_swing(swing, swing)
                .with_twist(min, max)
        }
    };
    RagdollPart::new(skeleton.names[plan.bone].as_ref(), shape)
        .with_offset(to_bone * (centre - bone_position))
        .with_rotation(
            (to_bone
                * match direction_of_shape {
                    ShapeAxis::Along(direction) => Quat::from_rotation_arc(Vec3::Y, direction),
                    ShapeAxis::Forward(forward) => Quat::from_rotation_arc(Vec3::Z, forward),
                })
            .normalize(),
        )
        .with_mass(plan.mass)
        .with_segment(plan.segment, plan.side)
        .with_joint(joint.with_friction((plan.mass * FRICTION_PER_KG).clamp(0.3, 5.0)))
}

impl Ragdoll {
    /// Generates a first-draft ragdoll for a humanoid.
    ///
    /// The ragdoll has a pelvis, up to three spine parts, a head, upper and lower
    /// arms, hands, thighs, shins and feet. Capsules run along each bone, as
    /// thick as the vertices of `meshes` skinned to it, or in proportion to the
    /// bone when there are none.
    ///
    /// # Errors
    ///
    /// Returns [`GenerateError::MissingChain`] when `rig` has no chain of a name
    /// in the settings' [`HumanoidChains`], and [`GenerateError::ShortChain`]
    /// when a chain has fewer bones than its limb needs.
    ///
    /// # Panics
    ///
    /// Panics if a chain of `rig` names a bone outside `rig`'s reference model, which a rig
    /// resolved against `skeleton` rules out.
    pub fn humanoid(
        skeleton_handle: AssetHandle<Skeleton>,
        skeleton: &Skeleton,
        rig: &ResolvedRig,
        meshes: &[&Mesh],
        settings: &HumanoidSettings,
    ) -> Result<Ragdoll, GenerateError> {
        let model = &rig.reference_model;
        let at = |bone: usize| position(&model[bone]);
        let chains = &settings.chains;
        let spine = chain(rig, chains.spine, 1)?;
        let neck = chain(rig, chains.neck, 1)?;
        let left_arm = chain(rig, chains.left_arm, 3)?;
        let right_arm = chain(rig, chains.right_arm, 3)?;
        let left_leg = chain(rig, chains.left_leg, 3)?;
        let right_leg = chain(rig, chains.right_leg, 3)?;

        let up = Vec3::Y;
        let hips = at(left_leg.bones[0]) - at(right_leg.bones[0]);
        let left = (hips - up * hips.dot(up))
            .try_normalize()
            .unwrap_or(Vec3::X);
        let frame = Frame {
            up,
            forward: left.cross(up).normalize(),
            left,
        };
        let hip_width = hips.length().max(0.05);
        let mass = settings.total_mass;
        let mut planned = Vec::new();

        let hip_centre = (at(left_leg.bones[0]) + at(right_leg.bones[0])) / 2.0;
        planned.push(Planned {
            bone: rig.root,
            segment: Segment::Pelvis,
            side: Side::Centre,
            start: hip_centre - frame.left * hip_width / 2.0,
            end: hip_centre + frame.left * hip_width / 2.0,
            mass: mass * Segment::Pelvis.mass_fraction(),
            joint_model: JointModel::Ball {
                swing_normal: 0.0,
                swing_plane: 0.0,
                twist: 0.0,
            },
            max_radius: f32::MAX,
        });

        let spine_bones: Vec<usize> = match spine.bones.len() {
            0..=3 => spine.bones.clone(),
            count => vec![
                spine.bones[0],
                spine.bones[count / 2],
                spine.bones[count - 1],
            ],
        };
        let head = *neck.bones.last().expect("the neck chain has a bone");
        let spine_top = if neck.bones.len() > 1 {
            at(neck.bones[0])
        } else {
            at(head)
        };
        for (index, bone) in spine_bones.iter().enumerate() {
            let end = spine_bones
                .get(index + 1)
                .map_or(spine_top, |next| at(*next));
            planned.push(Planned {
                bone: *bone,
                segment: Segment::Spine,
                side: Side::Centre,
                start: at(*bone),
                end,
                mass: mass * Segment::Spine.mass_fraction() / spine_bones.len() as f32,
                joint_model: JointModel::Ball {
                    swing_normal: 0.3,
                    swing_plane: 0.3,
                    twist: 0.3,
                },
                max_radius: f32::MAX,
            });
        }

        let below_head = neck
            .bones
            .iter()
            .rev()
            .nth(1)
            .copied()
            .unwrap_or(*spine_bones.last().expect("the spine chain has a bone"));
        let head_direction = (at(head) - at(below_head)).try_normalize().unwrap_or(up);
        let head_length = ((at(head) - at(below_head)).length() * 2.0).max(0.12);
        planned.push(Planned {
            bone: head,
            segment: Segment::Head,
            side: Side::Centre,
            start: at(head),
            end: at(head) + head_direction * head_length,
            mass: mass * Segment::Head.mass_fraction(),
            joint_model: JointModel::Ball {
                swing_normal: 0.6,
                swing_plane: 0.6,
                twist: 0.8,
            },
            max_radius: f32::MAX,
        });

        for (arm, side) in [left_arm, right_arm]
            .into_iter()
            .zip([Side::Left, Side::Right])
        {
            let count = arm.bones.len();
            let upper = arm.bones[count - 3];
            let lower = arm.bones[count - 2];
            let hand = arm.bones[count - 1];
            let upper_direction = (at(lower) - at(upper))
                .try_normalize()
                .unwrap_or(frame.left);
            let lower_vector = at(hand) - at(lower);
            let lower_direction = lower_vector.try_normalize().unwrap_or(upper_direction);
            planned.push(Planned {
                bone: upper,
                segment: Segment::UpperArm,
                side,
                start: at(upper),
                end: at(lower),
                mass: mass * Segment::UpperArm.mass_fraction(),
                joint_model: JointModel::Ball {
                    swing_normal: 1.4,
                    swing_plane: 1.4,
                    twist: 0.8,
                },
                max_radius: f32::MAX,
            });
            planned.push(Planned {
                bone: lower,
                segment: Segment::LowerArm,
                side,
                start: at(lower),
                end: at(hand),
                mass: mass * Segment::LowerArm.mass_fraction(),
                joint_model: JointModel::Hinge {
                    axis: hinge_axis(lower_direction, frame.forward, frame.up),
                    min: 0.0,
                    max: 2.4,
                    swing: 0.0,
                },
                max_radius: f32::MAX,
            });
            planned.push(Planned {
                bone: hand,
                segment: Segment::Hand,
                side,
                start: at(hand),
                end: at(hand) + lower_direction * lower_vector.length().max(0.05) * 0.4,
                mass: mass * Segment::Hand.mass_fraction(),
                joint_model: JointModel::Ball {
                    swing_normal: 0.8,
                    swing_plane: 0.5,
                    twist: 0.3,
                },
                max_radius: f32::MAX,
            });
        }

        for (leg, side) in [left_leg, right_leg]
            .into_iter()
            .zip([Side::Left, Side::Right])
        {
            let thigh = leg.bones[0];
            let shin = leg.bones[1];
            let foot = leg.bones[2];
            let shin_direction = (at(foot) - at(shin)).try_normalize().unwrap_or(-frame.up);
            let foot_end = match leg.bones.get(3) {
                Some(toe) => at(foot) + (at(*toe) - at(foot)) * 1.5,
                None => at(foot) + frame.forward * 0.18 - frame.up * 0.05,
            };
            planned.push(Planned {
                bone: thigh,
                segment: Segment::Thigh,
                side,
                start: at(thigh),
                end: at(shin),
                mass: mass * Segment::Thigh.mass_fraction(),
                joint_model: JointModel::Ball {
                    swing_normal: 1.0,
                    swing_plane: 0.8,
                    twist: 0.4,
                },
                max_radius: hip_width * 0.48,
            });
            planned.push(Planned {
                bone: shin,
                segment: Segment::Shin,
                side,
                start: at(shin),
                end: at(foot),
                mass: mass * Segment::Shin.mass_fraction(),
                joint_model: JointModel::Hinge {
                    axis: hinge_axis(shin_direction, -frame.forward, frame.left),
                    min: 0.0,
                    max: 2.4,
                    swing: 0.0,
                },
                max_radius: hip_width * 0.48,
            });
            planned.push(Planned {
                bone: foot,
                segment: Segment::Foot,
                side,
                start: at(foot),
                end: foot_end,
                mass: mass * Segment::Foot.mass_fraction(),
                joint_model: JointModel::Hinge {
                    axis: frame.left,
                    min: -0.6,
                    max: 0.6,
                    swing: 0.2,
                },
                max_radius: f32::MAX,
            });
        }

        let owners = part_owners(skeleton, &planned);
        let skin = skin_points(skeleton, model, meshes, &owners);
        let mut ragdoll = Ragdoll::new(skeleton_handle);
        for (index, plan) in planned.iter().enumerate() {
            ragdoll = ragdoll.with_part(part(skeleton, model, plan, &skin[index], &frame));
        }
        let mut flinch = StrengthProfile::new(FLINCH, PartStrength::Kinematic);
        let mut braced = StrengthProfile::new(BRACED, PartStrength::Kinematic);
        for plan in &planned {
            let bone = skeleton.names[plan.bone].as_ref();
            let muscle = PartStrength::powered(
                plan.segment.muscle_stiffness(),
                plan.segment.muscle_torque(),
            );
            braced = braced.with_part(bone, muscle);
            if plan.segment.above_the_pelvis() {
                flinch = flinch.with_part(bone, muscle);
            }
        }
        Ok(ragdoll
            .with_profile(flinch)
            .with_profile(braced)
            .with_profile(StrengthProfile::new(LIMP, PartStrength::Limp)))
    }
}
