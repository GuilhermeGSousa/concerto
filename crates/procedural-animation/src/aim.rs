use concerto_animation::{
    evaluation::AnimationGraphContext,
    look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, bone_under_first, model_joint},
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose},
};
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::look_at::{descends, limits, share};

const AIM_PASSES: usize = 3;

/// Turns a chain of bones so a muzzle's barrel points at a world-space point; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct AimNode {
    pub chain: Vec<AimBone>,
    pub muzzle: Muzzle,
    pub up: Vec3,
    pub target_param: Name,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub weight_halflife: f32,
    pub reacquire_halflife: f32,
    pub reacquire_param: Option<Name>,
}

impl Default for AimNode {
    fn default() -> Self {
        Self {
            chain: Vec::new(),
            muzzle: Muzzle::default(),
            up: Vec3::Y,
            target_param: Name::default(),
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
    pub bone: JointName,
    pub share: f32,
    pub yaw_limit: f32,
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
    pub bone: JointName,
    pub offset: Vec3,
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
    pub fn new(target_param: Name, muzzle: Muzzle) -> Self {
        Self {
            target_param,
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
    pub fn with_weight_param(mut self, param: Name) -> Self {
        self.weight_param = Some(param);
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
    pub fn with_reacquire_param(mut self, param: Name) -> Self {
        self.reacquire_param = Some(param);
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

/// The turn each chain bone of an [`AimNode`] last solved to, the pose it solves on, and whether it has warned about bones it cannot resolve.
#[derive(Debug)]
pub struct AimState {
    offsets: Vec<Quat>,
    working: Vec<JointPose>,
    warned: bool,
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

    fn target(&self, context: &AnimationGraphContext<'_>) -> Option<Vec3> {
        let target = *context.blackboard().get::<Vec3>(self.target_param)?;
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

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> AimState {
        AimState {
            offsets: vec![Quat::IDENTITY; self.chain.len()],
            working: Vec::new(),
            warned: false,
        }
    }

    fn update(&self, _: &mut AimState, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

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
        if self.chain.is_empty() {
            return;
        }
        let aim = match self.resolve(context.skeleton()) {
            Ok(aim) => aim,
            Err(reason) => {
                if !state.warned {
                    warn!(
                        "aim toward `{}` skipped: {reason}",
                        self.target_param.as_str()
                    );
                    state.warned = true;
                }
                return;
            }
        };
        if state.offsets.len() != aim.links.len() {
            state.offsets = vec![Quat::IDENTITY; aim.links.len()];
        }
        let Some(target) = self.target(context) else {
            state.offsets.fill(Quat::IDENTITY);
            return;
        };
        if let Some(offsets) = self.solve(
            &aim,
            output.joints(),
            target,
            AIM_PASSES,
            &mut state.working,
        ) {
            state.offsets = offsets;
        }
        for (link, offset) in aim.links.iter().zip(&state.offsets) {
            if let Some(joint) = output.joint_mut(link.bone) {
                joint.rotation *= *offset;
            }
        }
    }

    fn reset(&self, state: &mut AimState) {
        state.offsets.fill(Quat::IDENTITY);
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        look_at::{LookFrame, YawPitch, look_rotation, yaw_pitch_to},
        node::{AnimationNode, ReflectAnimationNode},
        player::{AnimationHandleComponent, AnimationPlayer},
        plugin::{AnimationAppExt, AnimationPlugin, register_animation_types},
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion},
    };
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        component::scene::SceneEntityRef,
        entity::hierarchy::ChildOf,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
        world::World,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
    use concerto_name::{Name, name};
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{AimBone, AimNode, AimState, Muzzle};

    const SPINE: usize = 1;
    const CHEST: usize = 2;
    const HAND: usize = 3;
    const ARM: usize = 4;
    const PROP: usize = 5;
    const LOW: usize = 1;
    const HIGH: usize = 2;
    const GUN: usize = 3;
    const MISS: f32 = 0.005;

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

    fn build(bones: &[RigBone]) -> Skeleton {
        Skeleton::new(
            vec![Mat4::IDENTITY; bones.len()],
            bones.iter().map(|bone| bone.parent).collect(),
            bones.iter().map(|bone| bone.name.into()).collect(),
            bones.iter().map(|bone| bone.joint.clone()).collect(),
            (0..bones.len() as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
    }

    fn spine_rig() -> Skeleton {
        build(&[
            rig_bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            rig_bone(
                "spine",
                Some(0),
                Vec3::new(0.0, 0.1, 0.0),
                Quat::from_rotation_x(0.1),
            ),
            rig_bone(
                "chest",
                Some(1),
                Vec3::new(0.0, 0.25, 0.0),
                Quat::from_rotation_x(-0.1),
            ),
            rig_bone(
                "hand",
                Some(2),
                Vec3::new(0.1, 0.0, 0.5),
                Quat::from_rotation_y(0.3),
            ),
            rig_bone(
                "arm",
                Some(1),
                Vec3::new(0.2, 0.2, 0.0),
                Quat::from_rotation_z(-1.2),
            ),
            rig_bone("prop", None, Vec3::X, Quat::from_rotation_x(0.3)),
        ])
    }

    fn straight_rig() -> Skeleton {
        build(&[
            rig_bone("hips", None, Vec3::Y, Quat::from_rotation_y(0.4)),
            rig_bone("low", Some(0), Vec3::new(0.0, 0.2, 0.0), Quat::IDENTITY),
            rig_bone("high", Some(1), Vec3::new(0.0, 0.0, 0.25), Quat::IDENTITY),
            rig_bone("gun", Some(2), Vec3::new(0.0, 0.0, 0.5), Quat::IDENTITY),
        ])
    }

    fn input_motion() -> RootMotion {
        RootMotion {
            translation: Vec3::new(0.1, 0.0, 0.3),
            rotation: Quat::from_rotation_y(0.05),
        }
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("aim_input"),
            key: 1,
            time: 0.3,
            weight: 0.5,
            mirrored: false,
        }
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

    struct Frame {
        joints: Vec<JointPose>,
        root_motion: RootMotion,
        events: Vec<FiredEvent>,
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Capture {}

    impl AnimationNode for Capture {
        type State = Option<Frame>;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> Option<Frame> {
            None
        }

        fn update(&self, _: &mut Option<Frame>, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            state: &mut Option<Frame>,
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            evaluated_inputs: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            let Some(input) = evaluated_inputs.first() else {
                return;
            };
            output.copy_from(&input.frame);
            *state = Some(Frame {
                joints: input.frame.joints().to_vec(),
                root_motion: input.frame.root_motion(),
                events: input.frame.events().to_vec(),
            });
        }

        fn reset(&self, state: &mut Option<Frame>) {
            *state = None;
        }
    }

    fn graph_of(node: AimNode, rotations: &[Quat]) -> AnimationGraph {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<AimNode>();
        registry.register::<InputPose>();
        registry.register::<Capture>();
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(Capture {}, |mut capture| {
            capture.with_input(node, |mut aim| {
                aim.with_input(
                    InputPose {
                        rotations: rotations.to_vec(),
                    },
                    |_| {},
                );
            });
        });
        graph.make_live(&registry, None).unwrap();
        graph
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

        fn along_barrel(&self, distance: f32) -> Vec3 {
            let input = self.input();
            self.model_position(&input, self.first) + self.barrel(&input).direction * distance
        }

        fn solve(&self, node: &AimNode) -> Frame {
            Stage::new(self, node.clone()).step()
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
        player: Entity,
    }

    impl Stage {
        fn new(rig: &Rig, node: AimNode) -> Self {
            let mut app = App::new();
            app.register_plugin(AssetManagerPlugin);
            app.insert_resource(Time::new());
            app.register_plugin(AnimationPlugin);
            app.register_animation_node::<AimNode>();
            app.register_animation_node::<InputPose>();
            app.register_animation_node::<Capture>();

            let bone_count = rig.skeleton.bone_count();
            let world = app.main_mut().world_mut();
            world.register_component::<Transform>();
            let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<Skeleton>>()
                .unwrap()
                .insert(skeleton_handle.id(), (rig.shape)());
            let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<AnimationGraph>>()
                .unwrap()
                .insert(graph_handle.id(), graph_of(node, &rig.rotations()));
            let bones: Vec<Entity> = (0..bone_count)
                .map(|_| world.spawn(Transform::IDENTITY))
                .collect();
            let space = world.spawn(GlobalTransform::new(Mat4::IDENTITY));
            world.insert(ChildOf::new(space), bones[0]);
            let mut player = AnimationPlayer::new(bone_count);
            player.set_paused(true);
            if let Some(target) = rig.target {
                player.set_param(name!("aim_target"), target);
            }
            let player = world.spawn((
                player,
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
            world.run_schedule(LateUpdate);
            world
                .get_component_for_entity_mut::<AnimationPlayer>(player)
                .unwrap()
                .set_paused(false);

            Self { app, player }
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

        fn state_of<N: AnimationNode, R>(
            &mut self,
            read: impl FnOnce(&mut N::State) -> R,
        ) -> Option<R> {
            let player = self.player;
            let mut read = Some(read);
            let mut result = None;
            self.world()
                .resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
                    world
                        .get_component_for_entity_mut::<AnimationPlayer>(player)
                        .unwrap()
                        .for_each_state_mut::<N>(graphs, |_, state, _| {
                            if let Some(read) = read.take() {
                                result = Some(read(state));
                            }
                        });
                });
            result
        }

        fn with_state<R>(&mut self, read: impl FnOnce(&mut AimState) -> R) -> R {
            self.state_of::<AimNode, R>(read)
                .expect("the graph holds an aim node")
        }

        fn step(&mut self) -> Frame {
            self.world().run_schedule(LateUpdate);
            self.state_of::<Capture, _>(|state: &mut Option<Frame>| state.take())
                .flatten()
                .expect("the aim node was evaluated")
        }
    }

    fn aim() -> AimNode {
        AimNode::new(name!("aim_target"), Rig::new().muzzle)
            .with_bone(AimBone::new("spine"))
            .with_bone(AimBone::new("chest"))
    }

    fn straight_aim(low: AimBone) -> AimNode {
        let wide = 90.0_f32.to_radians();
        AimNode::new(name!("aim_target"), Rig::straight().muzzle)
            .with_bone(low.with_share(1.0))
            .with_bone(AimBone::new("high").with_share(3.0).with_limits(wide, wide))
    }

    fn assert_near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() < tolerance,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_unchanged(rig: &Rig, frame: &Frame) {
        assert_eq!(frame.joints, rig.input());
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
    }

    fn assert_unturned(rig: &Rig, frame: &Frame) {
        for (bone, (output, input)) in frame.joints.iter().zip(rig.input()).enumerate() {
            assert_eq!(output.translation, input.translation, "bone {bone}");
            assert!(
                output.rotation.abs_diff_eq(input.rotation, 1e-6),
                "bone {bone} turned from {} to {}",
                input.rotation,
                output.rotation
            );
        }
        assert_eq!(frame.root_motion, input_motion());
        assert_eq!(frame.events, [input_event()]);
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
    fn the_barrel_passes_within_5_mm_of_targets_at_2_and_3_m() {
        for case in parallax_cases() {
            let mut rig = Rig::new();
            let target = rig.toward(case.yaw, case.pitch, case.distance);
            rig.target = Some(target);

            let frame = rig.solve(&aim());

            let miss = rig.miss(&frame.joints, target);
            assert!(
                miss < MISS,
                "{} m at yaw {} and pitch {}: missed by {miss} m",
                case.distance,
                case.yaw,
                case.pitch
            );
        }
    }

    #[test]
    fn one_pass_misses_by_more_than_three_passes_allow() {
        let rig = Rig::new();
        for case in parallax_cases() {
            let target = rig.toward(case.yaw, case.pitch, case.distance);

            let once = rig.miss(&rig.solve_directly(&aim(), 1, target), target);
            let thrice = rig.miss(&rig.solve_directly(&aim(), 3, target), target);

            assert!(
                once > MISS,
                "{} m at yaw {} and pitch {}: one pass missed by only {once} m",
                case.distance,
                case.yaw,
                case.pitch
            );
            assert!(thrice < MISS, "three passes missed by {thrice} m");
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
    fn a_bone_held_at_its_limit_leaves_the_rest_to_the_next() {
        let mut rig = Rig::straight();
        let target = rig.toward(40.0_f32.to_radians(), 0.0, 100.0);
        rig.target = Some(target);
        let limit = 5.0_f32.to_radians();
        let node = straight_aim(AimBone::new("low").with_limits(limit, 90.0_f32.to_radians()));

        let frame = rig.solve(&node);

        assert_near(rig.turn(&frame.joints, LOW).yaw, limit, 1e-3);
        let barrel = rig.barrel(&frame.joints);
        let angle = barrel.direction.angle_between(target - barrel.point);
        assert!(angle < 1e-3, "the barrel points {angle} rad off the target");
    }

    #[test]
    fn a_target_past_the_summed_limits_leaves_each_bone_at_its_limits() {
        let mut rig = Rig::new();
        rig.target = Some(rig.toward(70.0_f32.to_radians(), 0.0, 3.0));
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

    fn unsolvable(rig: &Rig) -> [Vec3; 2] {
        [rig.along_barrel(-1.0), rig.along_barrel(0.56)]
    }

    #[test]
    fn an_unsolvable_target_holds_the_last_solve() {
        let mut rig = Rig::new();
        rig.target = Some(rig.toward(20.0_f32.to_radians(), 10.0_f32.to_radians(), 2.0));

        for target in unsolvable(&rig) {
            let mut stage = Stage::new(&rig, aim());
            let solved = stage.step();
            let offsets = stage.with_state(|state| state.offsets.clone());
            assert_ne!(solved.joints, rig.input());
            stage.set_param(name!("aim_target"), target);

            let held = stage.step();

            assert_eq!(held.joints, solved.joints);
            assert_eq!(stage.with_state(|state| state.offsets.clone()), offsets);
        }
    }

    #[test]
    fn an_unsolvable_target_before_any_solve_leaves_the_input_pose() {
        let mut rig = Rig::new();
        for target in unsolvable(&rig) {
            rig.target = Some(target);
            assert_unturned(&rig, &rig.solve(&aim()));
        }

        rig.target = Some(rig.toward(0.2, 0.1, 2.0));
        let along_barrel = rig.barrel(&rig.input()).direction;
        assert_unturned(&rig, &rig.solve(&aim().with_up(along_barrel)));
    }

    #[test]
    fn bones_the_node_cannot_resolve_leave_the_input_pose_and_mark_the_warning() {
        let mut rig = Rig::new();
        rig.target = Some(rig.toward(0.3, 0.1, 2.0));
        let muzzle = rig.muzzle.clone();
        let with_chain = |names: &[&str]| {
            names.iter().fold(
                AimNode::new(name!("aim_target"), muzzle.clone()),
                |node, name| node.with_bone(AimBone::new(*name)),
            )
        };
        let unresolvable = [
            AimNode {
                muzzle: Muzzle::new("arm"),
                ..aim()
            },
            AimNode {
                muzzle: Muzzle::new("prop"),
                ..aim()
            },
            AimNode {
                muzzle: Muzzle::new("missing"),
                ..aim()
            },
            with_chain(&["spine", "spine"]),
            with_chain(&["chest", "spine"]),
            with_chain(&["spine", "missing"]),
            with_chain(&["prop"]),
            AimNode {
                muzzle: muzzle.clone().with_direction(Vec3::ZERO),
                ..aim()
            },
            AimNode {
                muzzle: muzzle.clone().with_direction(Vec3::new(f32::NAN, 0.0, 1.0)),
                ..aim()
            },
        ];

        for node in unresolvable {
            let mut stage = Stage::new(&rig, node.clone());
            let first = stage.step();
            let second = stage.step();

            assert_unchanged(&rig, &first);
            assert_unchanged(&rig, &second);
            assert!(stage.with_state(|state| state.warned), "{node:?}");
        }
    }

    #[test]
    fn a_resolvable_node_does_not_warn() {
        let mut rig = Rig::new();
        rig.target = Some(rig.toward(0.3, 0.1, 2.0));
        let mut stage = Stage::new(&rig, aim());

        stage.step();

        assert!(!stage.with_state(|state| state.warned));
    }

    #[test]
    fn a_missing_target_or_an_empty_chain_leaves_the_input_pose() {
        let mut rig = Rig::new();
        assert_unchanged(&rig, &rig.solve(&aim()));

        rig.target = Some(rig.toward(0.3, 0.1, 2.0));
        let empty = AimNode::new(name!("aim_target"), rig.muzzle.clone());
        assert_unchanged(&rig, &rig.solve(&empty));
    }

    #[test]
    fn root_motion_events_translations_and_other_bones_pass_through() {
        let mut rig = Rig::new();
        rig.target = Some(rig.toward(0.3, 0.1, 2.0));

        let frame = rig.solve(&aim());

        let input = rig.input();
        assert_eq!(frame.root_motion, input_motion());
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
}
