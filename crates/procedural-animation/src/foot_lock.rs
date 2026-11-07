use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::AnimationGraphContext,
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, model_joint},
    },
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_foundation::math::Spring;
use concerto_mesh::{JointName, Skeleton};
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::Vec3;
use log::warn;
use uuid::Uuid;

use crate::{
    foot_placement::{ease, elapsed_after},
    leg::{ResolvedLeg, resolve_leg},
};

/// Holds each planted foot where it was planted, horizontally, and eases back to the animated foot when the plant ends; passes its first input through first.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct FootLockNode {
    pub legs: Vec<FootLockLeg>,
    pub plant_speed: f32,
    pub plant_height: f32,
    pub unlock_radius: f32,
    pub release_halflife: f32,
    pub softness: f32,
    pub weight: f32,
    pub weight_param: Option<Name>,
    pub weight_halflife: f32,
}

impl Default for FootLockNode {
    fn default() -> Self {
        Self {
            legs: Vec::new(),
            plant_speed: 0.15,
            plant_height: 0.1,
            unlock_radius: 0.2,
            release_halflife: 0.1,
            softness: 0.0,
            weight: 1.0,
            weight_param: None,
            weight_halflife: 0.2,
        }
    }
}

impl FootLockNode {
    /// A node with the default settings and no legs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a leg to lock.
    pub fn with_leg(mut self, leg: FootLockLeg) -> Self {
        self.legs.push(leg);
        self
    }

    /// Sets the horizontal speed, in metres per second, under which a foot can plant.
    pub fn with_plant_speed(mut self, plant_speed: f32) -> Self {
        self.plant_speed = plant_speed;
        self
    }

    /// Sets the height above the model origin under which a contact point can plant.
    pub fn with_plant_height(mut self, plant_height: f32) -> Self {
        self.plant_height = plant_height;
        self
    }

    /// Sets how far the animated foot may move from a lock before the lock is released.
    pub fn with_unlock_radius(mut self, unlock_radius: f32) -> Self {
        self.unlock_radius = unlock_radius;
        self
    }

    /// Sets the halflife a released foot eases back to the animated one with.
    pub fn with_release_halflife(mut self, release_halflife: f32) -> Self {
        self.release_halflife = release_halflife;
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
}

/// A leg: the foot bone names the chain (its parent the knee, its grandparent the hip), and an optional toe bone is the point a plant is read from and held at.
#[derive(Clone, Debug, Default, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct FootLockLeg {
    pub foot: JointName,
    pub toe: Option<JointName>,
}

impl FootLockLeg {
    /// A leg on the given foot bone, without a toe.
    pub fn new(foot: impl Into<JointName>) -> Self {
        Self {
            foot: foot.into(),
            toe: None,
        }
    }

    /// Sets the toe bone a plant is read from and held at.
    pub fn with_toe(mut self, toe: impl Into<JointName>) -> Self {
        self.toe = Some(toe.into());
        self
    }
}

/// Where each leg's lock is and how it is held, the time it has to ease over, and whether it has warned about legs it cannot resolve.
#[derive(Debug)]
pub struct FootLockState {
    warned: bool,
    elapsed: f32,
    started: bool,
    weight: Spring<f32>,
    legs: Vec<LegLock>,
}

impl FootLockState {
    fn new(node: &FootLockNode) -> Self {
        Self {
            warned: false,
            elapsed: 0.0,
            started: false,
            weight: Spring::critically_damped_with_halflife(0.0, node.weight_halflife),
            legs: Vec::new(),
        }
    }

    /// Each leg's lock, in the node's leg order.
    pub fn legs(&self) -> &[LegLock] {
        &self.legs
    }
}

/// One leg's lock as the last evaluate left it, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegLock {
    pub phase: LockPhase,
    pub lock: Vec3,
    pub animated: Vec3,
    pub shown: Vec3,
    previous: Option<Vec3>,
    planted: bool,
    waits_for_lift: bool,
    offset: Vec3,
    release: Spring<Vec3>,
}

impl LegLock {
    fn new(release_halflife: f32) -> Self {
        Self {
            phase: LockPhase::Free,
            lock: Vec3::ZERO,
            animated: Vec3::ZERO,
            shown: Vec3::ZERO,
            previous: None,
            planted: false,
            waits_for_lift: false,
            offset: Vec3::ZERO,
            release: Spring::critically_damped_with_halflife(Vec3::ZERO, release_halflife),
        }
    }

    fn free(&mut self) {
        self.phase = LockPhase::Free;
        self.previous = None;
        self.planted = false;
        self.waits_for_lift = false;
        self.offset = Vec3::ZERO;
    }
}

/// Whether a leg's foot is held, easing back to the animated foot, or following it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LockPhase {
    #[default]
    Free,
    Locked,
    Releasing,
}

struct ResolvedLegs<'a> {
    skeleton: &'a Skeleton,
    legs: Vec<ResolvedLeg>,
}

impl FootLockNode {
    fn resolve<'a>(&self, skeleton: Option<&'a Skeleton>) -> Result<ResolvedLegs<'a>, String> {
        let skeleton = skeleton.ok_or("the player has no skeleton")?;
        let legs = self
            .legs
            .iter()
            .map(|leg| resolve_leg(skeleton, leg.foot, leg.toe))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ResolvedLegs { skeleton, legs })
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
}

fn contact_points(resolved: &ResolvedLegs<'_>, output: &AnimationFrame) -> Option<Vec<Vec3>> {
    resolved
        .legs
        .iter()
        .map(|leg| {
            model_joint(
                resolved.skeleton,
                output.joints(),
                leg.toe.unwrap_or(leg.foot),
            )
            .map(|joint| Vec3::from(joint.transform.translation))
        })
        .collect()
}

impl AnimationNode for FootLockNode {
    type State = FootLockState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> FootLockState {
        FootLockState::new(self)
    }

    fn update(
        &self,
        state: &mut FootLockState,
        delta_time: f32,
        _: &(),
        _: &AnimationGraphContext<'_>,
    ) {
        state.elapsed = elapsed_after(state.elapsed, delta_time);
    }

    fn evaluate(
        &self,
        state: &mut FootLockState,
        _: &(),
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
                    warn!("foot lock skipped: {reason}");
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

        let weight_target = self.weight_target(context.blackboard());
        state
            .legs
            .resize(resolved.legs.len(), LegLock::new(self.release_halflife));
        if !state.started {
            state.weight.reset_to(weight_target);
            state.started = true;
        }
        ease(
            &mut state.weight,
            weight_target,
            self.weight_halflife,
            elapsed,
        );
        for leg in &mut state.legs {
            leg.release.set_halflife(self.release_halflife);
        }

        let Some(contacts) = contact_points(&resolved, output) else {
            return;
        };
        for (leg, contact) in state.legs.iter_mut().zip(contacts) {
            leg.animated = model_to_world.transform_point3(contact);
            leg.shown = leg.animated;
        }
    }

    fn reset(&self, state: &mut FootLockState) {
        state.elapsed = 0.0;
        state.started = false;
        for leg in &mut state.legs {
            leg.free();
        }
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
        pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion},
    };
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        Entity,
        component::scene::SceneEntityRef,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
        world::World,
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
    use concerto_name::{Name, name};
    use concerto_reflect::{Reflect, TypeRegistry};
    use glam::{Mat4, Quat, Vec3};
    use uuid::Uuid;

    use super::{FootLockLeg, FootLockNode, FootLockState, LockPhase};
    use crate::plugin::ProceduralAnimationPlugin;

    const EPS: f32 = 1.0e-4;
    const DT: f32 = 1.0 / 60.0;
    const KNEE_BEND: f32 = 0.1;
    const TOE_AHEAD: Vec3 = Vec3::new(0.0, 0.0, 0.15);

    fn script_param() -> Name {
        name!("script")
    }

    fn input_event() -> FiredEvent {
        FiredEvent {
            clip: AssetId::from_path("foot_lock_input"),
            key: 1,
            time: 0.3,
            weight: 0.5,
            mirrored: false,
        }
    }

    #[derive(Clone, Default, Reflect)]
    struct Script {
        translations: Vec<Vec3>,
        rotations: Vec<Quat>,
        root_translation: Vec3,
        root_rotation: Quat,
    }

    impl Script {
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
            let mut script = Self::rest_of(skeleton);
            script.translations[0] += Vec3::new(0.02, -0.03, 0.01);
            script.rotations[0] = Quat::from_rotation_y(0.1);
            script.rotations[2] = Quat::from_rotation_x(0.05);
            script
        }
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct ScriptedPose {}

    impl AnimationNode for ScriptedPose {
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
            context: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            let Some(script) = context.blackboard().get::<Script>(script_param()) else {
                return;
            };
            for (bone, translation) in script.translations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.translation = *translation;
                }
            }
            for (bone, rotation) in script.rotations.iter().enumerate() {
                if let Some(joint) = output.joint_mut(bone) {
                    joint.rotation = *rotation;
                }
            }
            output.set_root_motion(RootMotion {
                translation: script.root_translation,
                rotation: script.root_rotation,
            });
            output.push_event(input_event());
        }

        fn reset(&self, _: &mut ()) {}
    }

    struct Captured {
        root_motion: RootMotion,
        events: Vec<FiredEvent>,
    }

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Capture {}

    impl AnimationNode for Capture {
        type State = Option<Captured>;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> Option<Captured> {
            None
        }

        fn update(&self, _: &mut Option<Captured>, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            state: &mut Option<Captured>,
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
            *state = Some(Captured {
                root_motion: input.frame.root_motion(),
                events: input.frame.events().to_vec(),
            });
        }

        fn reset(&self, state: &mut Option<Captured>) {
            *state = None;
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

    fn leg_bones(names: [&'static str; 4], hip_x: f32, first: u16) -> [Bone; 4] {
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
                translation: TOE_AHEAD,
            },
        ]
    }

    fn rig() -> Skeleton {
        let mut bones = vec![Bone {
            name: "pelvis",
            parent: None,
            translation: Vec3::new(0.0, 0.9, 0.0),
        }];
        bones.extend(leg_bones(
            ["left_hip", "left_knee", "left_foot", "left_toe"],
            0.1,
            1,
        ));
        bones.extend(leg_bones(
            ["right_hip", "right_knee", "right_foot", "right_toe"],
            -0.1,
            5,
        ));
        skeleton_of(&bones)
    }

    fn locking() -> FootLockNode {
        FootLockNode::new()
            .with_leg(FootLockLeg::new("left_foot").with_toe("left_toe"))
            .with_leg(FootLockLeg::new("right_foot").with_toe("right_toe"))
    }

    struct Stage {
        app: App,
        bones: Vec<Entity>,
        player: Entity,
        script: Script,
    }

    impl Stage {
        fn new(skeleton: Skeleton, script: Script, node: FootLockNode) -> Self {
            let mut app = App::new();
            app.register_plugin(AssetManagerPlugin);
            app.insert_resource(Time::new());
            app.register_plugin(AnimationPlugin);
            app.register_plugin(ProceduralAnimationPlugin);
            app.register_animation_node::<ScriptedPose>();
            app.register_animation_node::<Capture>();

            let bone_count = skeleton.bone_count();
            let world = app.main_mut().world_mut();
            world.register_component::<Transform>();
            let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
            world
                .get_resource_mut::<AssetStore<Skeleton>>()
                .unwrap()
                .insert(skeleton_handle.id(), skeleton);
            let graph_handle = add_graph(world, node);
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

            let mut stage = Self {
                app,
                bones,
                player,
                script: script.clone(),
            };
            stage.set_param(script_param(), script);
            stage
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

        fn run(&mut self) {
            self.world().run_schedule(LateUpdate);
        }

        fn settle(&mut self) {
            self.run();
            self.run();
        }

        fn state_of<N: AnimationNode, R>(&mut self, read: impl FnOnce(&mut N::State) -> R) -> R {
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
            result.expect("the graph holds the node")
        }

        fn with_state<R>(&mut self, read: impl FnOnce(&mut FootLockState) -> R) -> R {
            self.state_of::<FootLockNode, R>(read)
        }

        fn warned(&mut self) -> bool {
            self.with_state(|state| state.warned)
        }

        fn phases(&mut self) -> Vec<LockPhase> {
            self.with_state(|state| state.legs().iter().map(|leg| leg.phase).collect())
        }

        fn local(&mut self, bone: usize) -> Transform {
            let entity = self.bones[bone];
            self.world()
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .clone()
        }

        fn assert_input_pose(&mut self) {
            for bone in 0..self.bones.len() {
                let local = self.local(bone);
                let translation = self.script.translations[bone];
                let rotation = self.script.rotations[bone];
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

    fn add_graph(world: &mut World, node: FootLockNode) -> AssetHandle<AnimationGraph> {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<FootLockNode>();
        registry.register::<ScriptedPose>();
        registry.register::<Capture>();
        let mut graph = AnimationGraph::new();
        graph.result_node().with_input(Capture {}, |mut capture| {
            capture.with_input(node, |mut lock| {
                lock.with_input(ScriptedPose {}, |_| {});
            });
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
    fn a_leg_the_node_cannot_resolve_leaves_the_input_pose_and_marks_its_warning() {
        let unresolvable = [
            FootLockNode::new().with_leg(FootLockLeg::new("missing_foot")),
            FootLockNode::new().with_leg(FootLockLeg::new("left_hip")),
            FootLockNode::new().with_leg(FootLockLeg::new("left_foot").with_toe("missing_toe")),
        ];

        for node in unresolvable {
            let skeleton = rig();
            let script = Script::shifted_of(&skeleton);
            let mut stage = Stage::new(skeleton, script, node.clone());
            stage.settle();

            stage.assert_input_pose();
            assert!(stage.warned(), "{node:?}");
        }
    }

    #[test]
    fn a_resolvable_node_does_not_warn() {
        let skeleton = rig();
        let script = Script::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, locking());
        stage.settle();

        assert!(!stage.warned());
    }

    #[test]
    fn a_node_without_legs_passes_its_input_through() {
        let skeleton = rig();
        let script = Script::shifted_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, FootLockNode::new());
        stage.settle();

        stage.assert_input_pose();
        assert!(!stage.warned());
    }

    #[test]
    fn the_inputs_root_motion_and_events_reach_the_output_unchanged() {
        let skeleton = rig();
        let moved = RootMotion {
            translation: Vec3::new(0.01, 0.0, 0.04),
            rotation: Quat::from_rotation_y(0.03),
        };
        let script = Script {
            root_translation: moved.translation,
            root_rotation: moved.rotation,
            ..Script::shifted_of(&skeleton)
        };
        let mut stage = Stage::new(skeleton, script, locking());
        stage.settle();

        let captured = stage.state_of::<Capture, _>(|captured| {
            captured
                .take()
                .expect("the capture node evaluated its input")
        });
        assert_eq!(captured.root_motion, moved);
        assert_eq!(captured.events, [input_event()]);
    }

    #[test]
    fn evaluate_takes_the_elapsed_time() {
        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, locking());
        stage.settle();

        stage.with_state(|state| state.elapsed = 0.3);
        stage.run();

        assert_eq!(stage.with_state(|state| state.elapsed), 0.0);
    }

    #[test]
    fn no_leg_locks_on_the_first_evaluate_after_creation_or_after_a_reset() {
        let node = locking();
        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node.clone());
        stage.settle();
        assert_eq!(stage.phases(), [LockPhase::Free; 2]);

        stage.with_state(|state| state.elapsed = DT);
        stage.run();
        stage.with_state(|state| node.reset(state));
        stage.with_state(|state| state.elapsed = DT);
        stage.run();

        assert_eq!(stage.phases(), [LockPhase::Free; 2]);
    }
}
