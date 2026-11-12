use std::f32::consts::PI;

use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint, parent},
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};
use concerto_foundation::math::Spring;
use concerto_mesh::{JointName, Skeleton};
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

const MAX_ANGLE_CAP: f32 = 170.0 * PI / 180.0;
const SNAP_WEIGHT: f32 = 1.0e-4;

/// Turns a chain of bones, then each eye, toward a world-space point; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct LookAtNode {
    pub chain: Vec<LookAtBone>,
    pub eyes: Vec<LookAtEye>,
    pub forward: Vec3,
    pub up: Vec3,
    pub target_param: Param<Vec3>,
    pub weight: f32,
    pub weight_param: Option<Param<f32>>,
    pub weight_halflife: f32,
    pub max_angle: f32,
}

impl Default for LookAtNode {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            eyes: Vec::new(),
            forward: Vec3::Z,
            up: Vec3::Y,
            target_param: Param::default(),
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
            max_angle: 2.0 * PI / 3.0,
        }
    }
}

/// A bone of the look-at chain: its share of the turn, its limits in radians, and the halflife of its offset.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct LookAtBone {
    pub bone: JointName,
    pub share: f32,
    pub yaw_limit: f32,
    pub pitch_limit: f32,
    pub halflife: f32,
}

impl Default for LookAtBone {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            share: 1.0,
            yaw_limit: 60.0_f32.to_radians(),
            pitch_limit: 30.0_f32.to_radians(),
            halflife: 0.15,
        }
    }
}

/// An eye: aimed on its own at the target, from its own position, within its limits.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct LookAtEye {
    pub bone: JointName,
    pub yaw_limit: f32,
    pub pitch_limit: f32,
    pub halflife: f32,
}

impl Default for LookAtEye {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            yaw_limit: 30.0_f32.to_radians(),
            pitch_limit: 20.0_f32.to_radians(),
            halflife: 0.03,
        }
    }
}

/// The eased weight and turn of each chain bone and then each eye, the time since the last evaluate, the pose the turns are solved on, and whether a [`LookAtNode`] has already warned about bones it cannot resolve.
#[derive(Debug)]
pub struct LookAtState {
    elapsed: f32,
    fresh: bool,
    weight: Spring<f32>,
    offsets: Vec<Spring<Quat>>,
    working: Vec<JointPose>,
    warned: bool,
}

impl LookAtState {
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
}

fn identity_offsets(count: usize) -> Vec<Spring<Quat>> {
    vec![Spring::critically_damped(Quat::IDENTITY, 0.0); count]
}

struct LookBone {
    bone: usize,
    frame: LookFrame,
    part: f32,
    limits: YawPitch,
}

struct ResolvedLook<'a> {
    skeleton: &'a Skeleton,
    bones: Vec<LookBone>,
}

impl LookAtNode {
    fn entry_count(&self) -> usize {
        self.chain.len() + self.eyes.len()
    }

    fn halflives(&self) -> impl Iterator<Item = f32> + '_ {
        self.chain
            .iter()
            .map(|link| link.halflife)
            .chain(self.eyes.iter().map(|eye| eye.halflife))
            .map(halflife)
    }

    fn effective_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
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

    fn max_angle(&self) -> f32 {
        if self.max_angle > 0.0 {
            self.max_angle.min(MAX_ANGLE_CAP)
        } else {
            0.0
        }
    }

    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedLook<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let shares: Vec<f32> = self.chain.iter().map(|link| share(link.share)).collect();
        let mut bones = Vec::with_capacity(self.entry_count());
        let mut last = None;
        for (index, link) in self.chain.iter().enumerate() {
            let bone = unique_bone(skeleton, &bones, link.bone)?;
            if let Some(above) = last
                && !descends(skeleton, bone, above)
            {
                return Err(format!(
                    "chain bone `{}` does not descend from the chain bone before it",
                    &*link.bone
                ));
            }
            let left: f32 = shares[index..].iter().sum();
            bones.push(LookBone {
                bone,
                frame: self.look_frame(skeleton, bone)?,
                part: if left > 0.0 {
                    shares[index] / left
                } else {
                    0.0
                },
                limits: limits(link.yaw_limit, link.pitch_limit),
            });
            last = Some(bone);
        }
        let first_eye = bones.len();
        for eye in &self.eyes {
            let bone = unique_bone(skeleton, &bones, eye.bone)?;
            if let Some(head) = last
                && !descends(skeleton, bone, head)
            {
                return Err(format!(
                    "eye `{}` does not descend from the last chain bone",
                    &*eye.bone
                ));
            }
            if bones[first_eye..].iter().any(|other| {
                descends(skeleton, bone, other.bone) || descends(skeleton, other.bone, bone)
            }) {
                return Err(format!(
                    "eye `{}` and another eye descend one from the other",
                    &*eye.bone
                ));
            }
            bones.push(LookBone {
                bone,
                frame: self.look_frame(skeleton, bone)?,
                part: 1.0,
                limits: limits(eye.yaw_limit, eye.pitch_limit),
            });
        }
        Ok(ResolvedLook { skeleton, bones })
    }

    fn look_frame(&self, skeleton: &Skeleton, bone: usize) -> Result<LookFrame, String> {
        let rest = model_joint(skeleton, &skeleton.rest_pose, bone)
            .ok_or("the skeleton's rest pose is missing a bone")?;
        let inverse = rest.rotation.inverse();
        LookFrame::new(inverse * self.forward, inverse * self.up)
            .ok_or_else(|| "`forward` or `up` is zero or not finite, or they are parallel".into())
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

    fn goal_weight(
        &self,
        look: &ResolvedLook<'_>,
        joints: &[JointPose],
        target: Vec3,
        blackboard: &AnimationBlackboard,
    ) -> f32 {
        let Some(first) = look.bones.first() else {
            return 0.0;
        };
        let Some(joint) = model_joint(look.skeleton, joints, first.bone) else {
            return 0.0;
        };
        let gaze = joint.rotation * first.frame.forward();
        let angle = gaze.angle_between(target - Vec3::from(joint.transform.translation));
        if angle > self.max_angle() {
            0.0
        } else {
            self.effective_weight(blackboard)
        }
    }
}

pub(crate) fn share(share: f32) -> f32 {
    if share.is_finite() && share > 0.0 {
        share
    } else {
        0.0
    }
}

pub(crate) fn halflife(halflife: f32) -> f32 {
    if halflife.is_finite() && halflife > 0.0 {
        halflife
    } else {
        0.0
    }
}

pub(crate) fn limits(yaw: f32, pitch: f32) -> YawPitch {
    let limit = |limit: f32| if limit > 0.0 { limit } else { 0.0 };
    YawPitch {
        yaw: limit(yaw),
        pitch: limit(pitch),
    }
}

fn unique_bone(skeleton: &Skeleton, taken: &[LookBone], name: JointName) -> Result<usize, String> {
    let bone = bone_under_first(skeleton, name)?;
    if taken.iter().any(|look| look.bone == bone) {
        return Err(format!("bone `{}` appears twice", &*name));
    }
    Ok(bone)
}

pub(crate) fn descends(skeleton: &Skeleton, bone: usize, ancestor: usize) -> bool {
    let mut current = bone;
    while let Some(up) = parent(skeleton, current) {
        if up == ancestor {
            return true;
        }
        current = up;
    }
    false
}

fn solve(
    look: &ResolvedLook<'_>,
    target: Vec3,
    offsets: &mut [Spring<Quat>],
    delta_time: f32,
    working: &mut [JointPose],
) {
    for (look_bone, offset) in look.bones.iter().zip(offsets.iter_mut()) {
        let Some(joint) = model_joint(look.skeleton, working, look_bone.bone) else {
            continue;
        };
        let direction =
            joint.rotation.inverse() * (target - Vec3::from(joint.transform.translation));
        let angles = yaw_pitch_to(&look_bone.frame, direction);
        let limits = look_bone.limits;
        let goal = look_rotation(
            &look_bone.frame,
            YawPitch {
                yaw: (look_bone.part * angles.yaw).clamp(-limits.yaw, limits.yaw),
                pitch: (look_bone.part * angles.pitch).clamp(-limits.pitch, limits.pitch),
            },
        );
        let turn = offset.update(goal, delta_time);
        if let Some(local) = working.get_mut(look_bone.bone) {
            local.rotation = (local.rotation * turn).normalize();
        }
    }
}

impl AnimationNode for LookAtNode {
    type State = LookAtState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> LookAtState {
        LookAtState {
            elapsed: 0.0,
            fresh: true,
            weight: Spring::critically_damped(0.0, 0.0),
            offsets: identity_offsets(self.entry_count()),
            working: Vec::new(),
            warned: false,
        }
    }

    fn update(
        &self,
        state: &mut LookAtState,
        delta_time: f32,
        _: &(),
        _: &AnimationGraphContext<'_>,
    ) {
        if delta_time.is_finite() && delta_time > 0.0 {
            state.elapsed += delta_time;
        }
    }

    fn evaluate(
        &self,
        state: &mut LookAtState,
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
        if self.entry_count() == 0 {
            return;
        }
        let look = match self.resolve(context.skeleton()) {
            Ok(look) => look,
            Err(reason) => {
                if !state.warned {
                    warn!(
                        "look-at toward `{}` skipped: {reason}",
                        self.target_param.0.as_str()
                    );
                    state.warned = true;
                }
                return;
            }
        };
        if state.offsets.len() != look.bones.len() {
            state.offsets = identity_offsets(look.bones.len());
        }
        state.weight.set_halflife(halflife(self.weight_halflife));
        for (offset, seconds) in state.offsets.iter_mut().zip(self.halflives()) {
            offset.set_halflife(seconds);
        }
        let target = self.target(context);
        let goal = target.map_or(0.0, |target| {
            self.goal_weight(&look, output.joints(), target, context.blackboard())
        });
        let weight = state.weight.update(goal, delta_time).clamp(0.0, 1.0);
        state.weight.value = weight;
        if goal <= 0.0 && weight < SNAP_WEIGHT {
            state.weight.reset_to(0.0);
            state.offsets_to_rest();
            return;
        }
        match target {
            Some(target) => {
                state.working.clear();
                state.working.extend_from_slice(output.joints());
                solve(
                    &look,
                    target,
                    &mut state.offsets,
                    delta_time,
                    &mut state.working,
                );
            }
            None => {
                for offset in &mut state.offsets {
                    let held = offset.value;
                    offset.reset_to(held);
                }
            }
        }
        if weight <= 0.0 {
            return;
        }
        for (look_bone, offset) in look.bones.iter().zip(&state.offsets) {
            if let Some(joint) = output.joint_mut(look_bone.bone) {
                joint.rotation =
                    (joint.rotation * Quat::IDENTITY.slerp(offset.value, weight)).normalize();
            }
        }
    }

    fn reset(&self, state: &mut LookAtState) {
        state.elapsed = 0.0;
        state.fresh = true;
        state.weight.reset_to(0.0);
        state.offsets_to_rest();
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        look_at::{LookFrame, YawPitch, yaw_pitch_to},
        node::{AnimationNode, ReflectAnimationNode},
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
    use glam::{Affine3A, Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{LookAtBone, LookAtEye, LookAtNode, LookAtState};
    use crate::testing::{
        Frame, app_with, assert_near, bone, captured, captured_graph, compile, input_event,
        input_motion, insert_asset, read_state, skeleton_of, spawn_rigged, write_state,
    };

    const HIPS: usize = 0;
    const SPINE: usize = 1;
    const NECK: usize = 2;
    const HEAD: usize = 3;
    const EYE_L: usize = 4;
    const EYE_R: usize = 5;
    const LID: usize = 6;
    const ARM: usize = 7;
    const PROP: usize = 8;
    const PROP_CHILD: usize = 9;
    const CHAIN: [usize; 3] = [SPINE, NECK, HEAD];
    const EYES: [usize; 2] = [EYE_L, EYE_R];
    const DT: f32 = 1.0 / 60.0;
    const SETTLE_STEPS: usize = 300;

    fn skeleton() -> Skeleton {
        skeleton_of(&[
            bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.2, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            bone(
                "neck",
                Some(1),
                Vec3::new(0.0, 0.4, 0.0),
                Quat::from_rotation_z(0.15),
            ),
            bone(
                "head",
                Some(2),
                Vec3::new(0.0, 0.15, 0.0),
                Quat::from_rotation_y(-0.2),
            ),
            bone(
                "eye_l",
                Some(3),
                Vec3::new(0.03, 0.08, 0.03),
                Quat::from_rotation_x(0.05),
            ),
            bone(
                "eye_r",
                Some(3),
                Vec3::new(-0.03, 0.08, 0.03),
                Quat::from_rotation_x(-0.05),
            ),
            bone(
                "lid",
                Some(4),
                Vec3::new(0.0, 0.01, 0.01),
                Quat::from_rotation_z(0.1),
            ),
            bone(
                "arm",
                Some(1),
                Vec3::new(0.2, 0.35, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
            bone("prop_child", Some(8), Vec3::Y, Quat::from_rotation_y(0.2)),
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
            for (bone, rotation) in self.rotations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = *rotation;
                }
            }
            output.set_root_motion(input_motion());
            output.push_event(input_event());
        }

        fn reset(&self, _: &mut ()) {}
    }

    fn graph_of(node: LookAtNode, rotations: &[Quat]) -> AnimationGraph {
        captured_graph(
            node,
            InputPose {
                rotations: rotations.to_vec(),
            },
        )
    }

    struct FloatParam {
        name: Name,
        value: f32,
    }

    struct Rig {
        skeleton: Skeleton,
        rotations: Vec<Quat>,
        model_to_world: Affine3A,
        target: Option<Vec3>,
        floats: Vec<FloatParam>,
    }

    impl Rig {
        fn new() -> Self {
            let skeleton = skeleton();
            Self {
                rotations: skeleton
                    .rest_pose
                    .iter()
                    .map(|joint| joint.rotation)
                    .collect(),
                skeleton,
                model_to_world: Affine3A::IDENTITY,
                target: None,
                floats: Vec::new(),
            }
        }

        fn input(&self) -> Vec<JointPose> {
            self.skeleton
                .rest_pose
                .iter()
                .zip(&self.rotations)
                .map(|(joint, rotation)| JointPose {
                    rotation: *rotation,
                    ..joint.clone()
                })
                .collect()
        }

        fn set_float(&mut self, name: Name, value: f32) {
            self.floats.push(FloatParam { name, value });
        }

        fn solve(&self, node: &LookAtNode) -> Frame {
            Stage::start(self, node.clone()).step(DT)
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

        fn look_frame(&self, bone: usize) -> LookFrame {
            let rest = self.model_rotation(&self.skeleton.rest_pose, bone);
            LookFrame::new(rest.inverse() * Vec3::Z, rest.inverse() * Vec3::Y).unwrap()
        }

        fn gaze(&self, joints: &[JointPose], bone: usize) -> Vec3 {
            self.model_rotation(joints, bone) * self.look_frame(bone).forward()
        }

        fn turn(&self, frame: &Frame, bone: usize) -> YawPitch {
            let offset = self.input()[bone].rotation.inverse() * frame.joints[bone].rotation;
            let look = self.look_frame(bone);
            yaw_pitch_to(&look, offset * look.forward())
        }

        fn turned(&self, frame: &Frame, bone: usize) -> f32 {
            frame.joints[bone]
                .rotation
                .angle_between(self.input()[bone].rotation)
        }

        fn toward(&self, bone: usize, yaw: f32, pitch: f32, distance: f32) -> Vec3 {
            let direction = Vec3::new(
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            self.model_position(&self.input(), bone) + direction * distance
        }

        fn aim(&mut self, model_target: Vec3) {
            self.target = Some(self.model_to_world.transform_point3(model_target));
        }
    }

    struct Stage {
        app: App,
        rotations: Vec<Quat>,
        node: LookAtNode,
        graph: AssetId,
        player: Entity,
        space: Entity,
    }

    impl Stage {
        fn new(rig: &Rig, node: LookAtNode) -> Self {
            let mut app = app_with::<InputPose>();
            let bone_count = rig.skeleton.bone_count();
            let world = app.main_mut().world_mut();
            let skeleton = insert_asset(world, AssetId::new(), skeleton());
            let graph = insert_asset(
                world,
                AssetId::new(),
                graph_of(node.clone(), &rig.rotations),
            );
            let mut player = AnimationPlayer::new(bone_count);
            player.set_paused(true);
            for float in &rig.floats {
                player.set_param(float.name, float.value);
            }
            if let Some(target) = rig.target {
                player.set_param(name!("look_target"), target);
            }
            let rigged = spawn_rigged(world, &skeleton, &graph, player, bone_count);
            let space = world.spawn(GlobalTransform::new(Mat4::from(rig.model_to_world)));
            world.insert(ChildOf::new(space), rigged.bones[0]);
            compile(world);
            world.run_schedule(LateUpdate);
            world
                .get_component_for_entity_mut::<AnimationPlayer>(rigged.player)
                .unwrap()
                .set_paused(false);

            Self {
                app,
                rotations: rig.rotations.clone(),
                node,
                graph: graph.id(),
                player: rigged.player,
                space,
            }
        }

        fn start(rig: &Rig, node: LookAtNode) -> Self {
            let mut stage = Self::new(rig, node);
            stage.step(0.0);
            stage
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

        fn follow(&mut self, rig: &Rig) {
            if let Some(target) = rig.target {
                self.set_param(name!("look_target"), target);
            }
        }

        fn forget_target(&mut self) {
            let node = LookAtNode {
                target_param: name!("no_target").into(),
                ..self.node.clone()
            };
            self.set_node(node);
        }

        fn set_space(&mut self, space: Affine3A) {
            let entity = self.space;
            self.world()
                .get_component_for_entity_mut::<GlobalTransform>(entity)
                .unwrap()
                .set_matrix(Mat4::from(space));
        }

        fn set_node(&mut self, node: LookAtNode) {
            let graph = graph_of(node.clone(), &self.rotations);
            let id = self.graph;
            self.world()
                .get_resource_mut::<AssetStore<AnimationGraph>>()
                .unwrap()
                .insert(id, graph);
            self.node = node;
        }

        fn read<R>(&mut self, read: impl FnOnce(&LookAtState) -> R) -> R {
            let player = self.player;
            read_state::<LookAtNode, _>(self.world(), player, |_, state| read(state))
                .expect("the graph holds a look-at node")
        }

        fn write<R>(&mut self, write: impl FnOnce(&LookAtNode, &mut LookAtState) -> R) -> R {
            let player = self.player;
            write_state::<LookAtNode, _>(self.world(), player, write)
                .expect("the graph holds a look-at node")
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

    fn link(name: &str) -> LookAtBone {
        LookAtBone {
            bone: name.into(),
            halflife: 0.0,
            ..LookAtBone::default()
        }
    }

    fn links(names: &[&str]) -> Vec<LookAtBone> {
        names.iter().map(|name| link(name)).collect()
    }

    fn eye(name: &str) -> LookAtEye {
        LookAtEye {
            bone: name.into(),
            halflife: 0.0,
            ..LookAtEye::default()
        }
    }

    fn look() -> LookAtNode {
        LookAtNode {
            chain: links(&["spine", "neck", "head"]),
            target_param: name!("look_target").into(),
            weight_halflife: 0.0,
            ..LookAtNode::default()
        }
    }

    fn with_eyes() -> LookAtNode {
        LookAtNode {
            eyes: vec![eye("eye_l"), eye("eye_r")],
            ..look()
        }
    }

    fn eased() -> LookAtNode {
        let mut node = with_eyes();
        for link in &mut node.chain {
            link.halflife = LookAtBone::default().halflife;
        }
        for eye in &mut node.eyes {
            eye.halflife = LookAtEye::default().halflife;
        }
        node.weight_halflife = LookAtNode::default().weight_halflife;
        node
    }

    fn snapping(halflife: f32) -> LookAtNode {
        let mut node = with_eyes();
        node.weight_halflife = halflife;
        for link in &mut node.chain {
            link.halflife = halflife;
        }
        for eye in &mut node.eyes {
            eye.halflife = halflife;
        }
        node
    }

    fn shared(shares: [f32; 3], limit: f32) -> LookAtNode {
        let mut node = look();
        for (link, share) in node.chain.iter_mut().zip(shares) {
            link.share = share;
            link.yaw_limit = limit;
            link.pitch_limit = limit;
        }
        node
    }

    fn assert_gaze(rig: &Rig, frame: &Frame, bone: usize, target: Vec3) {
        let direction = target - rig.model_position(&frame.joints, bone);
        let angle = rig.gaze(&frame.joints, bone).angle_between(direction);
        assert!(angle < 1e-3, "bone {bone} looks {angle} rad off the target");
    }

    fn assert_rotation(actual: Quat, expected: Quat) {
        assert!(
            actual.angle_between(expected) < 1e-3,
            "expected rotation {expected}, got {actual}"
        );
    }

    fn assert_unchanged(rig: &Rig, frame: &Frame) {
        assert_eq!(frame.joints, rig.input());
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
    }

    fn turned_world() -> Affine3A {
        Affine3A::from_rotation_translation(Quat::from_rotation_y(1.1), Vec3::new(3.0, 0.0, -2.0))
    }

    fn named(name: &str, check: impl FnOnce()) {
        let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
        assert!(checked.is_ok(), "{name}");
    }

    struct InputTurn {
        bone: usize,
        turn: Quat,
    }

    struct Gaze {
        name: &'static str,
        model_to_world: Affine3A,
        input: Vec<InputTurn>,
        node: LookAtNode,
        target: fn(&Rig) -> Vec3,
        gazing: &'static [usize],
    }

    #[test]
    fn each_gazing_bone_looks_at_a_target_in_reach_from_any_input_pose_and_model_space() {
        let head = |rig: &Rig| rig.toward(HEAD, 0.4, 0.2, 2.0);
        let plain = |name, node, gazing| Gaze {
            name,
            model_to_world: Affine3A::IDENTITY,
            input: Vec::new(),
            node,
            target: head,
            gazing,
        };
        let animated = |name, spine, head| Gaze {
            input: vec![
                InputTurn {
                    bone: SPINE,
                    turn: spine,
                },
                InputTurn {
                    bone: HEAD,
                    turn: head,
                },
            ],
            target: |rig| rig.toward(HEAD, 0.3, 0.1, 2.0),
            ..plain(name, look(), &[HEAD])
        };
        let rows = [
            plain("the head in reach", look(), &[HEAD]),
            Gaze {
                model_to_world: turned_world(),
                target: |rig| rig.toward(HEAD, -0.3, 0.15, 3.0),
                ..plain("the head under a turned model", look(), &[HEAD])
            },
            animated(
                "the head over a turned spine and pitched head",
                Quat::from_rotation_y(0.3),
                Quat::from_rotation_x(-0.2),
            ),
            animated(
                "the head over a rolled spine and turned head",
                Quat::from_rotation_z(-0.25),
                Quat::from_rotation_y(-0.4),
            ),
            Gaze {
                input: vec![InputTurn {
                    bone: EYE_L,
                    turn: Quat::from_rotation_y(0.1),
                }],
                target: |rig| {
                    let input = rig.input();
                    let between = (rig.model_position(&input, EYE_L)
                        + rig.model_position(&input, EYE_R))
                        * 0.5;
                    between + rig.gaze(&input, HEAD) * 0.6
                },
                ..plain("each eye from its own position", with_eyes(), &EYES)
            },
            plain(
                "a negative halflife snaps",
                snapping(-1.0),
                &[HEAD, EYE_L, EYE_R],
            ),
            plain(
                "a nan halflife snaps",
                snapping(f32::NAN),
                &[HEAD, EYE_L, EYE_R],
            ),
            plain(
                "an infinite halflife snaps",
                snapping(f32::INFINITY),
                &[HEAD, EYE_L, EYE_R],
            ),
        ];

        for row in rows {
            let mut rig = Rig::new();
            rig.model_to_world = row.model_to_world;
            for input in &row.input {
                rig.rotations[input.bone] *= input.turn;
            }
            let target = (row.target)(&rig);
            rig.aim(target);

            let frame = rig.solve(&row.node);

            named(row.name, || {
                for bone in row.gazing {
                    assert_gaze(&rig, &frame, *bone, target);
                }
            });
        }
    }

    struct Share {
        name: &'static str,
        node: LookAtNode,
        target: fn(&Rig) -> Vec3,
        check: fn(&Rig, &Frame, Vec3),
    }

    #[test]
    fn each_bone_takes_its_share_of_the_turn_up_to_its_limit_and_leaves_the_rest_to_the_next() {
        let wide = 90.0_f32.to_radians();
        let far = |rig: &Rig| rig.toward(NECK, 60.0_f32.to_radians(), 0.0, 100.0);
        let near = |rig: &Rig| rig.toward(NECK, 0.5, 0.2, 3.0);
        let neck = |share: f32, limit: f32| {
            let mut node = look();
            node.chain[1].share = share;
            node.chain[1].yaw_limit = limit;
            node.chain[1].pitch_limit = limit;
            node
        };
        let neck_held_head_gazing: fn(&Rig, &Frame, Vec3) = |rig, frame, target| {
            assert!(rig.turned(frame, NECK) < 1e-3);
            assert_gaze(rig, frame, HEAD, target);
        };
        let narrow = |name: &str| LookAtEye {
            yaw_limit: 0.02,
            pitch_limit: 0.01,
            ..eye(name)
        };
        let rows = [
            Share {
                name: "shares of 0.1, 0.3 and 0.6",
                node: shared([0.1, 0.3, 0.6], wide),
                target: far,
                check: |rig, frame, _| {
                    let yaws = CHAIN.map(|bone| rig.turn(frame, bone).yaw);
                    let total: f32 = yaws.iter().sum();
                    for (yaw, share) in yaws.iter().zip([0.1, 0.3, 0.6]) {
                        assert_near(yaw / total, share, 1e-2);
                    }
                    assert_near(total, 60.0_f32.to_radians(), 0.5_f32.to_radians());
                },
            },
            Share {
                name: "a neck clamped at 5 degrees",
                node: {
                    let mut node = shared([0.1, 0.3, 0.6], wide);
                    node.chain[1].yaw_limit = 5.0_f32.to_radians();
                    node
                },
                target: far,
                check: |rig, frame, target| {
                    assert_near(rig.turn(frame, NECK).yaw, 5.0_f32.to_radians(), 1e-3);
                    assert_gaze(rig, frame, HEAD, target);
                },
            },
            Share {
                name: "a target past every limit",
                node: LookAtNode {
                    max_angle: 170.0_f32.to_radians(),
                    ..shared([1.0, 1.0, 1.0], 10.0_f32.to_radians())
                },
                target: |rig| rig.toward(NECK, 90.0_f32.to_radians(), 0.0, 100.0),
                check: |rig, frame, _| {
                    for bone in CHAIN {
                        assert_near(rig.turn(frame, bone).yaw, 10.0_f32.to_radians(), 1e-3);
                    }
                },
            },
            Share {
                name: "a negative share",
                node: neck(-1.0, LookAtBone::default().yaw_limit),
                target: near,
                check: neck_held_head_gazing,
            },
            Share {
                name: "a nan share",
                node: neck(f32::NAN, LookAtBone::default().yaw_limit),
                target: near,
                check: neck_held_head_gazing,
            },
            Share {
                name: "a negative limit",
                node: neck(LookAtBone::default().share, -0.5),
                target: near,
                check: neck_held_head_gazing,
            },
            Share {
                name: "a nan limit",
                node: neck(LookAtBone::default().share, f32::NAN),
                target: near,
                check: neck_held_head_gazing,
            },
            Share {
                name: "zero shares",
                node: shared([0.0, 0.0, 0.0], 1.0),
                target: near,
                check: |rig, frame, _| {
                    for bone in CHAIN {
                        assert!(rig.turned(frame, bone) < 1e-3, "bone {bone}");
                    }
                },
            },
            Share {
                name: "eyes that cannot reach",
                node: LookAtNode {
                    chain: Vec::new(),
                    eyes: vec![narrow("eye_l"), narrow("eye_r")],
                    ..look()
                },
                target: |rig| rig.toward(HEAD, 0.8, 0.4, 5.0),
                check: |rig, frame, _| {
                    for bone in EYES {
                        let turn = rig.turn(frame, bone);
                        assert_near(turn.yaw, 0.02, 1e-4);
                        assert_near(turn.pitch, 0.01, 1e-4);
                    }
                },
            },
        ];

        for row in rows {
            let mut rig = Rig::new();
            let target = (row.target)(&rig);
            rig.aim(target);

            let frame = rig.solve(&row.node);

            named(row.name, || (row.check)(&rig, &frame, target));
        }
    }

    fn aimed(bone: usize, yaw: f32, pitch: f32, distance: f32) -> Rig {
        let mut rig = Rig::new();
        rig.aim(rig.toward(bone, yaw, pitch, distance));
        rig
    }

    fn at_head() -> Rig {
        aimed(HEAD, 0.4, 0.1, 2.0)
    }

    struct Untouched {
        name: &'static str,
        node: LookAtNode,
        rig: fn() -> Rig,
    }

    #[test]
    fn a_look_with_nothing_it_can_look_at_leaves_the_input_pose() {
        let unresolvable = |name, node| Untouched {
            name,
            node,
            rig: at_head,
        };
        let rows = [
            unresolvable(
                "a missing chain bone",
                LookAtNode {
                    chain: links(&["spine", "missing", "head"]),
                    ..look()
                },
            ),
            unresolvable(
                "a chain out of order",
                LookAtNode {
                    chain: links(&["neck", "spine", "head"]),
                    ..look()
                },
            ),
            unresolvable(
                "an eye off the chain",
                LookAtNode {
                    eyes: vec![eye("arm")],
                    ..look()
                },
            ),
            unresolvable(
                "an eye under an eye",
                LookAtNode {
                    eyes: vec![eye("eye_l"), eye("lid")],
                    ..look()
                },
            ),
            unresolvable(
                "the same eye twice",
                LookAtNode {
                    eyes: vec![eye("eye_l"), eye("eye_l")],
                    ..look()
                },
            ),
            unresolvable(
                "a chain bone outside the hierarchy",
                LookAtNode {
                    chain: links(&["prop_child"]),
                    ..look()
                },
            ),
            unresolvable(
                "a zero forward",
                LookAtNode {
                    forward: Vec3::ZERO,
                    ..look()
                },
            ),
            unresolvable(
                "an up along forward",
                LookAtNode {
                    up: Vec3::Z * 2.0,
                    ..look()
                },
            ),
            unresolvable(
                "no bones",
                LookAtNode {
                    chain: Vec::new(),
                    ..look()
                },
            ),
            unresolvable(
                "weight zero",
                LookAtNode {
                    weight: 0.0,
                    ..with_eyes()
                },
            ),
            Untouched {
                name: "a weight param of zero",
                node: LookAtNode {
                    weight_param: Some(name!("look_weight").into()),
                    ..with_eyes()
                },
                rig: || {
                    let mut rig = at_head();
                    rig.set_float(name!("look_weight"), 0.0);
                    rig
                },
            },
            Untouched {
                name: "no target",
                node: with_eyes(),
                rig: Rig::new,
            },
            Untouched {
                name: "a target that is not a position",
                node: with_eyes(),
                rig: || {
                    let mut rig = Rig::new();
                    rig.set_float(name!("look_target"), 1.0);
                    rig
                },
            },
            Untouched {
                name: "a model to world that cannot be inverted",
                node: with_eyes(),
                rig: || {
                    let mut rig = Rig::new();
                    rig.model_to_world = Affine3A::from_scale(Vec3::ZERO);
                    rig.target = Some(Vec3::new(0.3, 1.5, 2.0));
                    rig
                },
            },
            Untouched {
                name: "a nan max angle",
                node: LookAtNode {
                    max_angle: f32::NAN,
                    ..look()
                },
                rig: || aimed(SPINE, 0.2, 0.0, 3.0),
            },
            Untouched {
                name: "a max angle past 170 degrees",
                node: LookAtNode {
                    max_angle: 4.0,
                    ..look()
                },
                rig: || aimed(SPINE, 175.0_f32.to_radians(), 0.0, 3.0),
            },
            Untouched {
                name: "a target past max angle",
                node: look(),
                rig: || aimed(SPINE, 125.0_f32.to_radians(), 0.0, 3.0),
            },
        ];

        for row in rows {
            let rig = (row.rig)();

            let frame = rig.solve(&row.node);

            named(row.name, || assert_unchanged(&rig, &frame));
        }

        let inside = aimed(SPINE, 115.0_f32.to_radians(), 0.0, 3.0);
        let frame = inside.solve(&look());
        assert!(
            CHAIN
                .into_iter()
                .any(|bone| inside.turned(&frame, bone) > 1e-3)
        );
    }

    #[test]
    fn root_motion_events_translations_scales_and_other_bones_pass_through() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 0.5));

        let frame = rig.solve(&with_eyes());

        let input = rig.input();
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
        assert_eq!(frame.joints.len(), rig.skeleton.bone_count());
        for (output, input) in frame.joints.iter().zip(&input) {
            assert_eq!(output.translation, input.translation);
            assert_eq!(output.scale, input.scale);
        }
        for bone in [HIPS, LID, ARM, PROP, PROP_CHILD] {
            assert_eq!(frame.joints[bone], input[bone], "bone {bone}");
        }
        for bone in CHAIN.into_iter().chain(EYES) {
            assert!(rig.turned(&frame, bone) > 1e-3, "bone {bone} did not turn");
        }
    }

    #[test]
    fn a_partial_weight_or_weight_param_turns_each_bone_part_way_at_once_or_once_eased() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.6, 0.2, 2.0));
        rig.set_float(name!("look_weight"), 0.5);
        let at_once = |node: LookAtNode| rig.solve(&node);
        let once_eased = |node: LookAtNode| Stage::start(&rig, node).settle();
        let modes: [&dyn Fn(LookAtNode) -> Frame; 2] = [&at_once, &once_eased];
        let input = rig.input();

        for (solve, full) in modes.into_iter().zip([with_eyes(), eased()]) {
            let solved = solve(full.clone());
            let by_weight = solve(LookAtNode {
                weight: 0.5,
                ..full.clone()
            });
            let by_param = solve(LookAtNode {
                weight_param: Some(name!("look_weight").into()),
                ..full.clone()
            });
            let off = solve(LookAtNode {
                weight: 0.0,
                ..full
            });

            for bone in CHAIN.into_iter().chain(EYES) {
                let halfway = input[bone]
                    .rotation
                    .slerp(solved.joints[bone].rotation, 0.5);
                assert_rotation(by_weight.joints[bone].rotation, halfway);
                assert_rotation(by_param.joints[bone].rotation, halfway);
            }
            assert_unchanged(&rig, &off);
        }
    }

    #[test]
    fn the_eyes_reach_the_target_while_a_slow_head_is_still_turning() {
        let mut rig = Rig::new();
        let target = rig.toward(HEAD, 0.6, 0.15, 2.0);
        rig.aim(target);
        let mut node = with_eyes();
        node.chain[2].halflife = 0.2;

        let first = Stage::start(&rig, node.clone()).step(DT);
        let settled = Stage::start(&rig, node).settle();

        let limits = LookAtEye::default();
        for bone in EYES {
            assert_gaze(&rig, &first, bone, target);
            let turn = rig.turn(&first, bone);
            assert!(turn.yaw.abs() <= limits.yaw_limit + 1e-4, "eye {bone}");
            assert!(turn.pitch.abs() <= limits.pitch_limit + 1e-4, "eye {bone}");
        }
        let head = rig.turned(&first, HEAD);
        let full = rig.turned(&settled, HEAD);
        assert!(head < 0.5 * full, "the head turned {head} of {full}");
    }

    #[test]
    fn an_offset_covers_half_its_turn_in_one_halflife() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 2.0));
        let halflife = 0.1;
        let node = LookAtNode {
            chain: vec![LookAtBone {
                halflife,
                ..link("head")
            }],
            ..look()
        };

        let half = Stage::start(&rig, node.clone()).step(halflife);
        let settled = Stage::start(&rig, node).settle();

        let ratio = rig.turned(&half, HEAD) / rig.turned(&settled, HEAD);
        assert_near(ratio, 0.5, 0.01);
    }

    #[test]
    fn a_target_that_goes_away_or_past_max_angle_eases_the_look_back_to_the_input_pose_exactly() {
        let absences: [fn(&mut Stage, &mut Rig); 4] = [
            |stage, _| stage.forget_target(),
            |stage, _| stage.set_param(name!("look_target"), 1.0_f32),
            |stage, _| stage.set_space(Affine3A::from_scale(Vec3::ZERO)),
            |stage, rig| {
                rig.aim(rig.toward(SPINE, 125.0_f32.to_radians(), 0.0, 3.0));
                stage.follow(rig);
            },
        ];

        for absence in absences {
            let mut rig = Rig::new();
            rig.aim(rig.toward(HEAD, 0.4, 0.2, 2.0));
            let mut stage = Stage::start(&rig, eased());
            stage.settle();
            absence(&mut stage, &mut rig);

            let easing = stage.step(DT);
            let settled = stage.settle();

            assert!(rig.turned(&easing, HEAD) > 1e-3);
            assert_unchanged(&rig, &settled);
        }

        let mut rig = Rig::new();
        rig.aim(rig.toward(SPINE, 0.3, 0.0, 3.0));
        let mut inside = Stage::start(&rig, eased());
        inside.settle();
        rig.aim(rig.toward(SPINE, 115.0_f32.to_radians(), 0.0, 3.0));
        inside.follow(&rig);
        let held = inside.settle();
        for bone in CHAIN {
            assert!(rig.turned(&held, bone) > 1e-3, "bone {bone}");
        }
    }

    #[test]
    fn an_absent_target_holds_the_offsets_at_rest() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 2.0));
        let mut stage = Stage::start(&rig, eased());
        for _ in 0..5 {
            stage.step(DT);
        }
        assert!(stage.read(|state| {
            state
                .offsets
                .iter()
                .any(|offset| offset.velocity != Vec3::ZERO)
        }));
        let held: Vec<Quat> =
            stage.read(|state| state.offsets.iter().map(|offset| offset.value).collect());
        stage.forget_target();

        stage.step(DT);

        stage.read(|state| {
            assert!(state.weight.value > 0.0);
            assert_eq!(state.offsets.len(), held.len());
            for (offset, value) in state.offsets.iter().zip(&held) {
                assert_eq!(offset.velocity, Vec3::ZERO);
                assert_eq!(offset.value, *value);
            }
        });
    }

    #[test]
    fn time_before_the_first_evaluate_does_not_snap_the_look_in() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 2.0));
        let node = LookAtNode {
            weight_halflife: 0.2,
            ..look()
        };
        let full = rig.turned(&Stage::start(&rig, node.clone()).settle(), HEAD);
        let mut stage = Stage::new(&rig, node);
        stage.write(|_, state| state.elapsed = 120.0 * DT);

        let first = stage.step(0.0);
        let next = stage.step(DT);

        assert_unchanged(&rig, &first);
        assert_ne!(next.joints[HEAD], rig.input()[HEAD]);
        let turned = rig.turned(&next, HEAD);
        assert!(turned < 0.5 * full, "turned {turned} of {full}");
    }

    #[test]
    fn a_chain_bone_added_live_turns() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.5, 0.2, 2.0));
        let node = LookAtNode {
            chain: links(&["neck", "head"]),
            ..look()
        };
        let mut stage = Stage::start(&rig, node.clone());
        stage.settle();

        let mut grown = node;
        grown.chain.insert(0, link("spine"));
        stage.set_node(grown);
        stage.step(DT);
        let frame = stage.step(DT);

        for bone in CHAIN {
            assert!(rig.turned(&frame, bone) > 1e-3, "bone {bone}");
        }
    }

    #[test]
    fn a_reset_look_starts_again_from_the_input_pose() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 2.0));
        let mut stage = Stage::start(&rig, eased());
        stage.settle();

        stage.write(|node, state| node.reset(state));

        assert_unchanged(&rig, &stage.step(DT));
    }
}
