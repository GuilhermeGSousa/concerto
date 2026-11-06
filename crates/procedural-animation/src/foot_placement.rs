use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    ik::{TwoBoneChain, solve_two_bone},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint, parent},
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};
use concerto_ecs::Res;
use concerto_foundation::math::{Spring, SpringValue};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_physics::{body::BodyId, physics_state::PhysicsState, query::QueryFilter};
use concerto_reflect::Reflect;
use glam::{Affine3A, Quat, Vec3};
use log::warn;
use uuid::Uuid;

const MIN_TOE_DISTANCE: f32 = 0.01;
const MIN_NORMAL_LENGTH: f32 = 1.0e-3;
const MIN_WEIGHT: f32 = 1.0e-4;

/// Puts each leg's foot on the ground under it and lowers the pelvis for the lowest; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct FootPlacementNode {
    pub pelvis: JointName,
    pub legs: Vec<FootPlacementLeg>,
    pub max_step_up: f32,
    pub max_step_down: f32,
    pub max_pelvis_drop: f32,
    pub max_foot_angle: f32,
    pub foot_halflife: f32,
    pub pelvis_halflife: f32,
    pub rotation_halflife: f32,
    pub softness: f32,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub weight_halflife: f32,
    pub ignore_param: Option<Name>,
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
    pub fn with_weight_param(mut self, weight_param: Name) -> Self {
        self.weight_param = Some(weight_param);
        self
    }

    /// Sets the halflife the weight eases with.
    pub fn with_weight_halflife(mut self, weight_halflife: f32) -> Self {
        self.weight_halflife = weight_halflife;
        self
    }

    /// Sets the param holding the [`IgnoredBody`] the rays skip.
    pub fn with_ignore_param(mut self, ignore_param: Name) -> Self {
        self.ignore_param = Some(ignore_param);
        self
    }
}

/// A leg:the foot bone names the chain (its parent the knee, its grandparent the hip), and an optional toe bone adds a second ray.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct FootPlacementLeg {
    pub foot: JointName,
    pub toe: Option<JointName>,
}

impl FootPlacementLeg {
    /// A leg on the given foot bone, without a toe.
    pub fn new(foot: impl Into<JointName>) -> Self {
        Self {
            foot: foot.into(),
            toe: None,
        }
    }

    /// Sets the toe bone the second ray is cast at.
    pub fn with_toe(mut self, toe: impl Into<JointName>) -> Self {
        self.toe = Some(toe.into());
        self
    }
}

/// The body a foot placement node's rays skip, set on the blackboard by the game: the character's own capsule.
#[derive(Clone, Copy, Debug, Default, Reflect)]
pub struct IgnoredBody {
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

struct ResolvedLeg {
    hip: usize,
    knee: usize,
    foot: usize,
    toe: Option<usize>,
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

fn elapsed_after(elapsed: f32, delta_time: f32) -> f32 {
    if delta_time.is_finite() && delta_time > 0.0 {
        elapsed + delta_time
    } else {
        elapsed
    }
}

impl FootPlacementNode {
    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedLegs<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let pelvis = bone_under_first(skeleton, self.pelvis)?;
        let mut legs = Vec::with_capacity(self.legs.len());
        for leg in &self.legs {
            let foot = bone_under_first(skeleton, leg.foot)?;
            let knee = parent(skeleton, foot)
                .ok_or_else(|| format!("foot `{}` has no parent", &*leg.foot))?;
            let hip = parent(skeleton, knee)
                .ok_or_else(|| format!("foot `{}` has no grandparent", &*leg.foot))?;
            if !descends_from(skeleton, hip, pelvis) {
                return Err(format!(
                    "the hip of foot `{}` does not descend from pelvis `{}`",
                    &*leg.foot, &*self.pelvis
                ));
            }
            let toe = leg
                .toe
                .map(|toe| bone_under_first(skeleton, toe))
                .transpose()?;
            legs.push(ResolvedLeg {
                hip,
                knee,
                foot,
                toe,
            });
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
            .and_then(|param| blackboard.get_float(param))
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
            .and_then(|param| blackboard.get::<IgnoredBody>(param))
            .and_then(|ignored| ignored.body);
        let caster = Caster {
            physics,
            filter: QueryFilter {
                ignore: ignored.as_slice(),
            },
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

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
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

fn ease<T: SpringValue>(spring: &mut Spring<T>, target: T, halflife: f32, elapsed: f32) -> T {
    spring.set_halflife(halflife);
    if halflife > 0.0 {
        spring.update(target, elapsed)
    } else {
        spring.reset_to(target);
        target
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

fn bend_leg(
    skeleton: &Skeleton,
    leg: &ResolvedLeg,
    goal: &LegGoal,
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
        goal.position,
        None,
        softness,
    );
    let hip_rotation = solution.root * hip.rotation;
    let knee_rotation = solution.mid * knee.rotation;
    let foot_rotation = goal.tilt * foot.rotation;
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
        _: &AnimationGraphContext<'_>,
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
            bend_leg(resolved.skeleton, leg, goal, self.softness, output);
        }
    }

    fn reset(&self, state: &mut FootPlacementState) {
        state.elapsed = 0.0;
        state.started = false;
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{AnimationNode, ReflectAnimationNode},
        player::{AnimationHandleComponent, AnimationPlayer},
        plugin::{AnimationAppExt, AnimationPlugin, register_animation_types},
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, RootMotion},
        root::AnimationRootBone,
    };
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        component::scene::SceneEntityRef,
        entity::hierarchy::ChildOf,
        system::{
            executor::single_thread::SingleThreadedExecutor,
            schedule::{Schedule, Schedules},
        },
        world::World,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::{
        JointName,
        skeleton::{Skeleton, SkeletonComponent},
    };
    use concerto_name::{Name, name};
    use concerto_physics::{
        body::BodyId,
        collider::{Collider, register_colliders},
        physics_state::PhysicsState,
    };
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Affine3A, Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{
        FootPlacementLeg, FootPlacementNode, FootPlacementState, IgnoredBody, elapsed_after,
        unit_normal,
    };
    use crate::plugin::ProceduralAnimationPlugin;

    const EPS: f32 = 1.0e-4;
    const ANGLE_EPS: f32 = 1.0e-3;
    const KNEE_BEND: f32 = 0.1;
    const PELVIS: usize = 0;
    const LEFT_FOOT: usize = 3;
    const LEFT_TOE: usize = 4;
    const RIGHT_FOOT: usize = 7;
    const TOE_AHEAD: Vec3 = Vec3::new(0.0, 0.0, 0.15);
    const SLOPE: f32 = 10.0_f32.to_radians();

    #[derive(Default, Reflect)]
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

        fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

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
            output.set_root_motion(RootMotion {
                translation: self.root_translation,
                rotation: self.root_rotation,
            });
        }

        fn reset(&self, _: &mut ()) {}
    }

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

        fn shifted_of(skeleton: &Skeleton) -> Self {
            let mut pose = Self::rest_of(skeleton);
            pose.translations[0] += Vec3::new(0.02, -0.03, 0.01);
            pose.rotations[0] = Quat::from_rotation_y(0.1);
            pose.rotations[2] = Quat::from_rotation_x(0.05);
            pose
        }
    }

    struct Bone {
        name: &'static str,
        parent: Option<u16>,
        translation: Vec3,
    }

    fn skeleton_of(bones: &[Bone]) -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones
                .iter()
                .map(|bone| JointPose {
                    translation: bone.translation,
                    ..JointPose::IDENTITY
                })
                .collect(),
            (0..bones.len() as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
    }

    fn leg_bones(names: [&'static str; 4], hip_x: f32, first: u16, toe: Vec3) -> [Bone; 4] {
        let half = 0.45;
        [
            Bone {
                name: names[0],
                parent: Some(0),
                translation: Vec3::new(hip_x, 0.0, 0.0),
            },
            Bone {
                name: names[1],
                parent: Some(first),
                translation: Vec3::new(0.0, -half * KNEE_BEND.cos(), half * KNEE_BEND.sin()),
            },
            Bone {
                name: names[2],
                parent: Some(first + 1),
                translation: Vec3::new(0.0, -half * KNEE_BEND.cos(), -half * KNEE_BEND.sin()),
            },
            Bone {
                name: names[3],
                parent: Some(first + 2),
                translation: toe,
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
            translation: Vec3::new(0.0, 0.9, 0.0),
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

    struct SlopedFoot {
        height: f32,
        pitch: f32,
    }

    impl Stage {
        fn new(skeleton: Skeleton, input: ConstantPose, node: FootPlacementNode) -> Self {
            let mut app = App::new();
            app.register_plugin(AssetManagerPlugin);
            app.insert_resource(Time::new());
            app.register_plugin(AnimationPlugin);
            app.register_plugin(ProceduralAnimationPlugin);
            app.register_animation_node::<ConstantPose>();

            let bone_count = skeleton.bone_count();
            let parents = skeleton.parents.to_vec();
            let world = app.main_mut().world_mut();
            world.register_component::<Transform>();
            world.register_component::<Collider>();
            let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<Skeleton>>()
                .unwrap()
                .insert(skeleton_handle.id(), skeleton);
            let graph_handle = add_graph(world, &input, node);
            let bones: Vec<Entity> = (0..bone_count)
                .map(|_| world.spawn(Transform::IDENTITY))
                .collect();
            let player = world.spawn((
                AnimationPlayer::new(bone_count),
                AnimationHandleComponent::new(graph_handle),
                SkeletonComponent::new(
                    skeleton_handle,
                    bones
                        .iter()
                        .map(|bone| SceneEntityRef::Entity(*bone))
                        .collect(),
                    None,
                ),
            ));
            let schedules = world.remove_resource::<Schedules>().unwrap();
            let compiled = schedules.compile::<SingleThreadedExecutor>(world);
            world.insert_resource(compiled);

            Self {
                app,
                bones,
                parents,
                model_to_world: Affine3A::IDENTITY,
                player,
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

        fn run(&mut self) {
            self.world().run_schedule(LateUpdate);
        }

        fn settle(&mut self) {
            self.run();
            self.run();
        }

        fn with_state<R>(&mut self, read: impl FnOnce(&mut FootPlacementState) -> R) -> R {
            let player = self.player;
            let mut read = Some(read);
            let mut result = None;
            self.world()
                .resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
                    world
                        .get_component_for_entity_mut::<AnimationPlayer>(player)
                        .unwrap()
                        .for_each_state_mut::<FootPlacementNode>(graphs, |_, state, _| {
                            if let Some(read) = read.take() {
                                result = Some(read(state));
                            }
                        });
                });
            result.expect("the graph holds a foot placement node")
        }

        fn warned(&mut self) -> bool {
            self.with_state(|state| state.warned)
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

        fn assert_input_pose(&mut self) {
            for bone in 0..self.bones.len() {
                let local = self.local(bone);
                let translation = self.input.translations[bone];
                let rotation = self.input.rotations[bone];
                assert!(
                    local.translation.distance(translation) < EPS,
                    "bone {bone}: {:?} vs {translation:?}",
                    local.translation
                );
                assert!(
                    angle(local.rotation, rotation) < EPS,
                    "bone {bone}: {:?} vs {rotation:?}",
                    local.rotation
                );
            }
        }
    }

    fn angle(a: Quat, b: Quat) -> f32 {
        2.0 * (a * b.inverse()).xyz().length().min(1.0).asin()
    }

    fn rotation_of(transform: Affine3A) -> Quat {
        transform.to_scale_rotation_translation().1
    }

    fn pitch_of(tilt: Quat, forward: Vec3) -> f32 {
        (tilt * forward).y.clamp(-1.0, 1.0).asin()
    }

    fn assert_near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() < tolerance,
            "{actual} vs {expected}"
        );
    }

    fn settled(node: FootPlacementNode, build: impl FnOnce(&mut Stage)) -> Stage {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, node).with_physics();
        build(&mut stage);
        stage.settle();
        stage
    }

    fn add_graph(
        world: &mut World,
        input: &ConstantPose,
        node: FootPlacementNode,
    ) -> AssetHandle<AnimationGraph> {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<FootPlacementNode>();
        registry.register::<ConstantPose>();
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(node, |mut placement| {
            placement.with_input(
                ConstantPose {
                    translations: input.translations.clone(),
                    rotations: input.rotations.clone(),
                    root_translation: input.root_translation,
                    root_rotation: input.root_rotation,
                },
                |_| {},
            );
        });
        graph.make_live(&registry, None).unwrap();
        let handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(handle.id(), graph);
        handle
    }

    #[test]
    fn the_builder_sets_every_field_over_the_defaults() {
        let built = FootPlacementNode::new("hips")
            .with_leg(FootPlacementLeg::new("left_foot").with_toe("left_toe"))
            .with_leg(FootPlacementLeg::new("right_foot"))
            .with_max_step_up(0.4)
            .with_max_step_down(0.6)
            .with_max_pelvis_drop(0.3)
            .with_max_foot_angle(0.4)
            .with_foot_halflife(0.07)
            .with_pelvis_halflife(0.12)
            .with_rotation_halflife(0.08)
            .with_softness(0.02)
            .with_weight(0.8)
            .with_weight_param(name!("weight"))
            .with_weight_halflife(0.25)
            .with_ignore_param(name!("ignore"));

        assert_eq!(
            built,
            FootPlacementNode {
                pelvis: JointName::from("hips"),
                legs: vec![
                    FootPlacementLeg {
                        foot: JointName::from("left_foot"),
                        toe: Some(JointName::from("left_toe")),
                    },
                    FootPlacementLeg {
                        foot: JointName::from("right_foot"),
                        toe: None,
                    },
                ],
                max_step_up: 0.4,
                max_step_down: 0.6,
                max_pelvis_drop: 0.3,
                max_foot_angle: 0.4,
                foot_halflife: 0.07,
                pelvis_halflife: 0.12,
                rotation_halflife: 0.08,
                softness: 0.02,
                weight: 0.8,
                weight_param: Some(name!("weight")),
                weight_halflife: 0.25,
                ignore_param: Some(name!("ignore")),
            }
        );
        assert_eq!(
            FootPlacementNode::new("hips"),
            FootPlacementNode {
                pelvis: JointName::from("hips"),
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_leg_the_node_cannot_resolve_leaves_the_input_pose_and_marks_its_warning() {
        let unresolvable = [
            FootPlacementNode::new("pelvis").with_leg(FootPlacementLeg::new("missing_foot")),
            FootPlacementNode::new("pelvis").with_leg(FootPlacementLeg::new("left_hip")),
            FootPlacementNode {
                pelvis: "right_hip".into(),
                ..placement()
            },
        ];

        for node in unresolvable {
            let skeleton = rig();
            let input = ConstantPose::shifted_of(&skeleton);
            let mut stage = Stage::new(skeleton, input, node.clone()).with_physics();
            stage.add_floor(0.1);
            stage.settle();

            stage.assert_input_pose();
            assert!(stage.warned(), "{node:?}");
        }
    }

    #[test]
    fn a_resolvable_node_does_not_warn() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.settle();

        assert!(!stage.warned());
    }

    #[test]
    fn without_physics_the_output_is_the_input_pose() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement());
        stage.settle();

        stage.assert_input_pose();
        assert!(!stage.warned());
    }

    #[test]
    fn the_inputs_root_motion_reaches_the_output_unchanged() {
        let skeleton = rig();
        let moved = RootMotion {
            translation: Vec3::new(0.01, 0.0, 0.04),
            rotation: Quat::from_rotation_y(0.03),
        };
        let input = ConstantPose {
            root_translation: moved.translation,
            root_rotation: moved.rotation,
            ..ConstantPose::shifted_of(&skeleton)
        };
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.1);
        let root = stage.bones[0];
        stage.world().insert(AnimationRootBone::default(), root);
        stage.settle();

        let root_motion = stage
            .world()
            .get_component_for_entity::<AnimationRootBone>(root)
            .unwrap()
            .root_motion;
        assert_eq!(root_motion, moved);
    }

    #[test]
    fn evaluate_takes_the_elapsed_time() {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.settle();

        stage.with_state(|state| state.elapsed = 0.3);
        stage.run();

        assert_eq!(stage.with_state(|state| state.elapsed), 0.0);
    }

    #[test]
    fn a_flat_floor_at_the_origins_height_leaves_the_input_pose() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.settle();

        stage.assert_input_pose();
    }

    #[test]
    fn a_step_under_one_foot_raises_that_foot_alone() {
        let mut stage = settled(placement(), |stage| {
            stage.add_floor(0.0);
            stage.add_slab(0.0, 2.0, 0.15);
        });

        assert_near(stage.rise(LEFT_FOOT), 0.15, EPS);
        assert_near(stage.rise(RIGHT_FOOT), 0.0, EPS);
        stage.assert_same_xz(LEFT_FOOT);
        stage.assert_same_xz(RIGHT_FOOT);
        let pelvis = stage.local(PELVIS).translation;
        assert!(pelvis.distance(stage.input.translations[PELVIS]) < EPS);
    }

    #[test]
    fn a_hole_under_one_foot_drops_the_pelvis_and_that_foot() {
        let mut stage = settled(placement(), |stage| {
            stage.add_slab(0.0, 2.0, 0.0);
            stage.add_slab(-2.0, 0.0, -0.2);
        });

        assert_near(stage.rise(PELVIS), -0.2, EPS);
        assert_near(stage.rise(RIGHT_FOOT), -0.2, EPS);
        assert_near(stage.rise(LEFT_FOOT), 0.0, EPS);
        stage.assert_same_xz(LEFT_FOOT);
        stage.assert_same_xz(RIGHT_FOOT);
    }

    #[test]
    fn the_pelvis_drops_no_further_than_its_limit() {
        let node = placement()
            .with_max_pelvis_drop(0.4)
            .with_max_step_down(0.7);
        let mut stage = settled(node, |stage| {
            stage.add_slab(0.0, 2.0, 0.0);
            stage.add_slab(-2.0, 0.0, -0.6);
        });

        assert_near(stage.rise(PELVIS), -0.4, EPS);
    }

    #[test]
    fn ground_deeper_than_the_step_down_leaves_the_foot_at_its_input_height() {
        let mut stage = settled(placement(), |stage| {
            stage.add_slab(0.0, 2.0, 0.0);
            stage.add_slab(-2.0, 0.0, -0.6);
        });

        assert_near(stage.rise(RIGHT_FOOT), 0.0, EPS);
        assert_near(stage.rise(PELVIS), 0.0, EPS);
    }

    #[test]
    fn a_slope_rising_ahead_pitches_the_foot_up_and_keeps_the_ankle_on_it() {
        let mut stage = settled(placement(), |stage| {
            stage.add_slope(Quat::from_rotation_x(-SLOPE));
        });

        assert_near(pitch_of(stage.tilt(LEFT_FOOT), Vec3::Z), SLOPE, ANGLE_EPS);
        let toe_above_ankle = stage.rise(LEFT_TOE) - stage.rise(LEFT_FOOT);
        assert_near(toe_above_ankle, TOE_AHEAD.z * SLOPE.sin(), EPS);
        assert!(stage.rise(LEFT_TOE) > 0.0);
        assert_near(
            stage.rise(LEFT_FOOT),
            TOE_AHEAD.z * (SLOPE.tan() - SLOPE.sin()),
            EPS,
        );
    }

    #[test]
    fn a_cross_slope_rolls_the_foot_toward_it_without_pitch() {
        let rotation = Quat::from_rotation_z(SLOPE);
        let mut stage = settled(placement(), |stage| stage.add_slope(rotation));

        let tilt = stage.tilt(LEFT_FOOT);
        assert!(
            (tilt * Vec3::Y).distance(rotation * Vec3::Y) < ANGLE_EPS,
            "{tilt:?}"
        );
        assert!((tilt * Vec3::Z).distance(Vec3::Z) < ANGLE_EPS, "{tilt:?}");
        let height = 0.1 * SLOPE.tan();
        assert_near(stage.rise(LEFT_FOOT), height, EPS);
        assert_near(stage.rise(RIGHT_FOOT), -height, EPS);
        assert_near(stage.rise(PELVIS), -height, EPS);
    }

    fn sloped_left_foot(space: Affine3A) -> SlopedFoot {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement())
            .with_physics()
            .under(space);
        stage.add_slope(Quat::from_rotation_x(-SLOPE));
        stage.settle();

        let foot = stage.output_world(LEFT_FOOT).translation.y;
        SlopedFoot {
            height: foot - space.translation.y,
            pitch: pitch_of(stage.tilt(LEFT_FOOT), space.transform_vector3(Vec3::Z)),
        }
    }

    #[test]
    fn a_turned_and_raised_model_places_its_feet_as_an_unturned_one() {
        let unturned = sloped_left_foot(Affine3A::IDENTITY);
        let turned = sloped_left_foot(Affine3A::from_rotation_translation(
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::Y,
        ));

        assert_near(unturned.pitch, SLOPE, ANGLE_EPS);
        assert_near(turned.height, unturned.height, EPS);
        assert_near(turned.pitch, unturned.pitch, ANGLE_EPS);
    }

    #[test]
    fn a_steep_slope_pitches_the_foot_by_the_angle_limit_with_the_toe_above_it() {
        let steep = 45.0_f32.to_radians();
        let mut stage = settled(placement(), |stage| {
            stage.add_slope(Quat::from_rotation_x(-steep));
        });

        assert_near(
            pitch_of(stage.tilt(LEFT_FOOT), Vec3::Z),
            placement().max_foot_angle,
            ANGLE_EPS,
        );
        let toe = stage.output_world(LEFT_TOE).translation;
        assert!(toe.y >= toe.z * steep.tan() - EPS, "{toe:?}");
    }

    #[test]
    fn a_toe_over_the_next_tread_clamps_the_pitch_and_lifts_the_ankle() {
        let skeleton = rig_with_toe(Vec3::new(0.0, 0.0, 0.1));
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_floor(0.0);
        stage.add_cuboid(
            Vec3::new(2.0, 0.5, 1.0),
            Vec3::new(0.0, 0.17 - 0.5, 1.05),
            Quat::IDENTITY,
        );
        stage.settle();

        let limit = 30.0_f32.to_radians();
        assert_near(pitch_of(stage.tilt(LEFT_FOOT), Vec3::Z), limit, ANGLE_EPS);
        assert_near(stage.rise(LEFT_FOOT), 0.17 - 0.1 * limit.sin(), EPS);
    }

    #[test]
    fn a_toe_over_the_next_tread_from_a_cross_slope_stays_on_its_hit() {
        let skeleton = rig_with_toe(Vec3::new(0.0, 0.0, 0.1));
        let input = ConstantPose::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement()).with_physics();
        stage.add_slope(Quat::from_rotation_z(6.0_f32.to_radians()));
        stage.add_cuboid(
            Vec3::new(2.0, 0.5, 1.0),
            Vec3::new(0.0, 0.17 - 0.5, 1.05),
            Quat::IDENTITY,
        );
        stage.settle();

        let toe = stage.rise(LEFT_TOE);
        assert!(toe > 0.17 - EPS, "{toe}");
    }

    #[test]
    fn a_toe_hit_without_an_ankle_hit_lifts_the_foot_to_it_untilted() {
        let mut stage = settled(placement(), |stage| {
            stage.add_cuboid(
                Vec3::new(2.0, 0.5, 1.0),
                Vec3::new(0.0, 0.05 - 0.5, 1.1),
                Quat::IDENTITY,
            );
        });

        assert_near(stage.rise(LEFT_FOOT), 0.05, EPS);
        assert!(angle(stage.tilt(LEFT_FOOT), Quat::IDENTITY) < ANGLE_EPS);
    }

    #[test]
    fn a_normal_that_is_not_finite_or_too_short_is_no_normal() {
        for normal in [
            Vec3::ZERO,
            Vec3::splat(1.0e-4),
            Vec3::NAN,
            Vec3::new(0.0, f32::INFINITY, 0.0),
        ] {
            assert_eq!(unit_normal(normal), None, "{normal:?}");
        }
        assert_eq!(unit_normal(Vec3::new(0.0, 2.0, 0.0)), Some(Vec3::Y));
    }

    #[test]
    fn a_leg_without_a_toe_tilts_its_foot_to_the_ankle_hits_normal() {
        let node = FootPlacementNode::new("pelvis")
            .with_leg(FootPlacementLeg::new("left_foot"))
            .with_leg(FootPlacementLeg::new("right_foot"));
        let rotation = Quat::from_rotation_x(-SLOPE);
        let mut stage = settled(node, |stage| stage.add_slope(rotation));

        let up = stage.tilt(LEFT_FOOT) * Vec3::Y;
        assert!(up.distance(rotation * Vec3::Y) < ANGLE_EPS, "{up:?}");
    }

    fn around_the_feet(stage: &mut Stage) -> Entity {
        stage.add_floor(0.0);
        stage.add_cuboid(
            Vec3::new(0.3, 0.3, 0.3),
            Vec3::new(0.0, 0.4, 0.0),
            Quat::IDENTITY,
        )
    }

    #[test]
    fn a_body_around_the_feet_is_hit_when_nothing_is_ignored() {
        let mut stage = settled(placement(), |stage| {
            around_the_feet(stage);
        });

        assert!(stage.rise(LEFT_FOOT) > 0.1 - EPS);
        assert!(stage.rise(RIGHT_FOOT) > 0.1 - EPS);
    }

    #[test]
    fn the_ignored_body_is_skipped_and_the_feet_land_on_the_floor_below() {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let node = placement().with_ignore_param(name!("ignore"));
        let mut stage = Stage::new(skeleton, input, node).with_physics();
        let around = around_the_feet(&mut stage);
        let body = *stage
            .world()
            .get_component_for_entity::<BodyId>(around)
            .unwrap();
        stage.set_param(name!("ignore"), IgnoredBody { body: Some(body) });
        stage.settle();

        assert_near(stage.rise(LEFT_FOOT), 0.0, EPS);
        assert_near(stage.rise(RIGHT_FOOT), 0.0, EPS);
    }

    fn step_under_the_left_foot(stage: &mut Stage) {
        stage.add_floor(0.0);
        stage.add_slab(0.0, 2.0, 0.15);
    }

    #[test]
    fn weight_zero_leaves_the_input_pose() {
        let node = placement().with_weight(0.0);
        let mut stage = settled(node, step_under_the_left_foot);

        stage.assert_input_pose();
    }

    #[test]
    fn half_weight_gives_half_the_offset() {
        let node = placement().with_weight(0.5);
        let mut stage = settled(node, step_under_the_left_foot);

        assert_near(stage.rise(LEFT_FOOT), 0.075, EPS);
    }

    #[test]
    fn a_weight_param_of_half_gives_half_the_offset() {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let node = placement().with_weight_param(name!("weight"));
        let mut stage = Stage::new(skeleton, input, node).with_physics();
        step_under_the_left_foot(&mut stage);
        stage.set_param(name!("weight"), 0.5_f32);
        stage.settle();

        assert_near(stage.rise(LEFT_FOOT), 0.075, EPS);
    }

    #[test]
    fn a_new_step_is_reached_halfway_after_one_halflife() {
        let node = placement().with_foot_halflife(0.1);
        let mut stage = settled(node, |stage| stage.add_floor(0.0));
        assert_near(stage.rise(LEFT_FOOT), 0.0, EPS);

        stage.add_slab(0.0, 2.0, 0.15);
        stage.with_state(|state| state.elapsed = 0.1);
        stage.run();

        assert_near(stage.rise(LEFT_FOOT), 0.075, 1.0e-3);
    }

    #[test]
    fn a_halflife_that_is_nan_or_not_positive_snaps() {
        for halflife in [f32::NAN, 0.0, -0.1] {
            let node = placement().with_foot_halflife(halflife);
            let mut stage = settled(node, |stage| stage.add_floor(0.0));

            stage.add_slab(0.0, 2.0, 0.15);
            stage.with_state(|state| state.elapsed = 0.01);
            stage.run();

            assert_near(stage.rise(LEFT_FOOT), 0.15, EPS);
        }
    }

    #[test]
    fn a_model_to_world_that_cannot_be_inverted_leaves_the_input_pose() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, input, placement())
            .with_physics()
            .under(Affine3A::from_scale(Vec3::ZERO));
        step_under_the_left_foot(&mut stage);
        stage.settle();

        stage.assert_input_pose();
    }

    #[test]
    fn only_a_finite_positive_delta_time_adds_to_the_elapsed_time() {
        assert_eq!(elapsed_after(0.1, 0.2), 0.1 + 0.2);
        for delta_time in [0.0, -0.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(elapsed_after(0.1, delta_time), 0.1, "{delta_time}");
        }
    }

    #[test]
    fn a_reset_node_snaps_on_its_next_evaluate() {
        let node = placement().with_foot_halflife(0.1);
        let mut stage = settled(node.clone(), |stage| stage.add_floor(0.0));

        stage.add_slab(0.0, 2.0, 0.15);
        stage.with_state(|state| node.reset(state));
        stage.run();

        assert_near(stage.rise(LEFT_FOOT), 0.15, EPS);
    }

    #[test]
    fn the_springs_ease_while_the_weight_is_zero() {
        let skeleton = rig();
        let input = ConstantPose::rest_of(&skeleton);
        let node = placement()
            .with_foot_halflife(0.1)
            .with_weight_param(name!("weight"))
            .with_weight_halflife(0.0);
        let mut stage = Stage::new(skeleton, input, node).with_physics();
        stage.add_floor(0.0);
        stage.set_param(name!("weight"), 0.0_f32);
        stage.settle();

        stage.add_slab(0.0, 2.0, 0.15);
        stage.with_state(|state| state.elapsed = 0.1);
        stage.run();
        stage.assert_input_pose();

        stage.set_param(name!("weight"), 1.0_f32);
        stage.with_state(|state| state.elapsed = 0.0);
        stage.run();

        assert_near(stage.rise(LEFT_FOOT), 0.075, 1.0e-3);
    }

    #[test]
    fn a_node_without_legs_passes_its_input_through() {
        let skeleton = rig();
        let input = ConstantPose::shifted_of(&skeleton);
        let node = FootPlacementNode::new("pelvis");
        let mut stage = Stage::new(skeleton, input, node).with_physics();
        step_under_the_left_foot(&mut stage);
        stage.settle();

        stage.assert_input_pose();
        assert!(!stage.warned());
    }
}
