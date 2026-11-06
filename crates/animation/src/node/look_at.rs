use std::f32::consts::PI;

use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint, parent},
        pass_through,
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};

const MAX_ANGLE_CAP: f32 = 170.0 * PI / 180.0;

/// Turns a chain of bones, then each eye, toward a world-space point; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct LookAtNode {
    pub chain: Vec<LookAtBone>,
    pub eyes: Vec<LookAtEye>,
    pub forward: Vec3,
    pub up: Vec3,
    pub target_param: Name,
    pub weight: f32,
    pub weight_param: Option<Name>,
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
            target_param: Name::default(),
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

/// The turn of each chain bone and then each eye, and whether a [`LookAtNode`] has already warned about bones it cannot resolve.
#[derive(Debug, Default)]
pub struct LookAtState {
    offsets: Vec<Quat>,
    warned: bool,
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

    fn effective_weight(&self, blackboard: &AnimationBlackboard) -> f32 {
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
        let target = *context.blackboard().get::<Vec3>(self.target_param)?;
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

fn share(share: f32) -> f32 {
    if share.is_finite() && share > 0.0 {
        share
    } else {
        0.0
    }
}

fn limits(yaw: f32, pitch: f32) -> YawPitch {
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

fn descends(skeleton: &Skeleton, bone: usize, ancestor: usize) -> bool {
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
    offsets: &mut [Quat],
    working: &mut AnimationFrame,
) {
    for (look_bone, offset) in look.bones.iter().zip(offsets.iter_mut()) {
        let Some(joint) = model_joint(look.skeleton, working.joints(), look_bone.bone) else {
            continue;
        };
        let direction =
            joint.rotation.inverse() * (target - Vec3::from(joint.transform.translation));
        let angles = yaw_pitch_to(&look_bone.frame, direction);
        let limits = look_bone.limits;
        *offset = look_rotation(
            &look_bone.frame,
            YawPitch {
                yaw: (look_bone.part * angles.yaw).clamp(-limits.yaw, limits.yaw),
                pitch: (look_bone.part * angles.pitch).clamp(-limits.pitch, limits.pitch),
            },
        );
        if let Some(local) = working.joint_mut(look_bone.bone) {
            local.rotation = (local.rotation * *offset).normalize();
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
            offsets: vec![Quat::IDENTITY; self.entry_count()],
            warned: false,
        }
    }

    fn update(&self, _: &mut LookAtState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        state: &mut LookAtState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        pass_through(evaluated_inputs, output);
        if self.entry_count() == 0 {
            return;
        }
        let look = match self.resolve(context.skeleton()) {
            Ok(look) => look,
            Err(reason) => {
                if !state.warned {
                    warn!(
                        "look-at toward `{}` skipped: {reason}",
                        self.target_param.as_str()
                    );
                    state.warned = true;
                }
                return;
            }
        };
        if state.offsets.len() != look.bones.len() {
            state.offsets = vec![Quat::IDENTITY; look.bones.len()];
        }
        let Some(target) = self.target(context) else {
            return;
        };
        let weight = self.goal_weight(&look, output.joints(), target, context.blackboard());
        if weight <= 0.0 {
            return;
        }
        let mut working = pool.acquire();
        working.pose_mut().copy_from(output.pose());
        solve(&look, target, &mut state.offsets, &mut working);
        pool.release(working);
        for (look_bone, offset) in look.bones.iter().zip(&state.offsets) {
            if let Some(joint) = output.joint_mut(look_bone.bone) {
                joint.rotation =
                    (joint.rotation * Quat::IDENTITY.slerp(*offset, weight)).normalize();
            }
        }
    }

    fn reset(&self, _: &mut LookAtState) {}
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{AssetId, asset_store::AssetStore};
    use concerto_mesh::{Skeleton, skeleton::JointPose};
    use concerto_name::name;
    use glam::{Affine3A, Mat4, Quat, Vec3};

    use super::{LookAtBone, LookAtEye, LookAtNode};
    use crate::{
        blackboard::AnimationBlackboard,
        evaluation::AnimationGraphContext,
        look_at::{LookFrame, YawPitch, yaw_pitch_to},
        node::AnimationNode,
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, Pose, RootMotion},
        test_walks::Walks,
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

    struct RigBone {
        name: &'static str,
        parent: Option<u16>,
        joint: JointPose,
    }

    fn rig_bone(
        name: &'static str,
        parent: Option<u16>,
        translation: Vec3,
        rotation: Quat,
    ) -> RigBone {
        RigBone {
            name,
            parent,
            joint: JointPose {
                translation,
                rotation,
                scale: Vec3::ONE,
            },
        }
    }

    fn skeleton() -> Skeleton {
        let bones = [
            rig_bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            rig_bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.2, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            rig_bone(
                "neck",
                Some(1),
                Vec3::new(0.0, 0.4, 0.0),
                Quat::from_rotation_z(0.15),
            ),
            rig_bone(
                "head",
                Some(2),
                Vec3::new(0.0, 0.15, 0.0),
                Quat::from_rotation_y(-0.2),
            ),
            rig_bone(
                "eye_l",
                Some(3),
                Vec3::new(0.03, 0.08, 0.03),
                Quat::from_rotation_x(0.05),
            ),
            rig_bone(
                "eye_r",
                Some(3),
                Vec3::new(-0.03, 0.08, 0.03),
                Quat::from_rotation_x(-0.05),
            ),
            rig_bone(
                "lid",
                Some(4),
                Vec3::new(0.0, 0.01, 0.01),
                Quat::from_rotation_z(0.1),
            ),
            rig_bone(
                "arm",
                Some(1),
                Vec3::new(0.2, 0.35, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            rig_bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
            rig_bone("prop_child", Some(8), Vec3::Y, Quat::from_rotation_y(0.2)),
        ];
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones.iter().map(|bone| bone.joint.clone()).collect(),
            (0..bones.len() as u128)
                .map(uuid::Uuid::from_u128)
                .collect(),
        )
        .unwrap()
    }

    fn input_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.1, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.05),
        }
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("look_input"),
            key: 1,
            time: 0.3,
            weight: 0.5,
            mirrored: false,
        }
    }

    struct Rig {
        walks: Walks,
        blackboard: AnimationBlackboard,
        skeleton: Skeleton,
        joints: Vec<JointPose>,
        model_to_world: Option<Affine3A>,
    }

    impl Rig {
        fn new() -> Self {
            let skeleton = skeleton();
            Self {
                walks: Walks::new(AssetStore::new(), AssetStore::new()),
                blackboard: AnimationBlackboard::default(),
                joints: skeleton.rest_pose.to_vec(),
                skeleton,
                model_to_world: None,
            }
        }

        fn input(&self) -> AnimationFrame {
            let mut frame = AnimationFrame::new(Pose::from_joints(self.joints.clone()));
            frame.set_root_motion(input_motion());
            frame.push_event(input_event());
            frame
        }

        fn solve(&self, node: &LookAtNode) -> AnimationFrame {
            self.run(node, true)
        }

        fn solve_without_skeleton(&self, node: &LookAtNode) -> AnimationFrame {
            self.run(node, false)
        }

        fn run(&self, node: &LookAtNode, with_skeleton: bool) -> AnimationFrame {
            let input = [EvaluatedFrame {
                frame: self.input(),
                weight: 1.0,
            }];
            let mut pool = FramePool::new(self.skeleton.bone_count());
            let mut output = pool.acquire();
            let mut state = self
                .walks
                .create(&self.blackboard, |context| node.create_state(&(), context));
            let evaluate = |context: &AnimationGraphContext<'_>| {
                node.evaluate(
                    &mut state,
                    &(),
                    context,
                    &[],
                    &input,
                    &mut pool,
                    &mut output,
                )
            };
            if with_skeleton {
                self.walks.evaluate_posed(
                    &self.blackboard,
                    &self.skeleton,
                    self.model_to_world,
                    evaluate,
                );
            } else {
                self.walks.evaluate(&self.blackboard, None, evaluate);
            }
            output
        }

        fn model(&self, frame: &AnimationFrame, bone: usize) -> Mat4 {
            self.skeleton.forward_kinematics(frame.joints()).unwrap()[bone]
        }

        fn model_position(&self, frame: &AnimationFrame, bone: usize) -> Vec3 {
            self.model(frame, bone).w_axis.truncate()
        }

        fn model_rotation(&self, frame: &AnimationFrame, bone: usize) -> Quat {
            self.model(frame, bone).to_scale_rotation_translation().1
        }

        fn look_frame(&self, bone: usize) -> LookFrame {
            let rest = self
                .skeleton
                .forward_kinematics(&self.skeleton.rest_pose)
                .unwrap()[bone]
                .to_scale_rotation_translation()
                .1;
            LookFrame::new(rest.inverse() * Vec3::Z, rest.inverse() * Vec3::Y).unwrap()
        }

        fn gaze(&self, frame: &AnimationFrame, bone: usize) -> Vec3 {
            self.model_rotation(frame, bone) * self.look_frame(bone).forward()
        }

        fn turn(&self, frame: &AnimationFrame, bone: usize) -> YawPitch {
            let offset =
                self.input().joints()[bone].rotation.inverse() * frame.joints()[bone].rotation;
            let look = self.look_frame(bone);
            yaw_pitch_to(&look, offset * look.forward())
        }

        fn turned(&self, frame: &AnimationFrame, bone: usize) -> f32 {
            frame.joints()[bone]
                .rotation
                .angle_between(self.input().joints()[bone].rotation)
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
            let world = self
                .model_to_world
                .unwrap_or_default()
                .transform_point3(model_target);
            self.blackboard.set(name!("look_target"), world);
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
            target_param: name!("look_target"),
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

    fn shared(shares: [f32; 3], limit: f32) -> LookAtNode {
        let mut node = look();
        for (link, share) in node.chain.iter_mut().zip(shares) {
            link.share = share;
            link.yaw_limit = limit;
            link.pitch_limit = limit;
        }
        node
    }

    fn assert_gaze(rig: &Rig, frame: &AnimationFrame, bone: usize, target: Vec3) {
        let direction = target - rig.model_position(frame, bone);
        let angle = rig.gaze(frame, bone).angle_between(direction);
        assert!(angle < 1e-3, "bone {bone} looks {angle} rad off the target");
    }

    fn assert_rotation(actual: Quat, expected: Quat) {
        assert!(
            actual.angle_between(expected) < 1e-3,
            "expected rotation {expected}, got {actual}"
        );
    }

    fn assert_near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() < tolerance,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_unchanged(rig: &Rig, frame: &AnimationFrame) {
        assert_eq!(frame.joints(), rig.input().joints());
        assert_eq!(frame.root_motion(), input_motion());
        assert_eq!(frame.events(), [input_event()]);
    }

    fn turned_world() -> Affine3A {
        Affine3A::from_rotation_translation(Quat::from_rotation_y(1.1), Vec3::new(3.0, 0.0, -2.0))
    }

    #[test]
    fn the_head_looks_at_a_target_in_reach() {
        let mut rig = Rig::new();
        let target = rig.toward(HEAD, 0.4, 0.2, 2.0);
        rig.aim(target);

        let frame = rig.solve(&look());

        assert_gaze(&rig, &frame, HEAD, target);
    }

    #[test]
    fn each_chain_bone_takes_its_share_of_the_yaw() {
        let mut rig = Rig::new();
        let target = rig.toward(NECK, 60.0_f32.to_radians(), 0.0, 100.0);
        rig.aim(target);
        let shares = [0.1, 0.3, 0.6];

        let frame = rig.solve(&shared(shares, 90.0_f32.to_radians()));

        let yaws = CHAIN.map(|bone| rig.turn(&frame, bone).yaw);
        let total: f32 = yaws.iter().sum();
        for (yaw, share) in yaws.iter().zip(shares) {
            assert_near(yaw / total, share, 1e-2);
        }
        assert_near(total, 60.0_f32.to_radians(), 0.5_f32.to_radians());
    }

    #[test]
    fn a_clamped_bone_leaves_the_rest_of_the_turn_to_the_bones_after_it() {
        let mut rig = Rig::new();
        let target = rig.toward(NECK, 60.0_f32.to_radians(), 0.0, 100.0);
        rig.aim(target);
        let mut node = shared([0.1, 0.3, 0.6], 90.0_f32.to_radians());
        node.chain[1].yaw_limit = 5.0_f32.to_radians();

        let frame = rig.solve(&node);

        assert_near(rig.turn(&frame, NECK).yaw, 5.0_f32.to_radians(), 1e-3);
        assert_gaze(&rig, &frame, HEAD, target);
    }

    #[test]
    fn a_target_past_every_limit_leaves_each_bone_at_its_limit() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(NECK, 90.0_f32.to_radians(), 0.0, 100.0));
        let node = LookAtNode {
            max_angle: 170.0_f32.to_radians(),
            ..shared([1.0, 1.0, 1.0], 10.0_f32.to_radians())
        };

        let frame = rig.solve(&node);

        for bone in CHAIN {
            assert_near(rig.turn(&frame, bone).yaw, 10.0_f32.to_radians(), 1e-3);
        }
    }

    struct Animated {
        spine: Quat,
        head: Quat,
    }

    #[test]
    fn the_look_turns_on_top_of_the_input_pose() {
        let mut rig = Rig::new();
        let target = rig.toward(HEAD, 0.3, 0.1, 2.0);
        rig.aim(target);
        let rest = rig.skeleton.rest_pose.to_vec();
        let animations = [
            Animated {
                spine: Quat::from_rotation_y(0.3),
                head: Quat::from_rotation_x(-0.2),
            },
            Animated {
                spine: Quat::from_rotation_z(-0.25),
                head: Quat::from_rotation_y(-0.4),
            },
        ];

        for animated in animations {
            rig.joints = rest.clone();
            rig.joints[SPINE].rotation = rest[SPINE].rotation * animated.spine;
            rig.joints[HEAD].rotation = rest[HEAD].rotation * animated.head;

            let frame = rig.solve(&look());

            assert_gaze(&rig, &frame, HEAD, target);
            let input = rig.input();
            for bone in CHAIN {
                assert_eq!(
                    frame.joints()[bone].translation,
                    input.joints()[bone].translation
                );
            }
        }
    }

    #[test]
    fn each_eye_looks_at_a_near_target_from_its_own_position() {
        let mut rig = Rig::new();
        let input = rig.input();
        let between = (rig.model_position(&input, EYE_L) + rig.model_position(&input, EYE_R)) * 0.5;
        let target = between + rig.gaze(&input, HEAD) * 0.6;
        rig.aim(target);

        let frame = rig.solve(&with_eyes());

        for bone in EYES {
            assert_gaze(&rig, &frame, bone, target);
        }
        let left = rig.turn(&frame, EYE_L).yaw;
        let right = rig.turn(&frame, EYE_R).yaw;
        assert!(left * right < 0.0, "the eyes yaw {left} and {right}");
    }

    #[test]
    fn an_eye_that_cannot_reach_holds_its_limits() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.8, 0.4, 5.0));
        let narrow = |name: &str| LookAtEye {
            yaw_limit: 0.02,
            pitch_limit: 0.01,
            ..eye(name)
        };
        let node = LookAtNode {
            chain: Vec::new(),
            eyes: vec![narrow("eye_l"), narrow("eye_r")],
            ..look()
        };

        let frame = rig.solve(&node);

        for bone in EYES {
            let turn = rig.turn(&frame, bone);
            assert_near(turn.yaw, 0.02, 1e-4);
            assert_near(turn.pitch, 0.01, 1e-4);
        }
    }

    #[test]
    fn a_target_under_a_turned_model_is_looked_at_in_model_space() {
        let mut rig = Rig::new();
        rig.model_to_world = Some(turned_world());
        let target = rig.toward(HEAD, -0.3, 0.15, 3.0);
        rig.aim(target);

        let frame = rig.solve(&look());

        assert_gaze(&rig, &frame, HEAD, target);
    }

    #[test]
    fn bones_the_node_cannot_resolve_leave_the_input_pose() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.1, 2.0));
        let unresolvable = [
            LookAtNode {
                chain: links(&["spine", "missing", "head"]),
                ..look()
            },
            LookAtNode {
                chain: links(&["neck", "spine", "head"]),
                ..look()
            },
            LookAtNode {
                eyes: vec![eye("arm")],
                ..look()
            },
            LookAtNode {
                eyes: vec![eye("eye_l"), eye("lid")],
                ..look()
            },
            LookAtNode {
                eyes: vec![eye("eye_l"), eye("eye_l")],
                ..look()
            },
            LookAtNode {
                chain: links(&["prop_child"]),
                ..look()
            },
            LookAtNode {
                forward: Vec3::ZERO,
                ..look()
            },
            LookAtNode {
                up: Vec3::Z * 2.0,
                ..look()
            },
        ];

        for node in &unresolvable {
            assert_unchanged(&rig, &rig.solve(node));
        }
        assert_unchanged(&rig, &rig.solve_without_skeleton(&look()));
    }

    #[test]
    fn root_motion_events_translations_scales_and_other_bones_pass_through() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.2, 0.5));

        let frame = rig.solve(&with_eyes());

        let input = rig.input();
        assert_eq!(frame.root_motion(), input_motion());
        assert_eq!(frame.events(), [input_event()]);
        for bone in 0..rig.skeleton.bone_count() {
            assert_eq!(
                frame.joints()[bone].translation,
                input.joints()[bone].translation
            );
            assert_eq!(frame.joints()[bone].scale, input.joints()[bone].scale);
        }
        for bone in [HIPS, LID, ARM, PROP, PROP_CHILD] {
            assert_eq!(frame.joints()[bone], input.joints()[bone], "bone {bone}");
        }
        for bone in CHAIN.into_iter().chain(EYES) {
            assert!(rig.turned(&frame, bone) > 1e-3, "bone {bone} did not turn");
        }
    }

    #[test]
    fn a_negative_or_nan_share_counts_as_zero() {
        let mut rig = Rig::new();
        let target = rig.toward(NECK, 0.5, 0.2, 3.0);
        rig.aim(target);

        for share in [-1.0, f32::NAN] {
            let mut node = look();
            node.chain[1].share = share;

            let frame = rig.solve(&node);

            assert!(rig.turned(&frame, NECK) < 1e-3, "share {share}");
            assert_gaze(&rig, &frame, HEAD, target);
        }
    }

    #[test]
    fn zero_shares_turn_no_chain_bone() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(NECK, 0.5, 0.2, 3.0));

        let frame = rig.solve(&shared([0.0, 0.0, 0.0], 1.0));

        for bone in CHAIN {
            assert!(rig.turned(&frame, bone) < 1e-3, "bone {bone}");
        }
    }

    #[test]
    fn a_negative_or_nan_limit_holds_its_bone_still() {
        let mut rig = Rig::new();
        let target = rig.toward(NECK, 0.5, 0.2, 3.0);
        rig.aim(target);

        for limit in [-0.5, f32::NAN] {
            let mut node = look();
            node.chain[1].yaw_limit = limit;
            node.chain[1].pitch_limit = limit;

            let frame = rig.solve(&node);

            assert!(rig.turned(&frame, NECK) < 1e-3, "limit {limit}");
            assert_gaze(&rig, &frame, HEAD, target);
        }
    }

    #[test]
    fn a_nan_max_angle_fades_any_target_off_the_first_gaze() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(SPINE, 0.2, 0.0, 3.0));

        let frame = rig.solve(&LookAtNode {
            max_angle: f32::NAN,
            ..look()
        });

        assert_unchanged(&rig, &frame);
    }

    #[test]
    fn a_max_angle_past_170_degrees_is_capped() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(SPINE, 175.0_f32.to_radians(), 0.0, 3.0));

        let frame = rig.solve(&LookAtNode {
            max_angle: 4.0,
            ..look()
        });

        assert_unchanged(&rig, &frame);
    }

    #[test]
    fn a_partial_weight_turns_each_bone_part_way_at_once() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.6, 0.2, 2.0));
        rig.blackboard.set(name!("look_weight"), 0.5_f32);
        let full = with_eyes();
        let solved = rig.solve(&full);
        let input = rig.input();

        let by_weight = rig.solve(&LookAtNode {
            weight: 0.5,
            ..full.clone()
        });
        let by_param = rig.solve(&LookAtNode {
            weight_param: Some(name!("look_weight")),
            ..full.clone()
        });

        for bone in CHAIN.into_iter().chain(EYES) {
            let halfway = input.joints()[bone]
                .rotation
                .slerp(solved.joints()[bone].rotation, 0.5);
            assert_rotation(by_weight.joints()[bone].rotation, halfway);
            assert_rotation(by_param.joints()[bone].rotation, halfway);
        }
    }

    #[test]
    fn a_zero_weight_or_weight_param_leaves_the_input_pose() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.6, 0.2, 2.0));
        rig.blackboard.set(name!("look_weight"), 0.0_f32);

        let by_weight = rig.solve(&LookAtNode {
            weight: 0.0,
            ..with_eyes()
        });
        let by_param = rig.solve(&LookAtNode {
            weight_param: Some(name!("look_weight")),
            ..with_eyes()
        });

        assert_unchanged(&rig, &by_weight);
        assert_unchanged(&rig, &by_param);
    }

    #[test]
    fn a_missing_or_unmappable_target_leaves_the_input_pose() {
        let mut rig = Rig::new();
        assert_unchanged(&rig, &rig.solve(&with_eyes()));

        rig.blackboard.set(name!("look_target"), 1.0_f32);
        assert_unchanged(&rig, &rig.solve(&with_eyes()));

        rig.model_to_world = Some(Affine3A::from_scale(Vec3::ZERO));
        rig.blackboard
            .set(name!("look_target"), Vec3::new(0.3, 1.5, 2.0));
        assert_unchanged(&rig, &rig.solve(&with_eyes()));
    }

    #[test]
    fn a_target_past_max_angle_leaves_the_input_pose_and_one_inside_does_not() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(SPINE, 125.0_f32.to_radians(), 0.0, 3.0));
        assert_unchanged(&rig, &rig.solve(&look()));

        rig.aim(rig.toward(SPINE, 115.0_f32.to_radians(), 0.0, 3.0));
        let frame = rig.solve(&look());
        assert!(
            CHAIN
                .into_iter()
                .any(|bone| rig.turned(&frame, bone) > 1e-3)
        );
    }

    #[test]
    fn a_node_without_bones_passes_its_input_through() {
        let mut rig = Rig::new();
        rig.aim(rig.toward(HEAD, 0.4, 0.1, 2.0));

        let frame = rig.solve(&LookAtNode {
            chain: Vec::new(),
            ..look()
        });

        assert_unchanged(&rig, &frame);
    }
}
