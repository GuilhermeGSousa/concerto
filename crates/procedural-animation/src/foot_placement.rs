use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    ik::Hinge,
    node::{
        AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint, parent},
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};
use concerto_ecs::Res;
use concerto_foundation::math::Spring;
use concerto_mesh::{JointName, Skeleton};
use concerto_name::name;
use concerto_physics::{body::BodyId, physics_state::PhysicsState, query::QueryFilter};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Affine3A, Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::leg::{ResolvedLeg, bend_leg, ease, elapsed_after, horizontal, resolve_leg};

const MIN_TOE_DISTANCE: f32 = 0.01;
const MIN_NORMAL_LENGTH: f32 = 1.0e-3;
const MIN_WEIGHT: f32 = 1.0e-4;

/// Puts each leg's foot on the ground under it and lowers the pelvis for the lowest.
///
/// It passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct FootPlacementNode {
    /// The pelvis bone, lowered so the lowest foot can reach the ground.
    pub pelvis: JointName,
    /// The legs to place.
    pub legs: Vec<FootPlacementLeg>,
    /// How far above the character's floor, in metres, the rays start; the floor is the height of
    /// the model origin.
    pub max_step_up: f32,
    /// How far below the character's floor, in metres, the rays reach.
    pub max_step_down: f32,
    /// How far the pelvis may drop for the lowest foot, in metres; negative counts as 0.
    pub max_pelvis_drop: f32,
    /// The largest angle, in radians, a foot tilts to the ground; negative counts as 0.
    pub max_foot_angle: f32,
    /// The halflife, in seconds, the foot heights ease with.
    pub foot_halflife: f32,
    /// The halflife, in seconds, the pelvis drop eases with.
    pub pelvis_halflife: f32,
    /// The halflife, in seconds, the foot tilts ease with.
    pub rotation_halflife: f32,
    /// The distance below full leg extension, in metres, where reach starts to ease.
    pub softness: f32,
    /// The weight the placement is applied at, before the param scales it; clamped to 0 to 1.
    pub weight: f32,
    /// The `f32` blackboard param the weight is multiplied by, when it holds one.
    pub weight_param: Option<Param<f32>>,
    /// The halflife, in seconds, the weight eases with.
    pub weight_halflife: f32,
    /// The blackboard param holding the [`IgnoredBody`] the rays skip.
    pub ignore_param: Option<Param<IgnoredBody>>,
}

impl Default for FootPlacementNode {
    fn default() -> Self {
        Self {
            pelvis: JointName::default(),
            legs: Vec::new(),
            max_step_up: 0.5,
            max_step_down: 0.5,
            max_pelvis_drop: 0.4,
            max_foot_angle: 30.0_f32.to_radians(),
            foot_halflife: 0.05,
            pelvis_halflife: 0.1,
            rotation_halflife: 0.05,
            softness: 0.0,
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
            ignore_param: None,
        }
    }
}

impl FootPlacementNode {
    /// A node with the default settings and no legs, placing under the given pelvis bone.
    pub fn new(pelvis: impl Into<JointName>) -> Self {
        Self {
            pelvis: pelvis.into(),
            ..Default::default()
        }
    }

    /// Adds a leg to place.
    pub fn with_leg(mut self, leg: FootPlacementLeg) -> Self {
        self.legs.push(leg);
        self
    }

    /// Sets how far above the floor the rays start.
    pub fn with_max_step_up(mut self, max_step_up: f32) -> Self {
        self.max_step_up = max_step_up;
        self
    }

    /// Sets how far below the floor the rays reach.
    pub fn with_max_step_down(mut self, max_step_down: f32) -> Self {
        self.max_step_down = max_step_down;
        self
    }

    /// Sets how far the pelvis may drop for the lowest foot.
    pub fn with_max_pelvis_drop(mut self, max_pelvis_drop: f32) -> Self {
        self.max_pelvis_drop = max_pelvis_drop;
        self
    }

    /// Sets the largest angle, in radians, a foot tilts to the ground.
    pub fn with_max_foot_angle(mut self, max_foot_angle: f32) -> Self {
        self.max_foot_angle = max_foot_angle;
        self
    }

    /// Sets the halflife the foot heights ease with.
    pub fn with_foot_halflife(mut self, foot_halflife: f32) -> Self {
        self.foot_halflife = foot_halflife;
        self
    }

    /// Sets the halflife the pelvis drop eases with.
    pub fn with_pelvis_halflife(mut self, pelvis_halflife: f32) -> Self {
        self.pelvis_halflife = pelvis_halflife;
        self
    }

    /// Sets the halflife the foot tilts ease with.
    pub fn with_rotation_halflife(mut self, rotation_halflife: f32) -> Self {
        self.rotation_halflife = rotation_halflife;
        self
    }

    /// Sets the two-bone solve's softness near full extension.
    pub fn with_softness(mut self, softness: f32) -> Self {
        self.softness = softness;
        self
    }

    /// Sets the constant weight, multiplied by the weight param.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Sets the float param the weight is multiplied by.
    pub fn with_weight_param(mut self, weight_param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(weight_param.into());
        self
    }

    /// Sets the halflife the weight eases with.
    pub fn with_weight_halflife(mut self, weight_halflife: f32) -> Self {
        self.weight_halflife = weight_halflife;
        self
    }

    /// Sets the param holding the [`IgnoredBody`] the rays skip.
    pub fn with_ignore_param(mut self, ignore_param: impl Into<Param<IgnoredBody>>) -> Self {
        self.ignore_param = Some(ignore_param.into());
        self
    }
}

/// A leg of a [`FootPlacementNode`].
///
/// The foot bone names the chain: its parent is the knee and its grandparent
/// the hip. An optional toe bone adds a second ray, and `hinge` is the knee's
/// hinge axis.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct FootPlacementLeg {
    /// The foot bone, where the first ray is cast; its parent is the knee and its grandparent the
    /// hip.
    pub foot: JointName,
    /// The toe bone, where a second ray is cast, if any.
    pub toe: Option<JointName>,
    /// The knee's hinge axis.
    pub hinge: Hinge,
}

impl FootPlacementLeg {
    /// A leg on the given foot bone, without a toe.
    pub fn new(foot: impl Into<JointName>) -> Self {
        Self {
            foot: foot.into(),
            toe: None,
            hinge: Hinge::Off,
        }
    }

    /// Sets the toe bone the second ray is cast at.
    pub fn with_toe(mut self, toe: impl Into<JointName>) -> Self {
        self.toe = Some(toe.into());
        self
    }

    /// Sets the knee's hinge axis; see `Hinge`.
    pub fn with_hinge(mut self, hinge: Hinge) -> Self {
        self.hinge = hinge;
        self
    }
}

/// The body a foot placement node's rays skip, set on the blackboard by the game: the character's own capsule.
#[derive(Clone, Copy, Debug, Default, Reflect)]
pub struct IgnoredBody {
    /// The body to skip; `None` skips none.
    #[reflect(skip)]
    pub body: Option<BodyId>,
}

/// The springs a [`FootPlacementNode`] eases with, the time it has to ease over, and whether it has warned about legs it cannot resolve.
#[derive(Debug)]
pub struct FootPlacementState {
    warned: bool,
    elapsed: f32,
    started: bool,
    weight: Spring<f32>,
    pelvis: Spring<f32>,
    offsets: Vec<Spring<f32>>,
    tilts: Vec<Spring<Quat>>,
}

impl FootPlacementState {
    fn new(node: &FootPlacementNode) -> Self {
        Self {
            warned: false,
            elapsed: 0.0,
            started: false,
            weight: Spring::critically_damped_with_halflife(0.0, node.weight_halflife),
            pelvis: Spring::critically_damped_with_halflife(0.0, node.pelvis_halflife),
            offsets: Vec::new(),
            tilts: Vec::new(),
        }
    }
}

struct ResolvedLegs<'a> {
    skeleton: &'a Skeleton,
    pelvis: usize,
    legs: Vec<ResolvedLeg>,
}

#[derive(Clone, Copy)]
struct Ground {
    height: f32,
    normal: Option<Vec3>,
}

struct FootTarget {
    offset: f32,
    tilt: Quat,
}

struct PlacedLeg {
    foot: Vec3,
    target: FootTarget,
}

struct LegGoal {
    position: Vec3,
    tilt: Quat,
}

struct Caster<'a> {
    physics: Option<&'a PhysicsState>,
    filter: QueryFilter<'a>,
    floor: f32,
    up: f32,
    down: f32,
}

impl Caster<'_> {
    fn ground(&self, at: Vec3) -> Option<Ground> {
        let physics = self.physics?;
        let length = self.up + self.down;
        let origin = Vec3::new(at.x, self.floor + self.up, at.z);
        if length <= 0.0 || !length.is_finite() || !origin.is_finite() {
            return None;
        }
        let hit = physics.cast_ray(origin, Vec3::NEG_Y * length, self.filter)?;
        Some(Ground {
            height: hit.point.y - self.floor,
            normal: unit_normal(hit.normal),
        })
    }
}

fn unit_normal(normal: Vec3) -> Option<Vec3> {
    Some(normal)
        .filter(|normal| normal.is_finite() && normal.length() >= MIN_NORMAL_LENGTH)
        .map(Vec3::normalize)
}

impl FootPlacementNode {
    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedLegs<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let pelvis = bone_under_first(skeleton, self.pelvis)?;
        let mut legs = Vec::with_capacity(self.legs.len());
        for leg in &self.legs {
            let resolved = resolve_leg(skeleton, leg.foot, leg.toe, &leg.hinge)?;
            if !descends_from(skeleton, resolved.hip, pelvis) {
                return Err(format!(
                    "the hip of foot `{}` does not descend from pelvis `{}`",
                    &*leg.foot, &*self.pelvis
                ));
            }
            legs.push(resolved);
        }
        Ok(ResolvedLegs {
            skeleton,
            pelvis,
            legs,
        })
    }

    fn weight_target(&self, blackboard: &AnimationBlackboard) -> f32 {
        let scale = self
            .weight_param
            .and_then(|param| blackboard.read(&param).copied())
            .unwrap_or(1.0);
        let weight = self.weight * scale;
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

    fn max_foot_angle(&self) -> f32 {
        if self.max_foot_angle.is_nan() {
            0.0
        } else {
            self.max_foot_angle.max(0.0)
        }
    }

    fn place(
        &self,
        resolved: &ResolvedLegs<'_>,
        physics: Option<&PhysicsState>,
        blackboard: &AnimationBlackboard,
        model_to_world: Affine3A,
        joints: &[JointPose],
    ) -> Option<Vec<PlacedLeg>> {
        let ignored = self
            .ignore_param
            .and_then(|param| blackboard.read(&param))
            .and_then(|ignored| ignored.body);
        let caster = Caster {
            physics,
            filter: QueryFilter::ignoring(ignored.as_slice()),
            floor: model_to_world.translation.y,
            up: self.max_step_up,
            down: self.max_step_down,
        };
        let world = |bone| {
            model_joint(resolved.skeleton, joints, bone)
                .map(|joint| model_to_world.transform_point3(joint.transform.translation.into()))
        };
        resolved
            .legs
            .iter()
            .map(|leg| {
                let foot = world(leg.foot)?;
                let toe = match leg.toe {
                    Some(toe) => Some(world(toe)?),
                    None => None,
                };
                Some(PlacedLeg {
                    foot,
                    target: self.foot_target(&caster, foot, toe),
                })
            })
            .collect()
    }

    fn foot_target(&self, caster: &Caster<'_>, foot: Vec3, toe: Option<Vec3>) -> FootTarget {
        let max_angle = self.max_foot_angle();
        let toe = toe.filter(|toe| horizontal(*toe - foot).length() >= MIN_TOE_DISTANCE);
        let ankle_ground = caster.ground(foot);
        let toe_ground = toe.and_then(|toe| caster.ground(toe));
        let target = match (ankle_ground, toe, toe_ground) {
            (Some(ankle), Some(toe), Some(toe_ground)) => {
                pitched(foot, toe, ankle, toe_ground, max_angle)
            }
            (Some(ankle), _, _) => FootTarget {
                offset: ankle.height,
                tilt: ankle.normal.map_or(Quat::IDENTITY, |normal| {
                    Quat::from_rotation_arc(Vec3::Y, normal)
                }),
            },
            (None, _, Some(toe_ground)) => FootTarget {
                offset: toe_ground.height,
                tilt: Quat::IDENTITY,
            },
            (None, _, None) => FootTarget {
                offset: 0.0,
                tilt: Quat::IDENTITY,
            },
        };
        FootTarget {
            tilt: limit_angle(target.tilt, max_angle),
            ..target
        }
    }

    fn pelvis_target(&self, placed: &[PlacedLeg]) -> f32 {
        let max_drop = if self.max_pelvis_drop.is_nan() {
            0.0
        } else {
            self.max_pelvis_drop.max(0.0)
        };
        placed
            .iter()
            .map(|leg| leg.target.offset)
            .fold(0.0_f32, f32::min)
            .clamp(-max_drop, 0.0)
    }
}

fn descends_from(skeleton: &Skeleton, bone: usize, ancestor: usize) -> bool {
    std::iter::successors(parent(skeleton, bone), |&up| parent(skeleton, up))
        .any(|up| up == ancestor)
}

fn pitched(foot: Vec3, toe: Vec3, ankle: Ground, toe_ground: Ground, max_angle: f32) -> FootTarget {
    let v = toe - foot;
    let flat = horizontal(v);
    let forward = flat / flat.length();
    let side = forward.cross(Vec3::Y);
    let pitch = (toe_ground.height - ankle.height)
        .atan2(flat.length())
        .clamp(-max_angle, max_angle);
    let pitch_rotation = Quat::from_axis_angle(side, pitch);
    let roll = ankle
        .normal
        .map_or(0.0, |normal| roll_about(forward, normal))
        .clamp(-max_angle, max_angle);
    let roll_rotation = Quat::from_axis_angle(forward, roll);
    let tilt = limit_angle(pitch_rotation * roll_rotation, max_angle);
    let rise = (tilt * v).y - v.y;
    FootTarget {
        offset: ankle.height.max(toe_ground.height - rise),
        tilt,
    }
}

fn roll_about(forward: Vec3, normal: Vec3) -> f32 {
    let across = normal - forward * normal.dot(forward);
    if across.length() < MIN_NORMAL_LENGTH {
        return 0.0;
    }
    forward
        .dot(Vec3::Y.cross(across))
        .atan2(Vec3::Y.dot(across))
}

fn limit_angle(tilt: Quat, max_angle: f32) -> Quat {
    let angle = tilt.angle_between(Quat::IDENTITY);
    if angle > max_angle {
        Quat::IDENTITY.slerp(tilt, max_angle / angle)
    } else {
        tilt
    }
}

fn drop_pelvis(
    resolved: &ResolvedLegs<'_>,
    world_to_model: &WorldToModel,
    drop: f32,
    output: &mut AnimationFrame,
) {
    let skeleton = resolved.skeleton;
    let in_model = world_to_model.transform.transform_vector3(Vec3::Y * drop);
    let local = match parent(skeleton, resolved.pelvis) {
        Some(up) => match model_joint(skeleton, output.joints(), up) {
            Some(joint) => joint.transform.inverse().transform_vector3(in_model),
            None => return,
        },
        None => in_model,
    };
    if let Some(joint) = output.joint_mut(resolved.pelvis) {
        joint.translation += local;
    }
}

impl AnimationNode for FootPlacementNode {
    type State = FootPlacementState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Option<Res<'static, PhysicsState>>;

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> FootPlacementState {
        FootPlacementState::new(self)
    }

    fn update(
        &self,
        state: &mut FootPlacementState,
        delta_time: f32,
        _: &(),
        _: &mut AnimationGraphContext<'_>,
    ) {
        state.elapsed = elapsed_after(state.elapsed, delta_time);
    }

    fn evaluate(
        &self,
        state: &mut FootPlacementState,
        physics: &Option<Res<PhysicsState>>,
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let elapsed = std::mem::take(&mut state.elapsed);
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.frame);
        }
        if self.legs.is_empty() {
            return;
        }
        let resolved = match self.resolve(context.skeleton()) {
            Ok(resolved) => resolved,
            Err(reason) => {
                if !state.warned {
                    warn!("foot placement on `{}` skipped: {reason}", &*self.pelvis);
                    state.warned = true;
                }
                return;
            }
        };
        let model_to_world = context.model_to_world().unwrap_or_default();
        let world_to_model = WorldToModel::new(model_to_world);
        if !model_to_world.is_finite() || !world_to_model.is_finite() {
            return;
        }
        let blackboard = context.blackboard();
        let Some(placed) = self.place(
            &resolved,
            physics.as_deref(),
            blackboard,
            model_to_world,
            output.joints(),
        ) else {
            return;
        };

        let weight_target = self.weight_target(blackboard);
        let pelvis_target = self.pelvis_target(&placed);
        state.offsets.resize(
            placed.len(),
            Spring::critically_damped_with_halflife(0.0, self.foot_halflife),
        );
        state.tilts.resize(
            placed.len(),
            Spring::critically_damped_with_halflife(Quat::IDENTITY, self.rotation_halflife),
        );
        if !state.started {
            state.weight.reset_to(weight_target);
            state.pelvis.reset_to(pelvis_target);
            for (spring, leg) in state.offsets.iter_mut().zip(&placed) {
                spring.reset_to(leg.target.offset);
            }
            for (spring, leg) in state.tilts.iter_mut().zip(&placed) {
                spring.reset_to(leg.target.tilt);
            }
            state.started = true;
        }

        let weight = ease(
            &mut state.weight,
            weight_target,
            self.weight_halflife,
            elapsed,
        )
        .clamp(0.0, 1.0);
        let pelvis = weight
            * ease(
                &mut state.pelvis,
                pelvis_target,
                self.pelvis_halflife,
                elapsed,
            );
        let goals: Vec<LegGoal> = placed
            .iter()
            .zip(state.offsets.iter_mut().zip(state.tilts.iter_mut()))
            .map(|(leg, (offset, tilt))| {
                let offset = weight * ease(offset, leg.target.offset, self.foot_halflife, elapsed);
                let tilt = Quat::IDENTITY.slerp(
                    ease(tilt, leg.target.tilt, self.rotation_halflife, elapsed),
                    weight,
                );
                LegGoal {
                    position: world_to_model
                        .transform
                        .transform_point3(leg.foot + Vec3::Y * offset),
                    tilt: world_to_model.rotation * tilt * world_to_model.rotation.inverse(),
                }
            })
            .collect();

        if weight.is_nan() || weight < MIN_WEIGHT {
            return;
        }
        drop_pelvis(&resolved, &world_to_model, pelvis, output);
        for (leg, goal) in resolved.legs.iter().zip(&goals) {
            bend_leg(
                resolved.skeleton,
                leg,
                goal.position,
                goal.tilt,
                self.softness,
                output,
            );
        }
    }

    fn reset(&self, state: &mut FootPlacementState) {
        state.elapsed = 0.0;
        state.started = false;
    }
}

impl EditorAnimationNode for FootPlacementNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        node::{AnimationNode, EditorAnimationNode, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, RootMotion, WeightedMotion},
    };
    use concerto_app::{App, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        entity::hierarchy::ChildOf,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedule},
        world::World,
    };
    use concerto_foundation::{
        assets::AssetId,
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::skeleton::Skeleton;
    use concerto_name::{Name, name};
    use concerto_physics::{
        body::BodyId,
        collider::{Collider, register_colliders},
        physics_state::PhysicsState,
    };
    use concerto_reflect::Reflect;
    use glam::{Affine3A, Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{FootPlacementLeg, FootPlacementNode, IgnoredBody};
    use crate::testing::{
        Bone, angle, app_with, assert_near, captured, captured_graph, compile, insert_asset,
        skeleton_of, spawn_rigged, write_state,
    };

    const EPS: f32 = 1.0e-4;
    const ANGLE_EPS: f32 = 1.0e-3;
    const KNEE_BEND: f32 = 0.1;
    const PELVIS: usize = 0;
    const LEFT_FOOT: usize = 3;
    const LEFT_TOE: usize = 4;
    const RIGHT_FOOT: usize = 7;
    const TOE_AHEAD: Vec3 = Vec3::new(0.0, 0.0, 0.15);
    const SHORT_TOE: Vec3 = Vec3::new(0.0, 0.0, 0.1);
    const SLOPE: f32 = 10.0_f32.to_radians();

    #[derive(Clone, Default, Reflect)]
    #[reflect(AnimationNode)]
    struct ConstantPose {
        translations: Vec<Vec3>,
        rotations: Vec<Quat>,
        root_translation: Vec3,
        root_rotation: Quat,
    }

    impl AnimationNode for ConstantPose {
        type State = ();
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            for (bone, translation) in self.translations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.translation = *translation;
                }
            }
            for (bone, rotation) in self.rotations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = *rotation;
                }
            }
        }

        fn root_motion(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[WeightedMotion],
        ) -> RootMotion {
            RootMotion {
                translation: self.root_translation,
                rotation: self.root_rotation,
            }
        }

        fn reset(&self, _: &mut ()) {}
    }

    impl EditorAnimationNode for ConstantPose {}

    impl ConstantPose {
        fn rest_of(skeleton: &Skeleton) -> Self {
            Self {
                translations: skeleton
                    .rest_pose
                    .iter()
                    .map(|joint| joint.translation)
                    .collect(),
                rotations: skeleton
                    .rest_pose
                    .iter()
                    .map(|joint| joint.rotation)
                    .collect(),
                ..Default::default()
            }
        }
    }

    fn leg_bones(names: [&'static str; 4], hip_x: f32, first: u16, toe: Vec3) -> [Bone; 4] {
        let half = 0.45;
        let joint = |translation| JointPose {
            translation,
            ..JointPose::IDENTITY
        };
        [
            Bone {
                name: names[0],
                parent: Some(0),
                joint: joint(Vec3::new(hip_x, 0.0, 0.0)),
            },
            Bone {
                name: names[1],
                parent: Some(first),
                joint: joint(Vec3::new(
                    0.0,
                    -half * KNEE_BEND.cos(),
                    half * KNEE_BEND.sin(),
                )),
            },
            Bone {
                name: names[2],
                parent: Some(first + 1),
                joint: joint(Vec3::new(
                    0.0,
                    -half * KNEE_BEND.cos(),
                    -half * KNEE_BEND.sin(),
                )),
            },
            Bone {
                name: names[3],
                parent: Some(first + 2),
                joint: joint(toe),
            },
        ]
    }

    fn rig() -> Skeleton {
        rig_with_toe(TOE_AHEAD)
    }

    fn rig_with_toe(toe: Vec3) -> Skeleton {
        let mut bones = vec![Bone {
            name: "pelvis",
            parent: None,
            joint: JointPose {
                translation: Vec3::new(0.0, 0.9, 0.0),
                ..JointPose::IDENTITY
            },
        }];
        bones.extend(leg_bones(
            ["left_hip", "left_knee", "left_foot", "left_toe"],
            0.1,
            1,
            toe,
        ));
        bones.extend(leg_bones(
            ["right_hip", "right_knee", "right_foot", "right_toe"],
            -0.1,
            5,
            toe,
        ));
        skeleton_of(&bones)
    }

    fn placement() -> FootPlacementNode {
        FootPlacementNode::new("pelvis")
            .with_leg(FootPlacementLeg::new("left_foot").with_toe("left_toe"))
            .with_leg(FootPlacementLeg::new("right_foot").with_toe("right_toe"))
    }

    struct Stage {
        app: App,
        bones: Vec<Entity>,
        parents: Vec<Option<u16>>,
        model_to_world: Affine3A,
        player: Entity,
        input: ConstantPose,
    }

    impl Stage {
        fn new(skeleton: Skeleton, input: ConstantPose, node: FootPlacementNode) -> Self {
            let mut app = app_with::<ConstantPose>();
            let bone_count = skeleton.bone_count();
            let parents = skeleton.parents.to_vec();
            let world = app.main_mut().world_mut();
            world.register_component::<Collider>();
            let skeleton = insert_asset(world, AssetId::new(), skeleton);
            let graph = insert_asset(world, AssetId::new(), captured_graph(node, input.clone()));
            let rigged = spawn_rigged(
                world,
                &skeleton,
                &graph,
                AnimationPlayer::new(bone_count),
                bone_count,
            );
            compile(world);

            Self {
                app,
                bones: rigged.bones,
                parents,
                model_to_world: Affine3A::IDENTITY,
                player: rigged.player,
                input,
            }
        }

        fn under(mut self, space: Affine3A) -> Self {
            let root = self.bones[0];
            let world = self.world();
            let parent = world.spawn(GlobalTransform::new(Mat4::from(space)));
            world.insert(ChildOf::new(parent), root);
            self.model_to_world = space;
            self
        }

        fn set_param<T: Reflect>(&mut self, key: Name, value: T) {
            let player = self.player;
            self.world()
                .get_component_for_entity_mut::<AnimationPlayer>(player)
                .unwrap()
                .set_param(key, value);
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn with_physics(mut self) -> Self {
            self.world().insert_resource(PhysicsState::new());
            self
        }

        fn add_cuboid(&mut self, half_extents: Vec3, center: Vec3, rotation: Quat) -> Entity {
            let world = self.world();
            let cuboid = world.spawn((
                Collider::cuboid(half_extents.x, half_extents.y, half_extents.z),
                Transform::from_translation_rotation(center, rotation),
            ));
            let mut registration = Schedule::new();
            registration.add_system(register_colliders);
            registration
                .compile::<SingleThreadedExecutor>(world)
                .run(world);
            cuboid
        }

        fn add_floor(&mut self, height: f32) {
            self.add_cuboid(
                Vec3::new(2.0, 0.5, 2.0),
                Vec3::new(0.0, height - 0.5, 0.0),
                Quat::IDENTITY,
            );
        }

        fn add_slab(&mut self, from_x: f32, to_x: f32, height: f32) -> Entity {
            self.add_cuboid(
                Vec3::new((to_x - from_x) / 2.0, 0.5, 2.0),
                Vec3::new((from_x + to_x) / 2.0, height - 0.5, 0.0),
                Quat::IDENTITY,
            )
        }

        fn add_slope(&mut self, rotation: Quat) {
            let center = rotation * Vec3::new(0.0, -0.5, 0.0);
            let space = self.model_to_world;
            self.add_cuboid(
                Vec3::new(2.0, 0.5, 2.0),
                space.transform_point3(center),
                rotation_of(space) * rotation,
            );
        }

        fn add_tread(&mut self) {
            self.add_cuboid(
                Vec3::new(2.0, 0.5, 1.0),
                Vec3::new(0.0, 0.17 - 0.5, 1.05),
                Quat::IDENTITY,
            );
        }

        fn run(&mut self) {
            self.world().run_schedule(LateUpdate);
        }

        fn step(&mut self, elapsed: f32) {
            let player = self.player;
            write_state::<FootPlacementNode, _>(self.world(), player, |_, state| {
                state.elapsed += elapsed
            });
            self.run();
        }

        fn settle(&mut self) {
            self.run();
            self.run();
        }

        fn local(&mut self, bone: usize) -> Transform {
            let entity = self.bones[bone];
            self.world()
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .clone()
        }

        fn output_world(&mut self, bone: usize) -> Affine3A {
            let locals: Vec<Affine3A> = (0..self.bones.len())
                .map(|bone| {
                    let local = self.local(bone);
                    Affine3A::from_scale_rotation_translation(
                        local.scale,
                        local.rotation,
                        local.translation,
                    )
                })
                .collect();
            self.model_to_world * self.in_model(&locals, bone)
        }

        fn input_world(&self, bone: usize) -> Affine3A {
            let locals: Vec<Affine3A> = (0..self.bones.len())
                .map(|bone| {
                    Affine3A::from_rotation_translation(
                        self.input.rotations[bone],
                        self.input.translations[bone],
                    )
                })
                .collect();
            self.model_to_world * self.in_model(&locals, bone)
        }

        fn in_model(&self, locals: &[Affine3A], bone: usize) -> Affine3A {
            match self.parents[bone] {
                Some(up) => self.in_model(locals, usize::from(up)) * locals[bone],
                None => locals[bone],
            }
        }

        fn rise(&mut self, bone: usize) -> f32 {
            self.output_world(bone).translation.y - self.input_world(bone).translation.y
        }

        fn tilt(&mut self, bone: usize) -> Quat {
            rotation_of(self.output_world(bone)) * rotation_of(self.input_world(bone)).inverse()
        }

        fn assert_same_xz(&mut self, bone: usize) {
            let output = self.output_world(bone).translation;
            let input = self.input_world(bone).translation;
            assert_near(output.x, input.x, EPS);
            assert_near(output.z, input.z, EPS);
        }

        fn assert_tilt(&mut self, bone: usize, slope: Quat) {
            let turn = rotation_of(self.model_to_world);
            let expected = turn * slope * turn.inverse();
            let tilt = self.tilt(bone);
            assert!(angle(tilt, expected) < ANGLE_EPS, "{tilt} vs {expected}");
        }
    }

    fn rotation_of(transform: Affine3A) -> Quat {
        transform.to_scale_rotation_translation().1
    }

    fn pitch_of(tilt: Quat, forward: Vec3) -> f32 {
        (tilt * forward).y.clamp(-1.0, 1.0).asin()
    }

    fn step_under_the_left_foot(stage: &mut Stage) {
        stage.add_floor(0.0);
        stage.add_slab(0.0, 2.0, 0.15);
    }

    struct Ground {
        name: &'static str,
        node: FootPlacementNode,
        toe: Vec3,
        space: Affine3A,
        ground: fn(&mut Stage),
        check: fn(&mut Stage),
    }

    #[test]
    fn the_feet_and_pelvis_settle_on_the_ground_under_them_within_their_limits() {
        let flat = |name, node, ground, check| Ground {
            name,
            node,
            toe: TOE_AHEAD,
            space: Affine3A::IDENTITY,
            ground,
            check,
        };
        let turned_and_raised = Affine3A::from_rotation_translation(
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::Y,
        );
        let rows = [
            flat(
                "a step under one foot",
                placement(),
                step_under_the_left_foot,
                |stage| {
                    assert_near(stage.rise(LEFT_FOOT), 0.15, EPS);
                    assert_near(stage.rise(RIGHT_FOOT), 0.0, EPS);
                    stage.assert_same_xz(LEFT_FOOT);
                    stage.assert_same_xz(RIGHT_FOOT);
                    let pelvis = stage.local(PELVIS).translation;
                    assert!(pelvis.distance(stage.input.translations[PELVIS]) < EPS);
                },
            ),
            flat(
                "a hole under one foot",
                placement(),
                |stage| {
                    stage.add_slab(0.0, 2.0, 0.0);
                    stage.add_slab(-2.0, 0.0, -0.2);
                },
                |stage| {
                    assert_near(stage.rise(PELVIS), -0.2, EPS);
                    assert_near(stage.rise(RIGHT_FOOT), -0.2, EPS);
                    assert_near(stage.rise(LEFT_FOOT), 0.0, EPS);
                    stage.assert_same_xz(LEFT_FOOT);
                    stage.assert_same_xz(RIGHT_FOOT);
                },
            ),
            flat(
                "a hole deeper than the pelvis may drop",
                placement()
                    .with_max_pelvis_drop(0.4)
                    .with_max_step_down(0.7),
                |stage| {
                    stage.add_slab(0.0, 2.0, 0.0);
                    stage.add_slab(-2.0, 0.0, -0.6);
                },
                |stage| assert_near(stage.rise(PELVIS), -0.4, EPS),
            ),
            flat(
                "half weight",
                placement().with_weight(0.5),
                step_under_the_left_foot,
                |stage| assert_near(stage.rise(LEFT_FOOT), 0.075, EPS),
            ),
            flat(
                "a slope rising ahead",
                placement(),
                |stage| stage.add_slope(Quat::from_rotation_x(-SLOPE)),
                |stage| {
                    stage.assert_tilt(LEFT_FOOT, Quat::from_rotation_x(-SLOPE));
                    let ankle = TOE_AHEAD.z * (SLOPE.tan() - SLOPE.sin());
                    assert_near(stage.rise(LEFT_FOOT), ankle, EPS);
                },
            ),
            Ground {
                space: turned_and_raised,
                ..flat(
                    "a slope rising ahead of a turned and raised model",
                    placement(),
                    |stage| stage.add_slope(Quat::from_rotation_x(-SLOPE)),
                    |stage| {
                        stage.assert_tilt(LEFT_FOOT, Quat::from_rotation_x(-SLOPE));
                        let ankle = TOE_AHEAD.z * (SLOPE.tan() - SLOPE.sin());
                        assert_near(stage.rise(LEFT_FOOT), ankle, EPS);
                    },
                )
            },
            flat(
                "a cross slope",
                placement(),
                |stage| stage.add_slope(Quat::from_rotation_z(SLOPE)),
                |stage| {
                    stage.assert_tilt(LEFT_FOOT, Quat::from_rotation_z(SLOPE));
                    let height = 0.1 * SLOPE.tan();
                    assert_near(stage.rise(LEFT_FOOT), height, EPS);
                    assert_near(stage.rise(RIGHT_FOOT), -height, EPS);
                    assert_near(stage.rise(PELVIS), -height, EPS);
                },
            ),
            flat(
                "a slope steeper than the angle limit",
                placement(),
                |stage| stage.add_slope(Quat::from_rotation_x(-45.0_f32.to_radians())),
                |stage| {
                    let pitch = pitch_of(stage.tilt(LEFT_FOOT), Vec3::Z);
                    assert_near(pitch, placement().max_foot_angle, ANGLE_EPS);
                    let toe = stage.output_world(LEFT_TOE).translation;
                    assert!(toe.y >= toe.z - EPS, "{toe:?}");
                },
            ),
            Ground {
                toe: SHORT_TOE,
                ..flat(
                    "a toe over the next tread",
                    placement(),
                    |stage| {
                        stage.add_floor(0.0);
                        stage.add_tread();
                    },
                    |stage| {
                        let limit = 30.0_f32.to_radians();
                        let pitch = pitch_of(stage.tilt(LEFT_FOOT), Vec3::Z);
                        assert_near(pitch, limit, ANGLE_EPS);
                        assert_near(stage.rise(LEFT_FOOT), 0.17 - 0.1 * limit.sin(), EPS);
                    },
                )
            },
        ];

        for row in rows {
            let skeleton = rig_with_toe(row.toe);
            let input = ConstantPose {
                root_translation: Vec3::new(0.01, 0.0, 0.04),
                root_rotation: Quat::from_rotation_y(0.03),
                ..ConstantPose::rest_of(&skeleton)
            };
            let passed = RootMotion {
                translation: input.root_translation,
                rotation: input.root_rotation,
            };
            let mut stage = Stage::new(skeleton, input, row.node)
                .with_physics()
                .under(row.space);
            (row.ground)(&mut stage);
            stage.settle();
            let player = stage.player;
            assert_eq!(
                captured(stage.world(), player).root_motion,
                passed,
                "{}",
                row.name
            );

            let checked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (row.check)(&mut stage)));
            assert!(checked.is_ok(), "{}", row.name);
        }
    }

    #[test]
    fn a_body_around_the_feet_is_hit_unless_it_is_the_ignored_body() {
        for ignored in [false, true] {
            let skeleton = rig();
            let input = ConstantPose::rest_of(&skeleton);
            let node = placement().with_ignore_param(name!("ignore"));
            let mut stage = Stage::new(skeleton, input, node).with_physics();
            stage.add_floor(0.0);
            let around = stage.add_cuboid(
                Vec3::new(0.3, 0.3, 0.3),
                Vec3::new(0.0, 0.4, 0.0),
                Quat::IDENTITY,
            );
            if ignored {
                let body = *stage
                    .world()
                    .get_component_for_entity::<BodyId>(around)
                    .unwrap();
                stage.set_param(name!("ignore"), IgnoredBody { body: Some(body) });
            }
            stage.settle();

            for foot in [LEFT_FOOT, RIGHT_FOOT] {
                let rise = stage.rise(foot);
                if ignored {
                    assert_near(rise, 0.0, EPS);
                } else {
                    assert!(rise > 0.1 - EPS, "{rise}");
                }
            }
        }
    }

    struct Easing {
        halflife: f32,
        steps: &'static [f32],
        rise: f32,
        tolerance: f32,
    }

    #[test]
    fn a_new_step_is_reached_halfway_in_one_halflife_and_at_once_without_one() {
        let rows = [
            Easing {
                halflife: 0.1,
                steps: &[0.05, 0.05],
                rise: 0.075,
                tolerance: 1.0e-3,
            },
            Easing {
                halflife: 0.0,
                steps: &[0.01],
                rise: 0.15,
                tolerance: EPS,
            },
        ];

        for row in rows {
            let skeleton = rig();
            let input = ConstantPose::rest_of(&skeleton);
            let node = placement().with_foot_halflife(row.halflife);
            let mut stage = Stage::new(skeleton, input, node.clone()).with_physics();
            stage.add_floor(0.0);
            stage.settle();
            assert_near(stage.rise(LEFT_FOOT), 0.0, EPS);

            stage.add_slab(0.0, 2.0, 0.15);
            for step in row.steps {
                stage.step(*step);
            }

            assert_near(stage.rise(LEFT_FOOT), row.rise, row.tolerance);
        }
    }
}
