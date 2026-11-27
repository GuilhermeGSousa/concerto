use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
    node::{
        AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint},
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};
use concerto_foundation::math::{Spring, SpringValue};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::name;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::look_at::{descends, halflife, limits, share};

const AIM_PASSES: usize = 3;
const SNAP_WEIGHT: f32 = 1.0e-4;

/// Turns a chain of bones so a muzzle's barrel points at a world-space point.
///
/// It passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct AimNode {
    /// The bones that turn, from the top of the chain down.
    pub chain: Vec<AimBone>,
    /// Where the barrel is.
    pub muzzle: Muzzle,
    /// The model-space up that yaw turns about.
    pub up: Vec3,
    /// The `Vec3` blackboard param holding the world-space point to aim at.
    pub target_param: Param<Vec3>,
    /// The weight the aim is applied at, before the param scales it; clamped to 0 to 1.
    pub weight: f32,
    /// The `f32` blackboard param the weight is multiplied by, when it holds one.
    pub weight_param: Option<Param<f32>>,
    /// The halflife, in seconds, the weight eases in and out with.
    pub weight_halflife: f32,
    /// The halflife, in seconds, the bones ease back onto the solve with; 0 applies it at once.
    pub reacquire_halflife: f32,
    /// The `f32` blackboard param the re-acquire halflife is multiplied by, when it holds one.
    pub reacquire_param: Option<Param<f32>>,
}

impl Default for AimNode {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            muzzle: Muzzle::default(),
            up: Vec3::Y,
            target_param: Param::default(),
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
            reacquire_halflife: 0.0,
            reacquire_param: None,
        }
    }
}

/// A bone of the aim chain: its share of the turn and its limits, in radians.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct AimBone {
    /// The bone, which must descend from the bone above it in the chain.
    pub bone: JointName,
    /// The bone's share of the turn, relative to the bones below it; 0 or less turns it not at
    /// all.
    pub share: f32,
    /// The most the bone yaws, in radians.
    pub yaw_limit: f32,
    /// The most the bone pitches, in radians.
    pub pitch_limit: f32,
}

impl Default for AimBone {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            share: 1.0,
            yaw_limit: 45.0_f32.to_radians(),
            pitch_limit: 30.0_f32.to_radians(),
        }
    }
}

/// Where the barrel is: a point and a direction in `bone`'s space.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct Muzzle {
    /// The bone the barrel is fixed to.
    pub bone: JointName,
    /// The barrel's tip, in the bone's space.
    pub offset: Vec3,
    /// The direction the barrel points, in the bone's space.
    pub direction: Vec3,
}

impl Default for Muzzle {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            offset: Vec3::ZERO,
            direction: Vec3::Z,
        }
    }
}

impl AimNode {
    /// A node with the default settings and no chain, pointing `muzzle` at the world `Vec3` under `target_param`.
    pub fn new(target_param: impl Into<Param<Vec3>>, muzzle: Muzzle) -> Self {
        Self {
            target_param: target_param.into(),
            muzzle,
            ..Self::default()
        }
    }

    /// Adds a chain bone, below the ones already added.
    pub fn with_bone(mut self, bone: AimBone) -> Self {
        self.chain.push(bone);
        self
    }

    /// Sets the model-space up that yaw turns about.
    pub fn with_up(mut self, up: Vec3) -> Self {
        self.up = up;
        self
    }

    /// Sets the weight, before the param scales it.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Multiplies the weight by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_weight_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(param.into());
        self
    }

    /// Sets the halflife, in seconds, the weight eases in and out with.
    pub fn with_weight_halflife(mut self, weight_halflife: f32) -> Self {
        self.weight_halflife = weight_halflife;
        self
    }

    /// Sets the halflife, in seconds, the bones ease back onto the solve with; 0 applies it at once.
    pub fn with_reacquire_halflife(mut self, reacquire_halflife: f32) -> Self {
        self.reacquire_halflife = reacquire_halflife;
        self
    }

    /// Multiplies the re-acquire halflife by the `f32` blackboard param named `param`, when it holds one.
    pub fn with_reacquire_param(mut self, param: impl Into<Param<f32>>) -> Self {
        self.reacquire_param = Some(param.into());
        self
    }
}

impl AimBone {
    /// The bone named `bone`, with the default share and limits.
    pub fn new(bone: impl Into<JointName>) -> Self {
        Self {
            bone: bone.into(),
            ..Self::default()
        }
    }

    /// Sets the bone's share of the turn.
    pub fn with_share(mut self, share: f32) -> Self {
        self.share = share;
        self
    }

    /// Sets the most the bone yaws and pitches, in radians.
    pub fn with_limits(mut self, yaw: f32, pitch: f32) -> Self {
        self.yaw_limit = yaw;
        self.pitch_limit = pitch;
        self
    }
}

impl Muzzle {
    /// A muzzle at the origin of `bone`, along its +Z.
    pub fn new(bone: impl Into<JointName>) -> Self {
        Self {
            bone: bone.into(),
            ..Self::default()
        }
    }

    /// Sets the barrel's point in the bone's space.
    pub fn with_offset(mut self, offset: Vec3) -> Self {
        self.offset = offset;
        self
    }

    /// Sets the barrel's direction in the bone's space.
    pub fn with_direction(mut self, direction: Vec3) -> Self {
        self.direction = direction;
        self
    }
}

/// The time an [`AimNode`] owes since its last evaluate, its eased weight and the eased turn of each chain bone, the pose it solves on, and whether it has warned about bones it cannot resolve.
#[derive(Debug)]
pub struct AimState {
    elapsed: f32,
    fresh: bool,
    weight: Spring<f32>,
    offsets: Vec<Spring<Quat>>,
    working: Vec<JointPose>,
    warned: bool,
}

impl AimState {
    fn take_time(&mut self) -> f32 {
        let elapsed = std::mem::take(&mut self.elapsed);
        if std::mem::take(&mut self.fresh) {
            0.0
        } else {
            elapsed
        }
    }

    fn offsets_to_rest(&mut self) {
        for offset in &mut self.offsets {
            offset.reset_to(Quat::IDENTITY);
        }
    }

    fn hold_offsets(&mut self) {
        for offset in &mut self.offsets {
            let held = offset.value;
            offset.reset_to(held);
        }
    }
}

fn identity_offsets(count: usize) -> Vec<Spring<Quat>> {
    vec![Spring::critically_damped(Quat::IDENTITY, 0.0); count]
}

fn ease<T: SpringValue>(spring: &mut Spring<T>, goal: T, halflife: f32, delta_time: f32) -> T {
    if halflife > 0.0 {
        spring.set_halflife(halflife);
        spring.update(goal, delta_time)
    } else {
        spring.reset_to(goal);
        goal
    }
}

fn scaled(value: f32, param: Option<Param<f32>>, blackboard: &AnimationBlackboard) -> f32 {
    value
        * param
            .and_then(|param| blackboard.read(&param).copied())
            .unwrap_or(1.0)
}

struct AimLink {
    bone: usize,
    part: f32,
    limits: YawPitch,
}

struct ResolvedAim<'a> {
    skeleton: &'a Skeleton,
    links: Vec<AimLink>,
    muzzle: usize,
    offset: Vec3,
    direction: Vec3,
}

struct Barrel {
    point: Vec3,
    direction: Vec3,
}

impl ResolvedAim<'_> {
    fn barrel(&self, joints: &[JointPose]) -> Option<Barrel> {
        let joint = model_joint(self.skeleton, joints, self.muzzle)?;
        Some(Barrel {
            point: joint.transform.transform_point3(self.offset),
            direction: joint.rotation * self.direction,
        })
    }

    fn bone_frame(
        &self,
        joints: &[JointPose],
        bone: usize,
        barrel: &Barrel,
        up: Vec3,
    ) -> Option<LookFrame> {
        let inverse = model_joint(self.skeleton, joints, bone)?.rotation.inverse();
        LookFrame::new(inverse * barrel.direction, inverse * up)
    }
}

impl AimNode {
    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedAim<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let shares: Vec<f32> = self.chain.iter().map(|link| share(link.share)).collect();
        let mut links: Vec<AimLink> = Vec::with_capacity(self.chain.len());
        for (index, link) in self.chain.iter().enumerate() {
            let bone = bone_under_first(skeleton, link.bone)?;
            if links.iter().any(|taken| taken.bone == bone) {
                return Err(format!("bone `{}` appears twice", &*link.bone));
            }
            if let Some(above) = links.last()
                && !descends(skeleton, bone, above.bone)
            {
                return Err(format!(
                    "chain bone `{}` does not descend from the chain bone before it",
                    &*link.bone
                ));
            }
            let left: f32 = shares[index..].iter().sum();
            links.push(AimLink {
                bone,
                part: if left > 0.0 {
                    shares[index] / left
                } else {
                    0.0
                },
                limits: limits(link.yaw_limit, link.pitch_limit),
            });
        }
        let last = links.last().ok_or("the chain is empty")?.bone;
        let muzzle = bone_under_first(skeleton, self.muzzle.bone)?;
        if muzzle != last && !descends(skeleton, muzzle, last) {
            return Err(format!(
                "muzzle bone `{}` is not the last chain bone or under it",
                &*self.muzzle.bone
            ));
        }
        let direction = self
            .muzzle
            .direction
            .try_normalize()
            .ok_or("the muzzle direction is zero or not finite")?;
        Ok(ResolvedAim {
            skeleton,
            links,
            muzzle,
            offset: self.muzzle.offset,
            direction,
        })
    }

    fn effective_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
        let weight = scaled(self.weight, self.weight_param, blackboard);
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

    fn reacquire_halflife(&self, blackboard: &AnimationBlackboard) -> f32 {
        halflife(scaled(
            self.reacquire_halflife,
            self.reacquire_param,
            blackboard,
        ))
    }

    fn target(&self, context: &AnimationGraphContext<'_>) -> Option<Vec3> {
        let target = *context.blackboard().read(&self.target_param)?;
        let world_to_model = WorldToModel::new(context.model_to_world().unwrap_or_default());
        if !world_to_model.is_finite() {
            return None;
        }
        let target = world_to_model.transform.transform_point3(target);
        target.is_finite().then_some(target)
    }

    fn solve(
        &self,
        aim: &ResolvedAim<'_>,
        input: &[JointPose],
        target: Vec3,
        passes: usize,
        working: &mut Vec<JointPose>,
    ) -> Option<Vec<Quat>> {
        let barrel = aim.barrel(input)?;
        let first = model_joint(aim.skeleton, input, aim.links.first()?.bone)?;
        let first = Vec3::from(first.transform.translation);
        if (target - barrel.point).dot(barrel.direction) <= 0.0
            || target.distance(first) < barrel.point.distance(first)
        {
            return None;
        }
        let frames = aim
            .links
            .iter()
            .map(|link| aim.bone_frame(input, link.bone, &barrel, self.up))
            .collect::<Option<Vec<LookFrame>>>()?;
        working.clear();
        working.extend_from_slice(input);
        let mut turns = vec![YawPitch::default(); aim.links.len()];
        for _ in 0..passes {
            for ((link, frame), turn) in aim.links.iter().zip(&frames).zip(&mut turns) {
                let barrel = aim.barrel(working)?;
                let now = LookFrame::new(barrel.direction, self.up)?;
                let needed = yaw_pitch_to(&now, target - barrel.point);
                let limits = link.limits;
                turn.yaw = (turn.yaw + link.part * needed.yaw).clamp(-limits.yaw, limits.yaw);
                turn.pitch =
                    (turn.pitch + link.part * needed.pitch).clamp(-limits.pitch, limits.pitch);
                let rotation = input.get(link.bone)?.rotation * look_rotation(frame, *turn);
                working.get_mut(link.bone)?.rotation = rotation;
            }
        }
        Some(
            frames
                .iter()
                .zip(&turns)
                .map(|(frame, turn)| look_rotation(frame, *turn))
                .collect(),
        )
    }
}

impl AnimationNode for AimNode {
    type State = AimState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> AimState {
        AimState {
            elapsed: 0.0,
            fresh: true,
            weight: Spring::critically_damped(0.0, 0.0),
            offsets: identity_offsets(self.chain.len()),
            working: Vec::new(),
            warned: false,
        }
    }

    fn update(
        &self,
        state: &mut AimState,
        delta_time: f32,
        _: &(),
        _: &mut AnimationGraphContext<'_>,
    ) {
        if delta_time.is_finite() && delta_time > 0.0 {
            state.elapsed += delta_time;
        }
    }

    fn evaluate(
        &self,
        state: &mut AimState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.frame);
        }
        let delta_time = state.take_time();
        if self.chain.is_empty() {
            return;
        }
        let aim = match self.resolve(context.skeleton()) {
            Ok(aim) => aim,
            Err(reason) => {
                if !state.warned {
                    warn!(
                        "aim toward `{}` skipped: {reason}",
                        self.target_param.0.as_str()
                    );
                    state.warned = true;
                }
                return;
            }
        };
        if state.offsets.len() != aim.links.len() {
            state.offsets = identity_offsets(aim.links.len());
        }
        let blackboard = context.blackboard();
        let target = self.target(context);
        let goal = target.map_or(0.0, |_| self.effective_weight(blackboard));
        let weight = ease(
            &mut state.weight,
            goal,
            halflife(self.weight_halflife),
            delta_time,
        )
        .clamp(0.0, 1.0);
        state.weight.value = weight;
        if goal <= 0.0 && weight < SNAP_WEIGHT {
            state.weight.reset_to(0.0);
            state.offsets_to_rest();
            return;
        }
        let solved = target.and_then(|target| {
            self.solve(
                &aim,
                output.joints(),
                target,
                AIM_PASSES,
                &mut state.working,
            )
        });
        match solved {
            Some(solved) => {
                let reacquire = self.reacquire_halflife(blackboard);
                for (offset, goal) in state.offsets.iter_mut().zip(solved) {
                    ease(offset, goal, reacquire, delta_time);
                }
            }
            None => state.hold_offsets(),
        }
        if weight <= 0.0 {
            return;
        }
        for (link, offset) in aim.links.iter().zip(&state.offsets) {
            if let Some(joint) = output.joint_mut(link.bone) {
                joint.rotation *= Quat::IDENTITY.slerp(offset.value, weight);
            }
        }
    }

    fn reset(&self, state: &mut AimState) {
        state.elapsed = 0.0;
        state.fresh = true;
        state.weight.reset_to(0.0);
        state.offsets_to_rest();
    }
}

impl EditorAnimationNode for AimNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
        node::{AnimationNode, EditorAnimationNode, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
    };
    use concerto_app::{App, schedule_groups::LateUpdate};
    use concerto_ecs::{Entity, entity::hierarchy::ChildOf, world::World};
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore},
        transform::GlobalTransform,
    };
    use concerto_mesh::skeleton::Skeleton;
    use concerto_name::{Name, name};
    use concerto_reflect::Reflect;
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{AimBone, AimNode, AimState, Muzzle};
    use crate::testing::{
        Frame, angle, app_with, assert_near, bone, captured, captured_graph, compile, input_event,
        insert_asset, read_state, skeleton_of, spawn_rigged, write_state,
    };

    const SPINE: usize = 1;
    const CHEST: usize = 2;
    const HAND: usize = 3;
    const ARM: usize = 4;
    const PROP: usize = 5;
    const LOW: usize = 1;
    const HIGH: usize = 2;
    const GUN: usize = 3;
    const MISS: f32 = 0.005;
    const DT: f32 = 1.0 / 60.0;
    const SETTLE_STEPS: usize = 300;
    const KICK: f32 = 0.05;

    fn spine_rig() -> Skeleton {
        skeleton_of(&[
            bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.1, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            bone(
                "chest",
                Some(1),
                Vec3::new(0.0, 0.25, 0.0),
                Quat::from_rotation_x(-0.1),
            ),
            bone(
                "hand",
                Some(2),
                Vec3::new(0.1, 0.0, 0.5),
                Quat::from_rotation_y(0.3),
            ),
            bone(
                "arm",
                Some(1),
                Vec3::new(0.2, 0.2, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
        ])
    }

    fn straight_rig() -> Skeleton {
        skeleton_of(&[
            bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            bone("low", Some(0), Vec3::new(0.0, 0.2, 0.0), Quat::IDENTITY),
            bone("high", Some(1), Vec3::new(0.0, 0.0, 0.25), Quat::IDENTITY),
            bone("gun", Some(2), Vec3::new(0.0, 0.0, 0.5), Quat::IDENTITY),
        ])
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct InputPose {
        rotations: Vec<Quat>,
    }

    impl AnimationNode for InputPose {
        type State = ();
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();
        type DriveInput = ();

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
            for (bone, rotation) in self.rotations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = *rotation;
                }
            }
            output.push_event(input_event());
        }

        fn reset(&self, _: &mut ()) {}
    }

    impl EditorAnimationNode for InputPose {}

    fn graph_of(node: AimNode, rotations: &[Quat]) -> AnimationGraph {
        captured_graph(
            node,
            InputPose {
                rotations: rotations.to_vec(),
            },
        )
    }

    struct Barrel {
        point: Vec3,
        direction: Vec3,
    }

    struct Rig {
        shape: fn() -> Skeleton,
        skeleton: Skeleton,
        muzzle: Muzzle,
        muzzle_bone: usize,
        first: usize,
        target: Option<Vec3>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                shape: spine_rig,
                skeleton: spine_rig(),
                muzzle: Muzzle::new("hand").with_direction(Quat::from_rotation_y(-0.3) * Vec3::Z),
                muzzle_bone: HAND,
                first: SPINE,
                target: None,
            }
        }

        fn straight() -> Self {
            Self {
                shape: straight_rig,
                skeleton: straight_rig(),
                muzzle: Muzzle::new("gun").with_offset(Vec3::new(0.0, 0.0, -0.5)),
                muzzle_bone: GUN,
                first: LOW,
                target: None,
            }
        }

        fn rotations(&self) -> Vec<Quat> {
            self.skeleton
                .rest_pose
                .iter()
                .map(|joint| joint.rotation)
                .collect()
        }

        fn input(&self) -> Vec<JointPose> {
            self.skeleton.rest_pose.to_vec()
        }

        fn model(&self, joints: &[JointPose], bone: usize) -> Mat4 {
            self.skeleton.forward_kinematics(joints).unwrap()[bone]
        }

        fn model_position(&self, joints: &[JointPose], bone: usize) -> Vec3 {
            self.model(joints, bone).w_axis.truncate()
        }

        fn model_rotation(&self, joints: &[JointPose], bone: usize) -> Quat {
            self.model(joints, bone).to_scale_rotation_translation().1
        }

        fn barrel(&self, joints: &[JointPose]) -> Barrel {
            let model = self.model(joints, self.muzzle_bone);
            Barrel {
                point: model.transform_point3(self.muzzle.offset),
                direction: model.transform_vector3(self.muzzle.direction).normalize(),
            }
        }

        fn miss(&self, joints: &[JointPose], target: Vec3) -> f32 {
            let barrel = self.barrel(joints);
            let reach = target - barrel.point;
            assert!(reach.dot(barrel.direction) > 0.0, "the target is behind");
            reach.reject_from_normalized(barrel.direction).length()
        }

        fn input_frame(&self, bone: usize) -> LookFrame {
            let inverse = self.model_rotation(&self.input(), bone).inverse();
            LookFrame::new(
                inverse * self.barrel(&self.input()).direction,
                inverse * Vec3::Y,
            )
            .unwrap()
        }

        fn turn(&self, joints: &[JointPose], bone: usize) -> YawPitch {
            let offset = self.input()[bone].rotation.inverse() * joints[bone].rotation;
            let frame = self.input_frame(bone);
            yaw_pitch_to(&frame, offset * frame.forward())
        }

        fn toward(&self, yaw: f32, pitch: f32, distance: f32) -> Vec3 {
            let input = self.input();
            let frame = LookFrame::new(self.barrel(&input).direction, Vec3::Y).unwrap();
            let direction = look_rotation(&frame, YawPitch { yaw, pitch }) * frame.forward();
            self.model_position(&input, self.first) + direction * distance
        }

        fn solve(&self, node: &AimNode) -> Frame {
            Stage::new(self, node.clone()).step(DT)
        }

        fn aimed(target: Vec3) -> Self {
            Self {
                target: Some(target),
                ..Self::new()
            }
        }

        fn kicked(&self) -> Vec<Quat> {
            let mut rotations = self.rotations();
            rotations[SPINE] *= Quat::from_rotation_x(-KICK);
            rotations
        }

        fn solve_directly(&self, node: &AimNode, passes: usize, target: Vec3) -> Vec<JointPose> {
            let aim = node.resolve(Some(&self.skeleton)).unwrap();
            let input = self.input();
            let offsets = node
                .solve(&aim, &input, target, passes, &mut Vec::new())
                .unwrap();
            let mut joints = input.clone();
            for (link, offset) in aim.links.iter().zip(offsets) {
                joints[link.bone].rotation = input[link.bone].rotation * offset;
            }
            joints
        }
    }

    struct Stage {
        app: App,
        node: AimNode,
        graph: AssetId,
        player: Entity,
    }

    impl Stage {
        fn new(rig: &Rig, node: AimNode) -> Self {
            let mut app = app_with::<InputPose>();
            let bone_count = rig.skeleton.bone_count();
            let world = app.main_mut().world_mut();
            let skeleton = insert_asset(world, AssetId::new(), (rig.shape)());
            let graph = insert_asset(
                world,
                AssetId::new(),
                graph_of(node.clone(), &rig.rotations()),
            );
            let mut player = AnimationPlayer::new(bone_count);
            player.set_paused(true);
            if let Some(target) = rig.target {
                player.set_param(name!("aim_target"), target);
            }
            let rigged = spawn_rigged(world, &skeleton, &graph, player, bone_count);
            let space = world.spawn(GlobalTransform::new(Mat4::IDENTITY));
            world.insert(ChildOf::new(space), rigged.bones[0]);
            compile(world);
            world.run_schedule(LateUpdate);
            world
                .get_component_for_entity_mut::<AnimationPlayer>(rigged.player)
                .unwrap()
                .set_paused(false);

            Self {
                app,
                node,
                graph: graph.id(),
                player: rigged.player,
            }
        }

        fn world(&mut self) -> &mut World {
            self.app.main_mut().world_mut()
        }

        fn set_param<T: Reflect>(&mut self, key: Name, value: T) {
            let player = self.player;
            self.world()
                .get_component_for_entity_mut::<AnimationPlayer>(player)
                .unwrap()
                .set_param(key, value);
        }

        fn forget_target(&mut self) {
            self.set_param(name!("aim_target"), 1.0_f32);
        }

        fn kick(&mut self, rotations: &[Quat]) {
            let graph = graph_of(self.node.clone(), rotations);
            let id = self.graph;
            self.world()
                .get_resource_mut::<AssetStore<AnimationGraph>>()
                .unwrap()
                .insert(id, graph);
        }

        fn write<R>(&mut self, write: impl FnOnce(&AimNode, &mut AimState) -> R) -> R {
            let player = self.player;
            write_state::<AimNode, _>(self.world(), player, write)
                .expect("the graph holds an aim node")
        }

        fn warned(&mut self) -> bool {
            let player = self.player;
            read_state::<AimNode, _>(self.world(), player, |_, state| state.warned)
                .expect("the graph holds an aim node")
        }

        fn step(&mut self, delta_time: f32) -> Frame {
            self.write(|_, state| state.elapsed += delta_time);
            self.world().run_schedule(LateUpdate);
            let player = self.player;
            captured(self.world(), player)
        }

        fn settle(&mut self) -> Frame {
            let mut frame = self.step(DT);
            for _ in 1..SETTLE_STEPS {
                frame = self.step(DT);
            }
            frame
        }
    }

    fn aim() -> AimNode {
        AimNode::new(name!("aim_target"), Rig::new().muzzle)
            .with_bone(AimBone::new("spine"))
            .with_bone(AimBone::new("chest"))
            .with_weight_halflife(0.0)
    }

    fn straight_aim(low: AimBone) -> AimNode {
        let wide = 90.0_f32.to_radians();
        AimNode::new(name!("aim_target"), Rig::straight().muzzle)
            .with_bone(low.with_share(1.0))
            .with_bone(AimBone::new("high").with_share(3.0).with_limits(wide, wide))
            .with_weight_halflife(0.0)
    }

    fn assert_unchanged(rig: &Rig, frame: &Frame) {
        assert_eq!(frame.joints, rig.input());
        assert_eq!(frame.events, [input_event()]);
    }

    fn assert_weighted(rig: &Rig, frame: &Frame, full: &Frame, weight: f32) {
        let input = rig.input();
        for bone in [SPINE, CHEST] {
            let expected = input[bone]
                .rotation
                .slerp(full.joints[bone].rotation, weight);
            let off = angle(frame.joints[bone].rotation, expected);
            assert!(off < 1e-3, "bone {bone} is {off} rad off weight {weight}");
        }
    }

    struct Aim {
        yaw: f32,
        pitch: f32,
        distance: f32,
    }

    fn parallax_cases() -> Vec<Aim> {
        let mut cases = Vec::new();
        for distance in [2.0, 3.0] {
            for yaw in [-30.0_f32, 0.0, 30.0] {
                for pitch in [-20.0_f32, 0.0, 20.0] {
                    cases.push(Aim {
                        yaw: yaw.to_radians(),
                        pitch: pitch.to_radians(),
                        distance,
                    });
                }
            }
        }
        cases
    }

    #[test]
    fn the_barrel_passes_within_5_mm_of_targets_at_2_and_3_m_where_one_pass_would_miss() {
        for case in parallax_cases() {
            let rig = Rig::new();
            let target = rig.toward(case.yaw, case.pitch, case.distance);
            let rig = Rig::aimed(target);

            let solved = rig.miss(&rig.solve(&aim()).joints, target);
            let once = rig.miss(&rig.solve_directly(&aim(), 1, target), target);

            let at = format!(
                "{} m at yaw {} and pitch {}",
                case.distance, case.yaw, case.pitch
            );
            assert!(solved < MISS, "{at}: missed by {solved} m");
            assert!(once > MISS, "{at}: one pass missed by only {once} m");
        }
    }

    #[test]
    fn each_bone_turns_by_its_share_when_the_barrel_crosses_every_pivot() {
        let turns = [
            YawPitch {
                yaw: 40.0_f32.to_radians(),
                pitch: 0.0,
            },
            YawPitch {
                yaw: -40.0_f32.to_radians(),
                pitch: 0.0,
            },
            YawPitch {
                yaw: 0.0,
                pitch: 30.0_f32.to_radians(),
            },
            YawPitch {
                yaw: 0.0,
                pitch: -30.0_f32.to_radians(),
            },
        ];
        let wide = 90.0_f32.to_radians();
        let node = straight_aim(AimBone::new("low").with_limits(wide, wide));

        for turn in turns {
            let mut rig = Rig::straight();
            rig.target = Some(rig.toward(turn.yaw, turn.pitch, 100.0));

            let frame = rig.solve(&node);

            let low = rig.turn(&frame.joints, LOW);
            let high = rig.turn(&frame.joints, HIGH);
            let total = YawPitch {
                yaw: low.yaw + high.yaw,
                pitch: low.pitch + high.pitch,
            };
            assert_near(total.yaw, turn.yaw, 0.5_f32.to_radians());
            assert_near(total.pitch, turn.pitch, 0.5_f32.to_radians());
            assert_near(low.yaw, 0.25 * total.yaw, 1e-3);
            assert_near(low.pitch, 0.25 * total.pitch, 1e-3);
            assert_near(high.yaw, 0.75 * total.yaw, 1e-3);
            assert_near(high.pitch, 0.75 * total.pitch, 1e-3);
        }
    }

    #[test]
    fn a_bone_held_at_its_limit_leaves_the_rest_to_the_next_until_every_bone_is_at_its_limits() {
        let mut rig = Rig::straight();
        let target = rig.toward(40.0_f32.to_radians(), 0.0, 100.0);
        rig.target = Some(target);
        let limit = 5.0_f32.to_radians();
        let node = straight_aim(AimBone::new("low").with_limits(limit, 90.0_f32.to_radians()));

        let frame = rig.solve(&node);

        assert_near(rig.turn(&frame.joints, LOW).yaw, limit, 1e-3);
        let barrel = rig.barrel(&frame.joints);
        let off = barrel.direction.angle_between(target - barrel.point);
        assert!(off < 1e-3, "the barrel points {off} rad off the target");

        let rig = Rig::aimed(Rig::new().toward(70.0_f32.to_radians(), 0.0, 3.0));
        let limit = 10.0_f32.to_radians();
        let mut node = aim();
        for link in &mut node.chain {
            link.yaw_limit = limit;
        }

        let frame = rig.solve(&node);

        for bone in [SPINE, CHEST] {
            let turn = rig.turn(&frame.joints, bone);
            assert_near(turn.yaw, limit, 1e-3);
            assert!(
                turn.pitch.abs() <= AimBone::default().pitch_limit + 1e-4,
                "bone {bone} pitched {}",
                turn.pitch
            );
        }
    }

    #[test]
    fn events_translations_and_other_bones_pass_through() {
        let rig = Rig::aimed(Rig::new().toward(0.3, 0.1, 2.0));
        let mut stage = Stage::new(&rig, aim());

        let frame = stage.step(DT);

        assert!(!stage.warned());
        let input = rig.input();
        assert_eq!(frame.events, [input_event()]);
        for (output, input) in frame.joints.iter().zip(&input) {
            assert_eq!(output.translation, input.translation);
            assert_eq!(output.scale, input.scale);
        }
        for bone in [0, HAND, ARM, PROP] {
            assert_eq!(frame.joints[bone], input[bone], "bone {bone}");
        }
        for bone in [SPINE, CHEST] {
            assert!(
                frame.joints[bone]
                    .rotation
                    .angle_between(input[bone].rotation)
                    > 1e-3,
                "bone {bone} did not turn"
            );
        }
    }

    struct Kick {
        node: AimNode,
        scale: Option<f32>,
        recovers_in: f32,
    }

    #[test]
    fn a_kick_between_frames_recovers_at_the_scaled_reacquire_halflife_or_at_once_without_one() {
        let rows = [
            Kick {
                node: aim()
                    .with_reacquire_halflife(0.05)
                    .with_reacquire_param(name!("reacquire_scale")),
                scale: Some(2.0),
                recovers_in: 0.1,
            },
            Kick {
                node: aim(),
                scale: None,
                recovers_in: 0.0,
            },
        ];
        let target = Rig::new().toward(0.3, 0.1, 2.0);
        let rig = Rig::aimed(target);

        for row in rows {
            let mut stage = Stage::new(&rig, row.node);
            if let Some(scale) = row.scale {
                stage.set_param(name!("reacquire_scale"), scale);
            }
            stage.settle();
            stage.kick(&rig.kicked());

            if row.recovers_in == 0.0 {
                let struck = stage.step(DT);
                assert_ne!(struck.joints[SPINE], rig.solve(&aim()).joints[SPINE]);
                let miss = rig.miss(&struck.joints, target);
                assert!(miss < MISS, "missed by {miss} m through the kick");
                continue;
            }
            let struck = stage.step(0.0);
            let recovering = stage.step(row.recovers_in);
            let recovered = stage.settle();

            assert!(
                rig.miss(&struck.joints, target) > MISS,
                "the kick did not move the barrel"
            );
            assert!(rig.miss(&recovered.joints, target) < MISS);
            for bone in [SPINE, CHEST] {
                let solved = recovered.joints[bone].rotation;
                let before = angle(struck.joints[bone].rotation, solved);
                let after = angle(recovering.joints[bone].rotation, solved);
                assert!(before > 5e-3, "bone {bone} was {before} rad off the solve");
                assert_near(after / before, 0.5, 0.01);
            }
        }
    }

    #[test]
    fn the_weight_eases_in_and_out_at_its_halflife() {
        let halflife = 0.1;
        let rig = Rig::new();
        let target = rig.toward(0.3, 0.1, 2.0);
        let full = Rig::aimed(target).solve(&aim());
        let mut stage = Stage::new(&rig, aim().with_weight_halflife(halflife));
        assert_unchanged(&rig, &stage.settle());

        stage.set_param(name!("aim_target"), target);
        let easing_in = stage.step(halflife);
        assert_weighted(&rig, &easing_in, &full, 0.5);

        stage.settle();
        stage.forget_target();
        let easing_out = stage.step(halflife);
        assert_weighted(&rig, &easing_out, &full, 0.5);
    }

    #[test]
    fn the_weight_param_scales_the_weight_within_zero_and_one() {
        let rig = Rig::aimed(Rig::new().toward(0.3, 0.1, 2.0));
        let full = rig.solve(&aim());
        let node = aim()
            .with_weight(0.8)
            .with_weight_param(name!("aim_weight"));

        for scale in [0.5_f32, 3.0] {
            let mut stage = Stage::new(&rig, node.clone());
            stage.set_param(name!("aim_weight"), scale);

            let frame = stage.step(DT);

            assert_weighted(&rig, &frame, &full, (0.8 * scale).min(1.0));
        }
    }

    #[test]
    fn a_target_gone_missing_eases_the_aim_out_then_leaves_the_input_bit_for_bit() {
        let rig = Rig::aimed(Rig::new().toward(0.3, 0.1, 2.0));
        let mut stage = Stage::new(&rig, aim().with_weight_halflife(0.05));
        let aimed = stage.settle();
        assert_ne!(aimed.joints, rig.input());
        stage.forget_target();

        let easing = stage.step(DT);
        let settled = stage.settle();

        assert_ne!(easing.joints, rig.input());
        assert_unchanged(&rig, &settled);
    }
}
