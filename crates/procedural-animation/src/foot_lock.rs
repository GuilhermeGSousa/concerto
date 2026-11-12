use concerto_animation::{
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, RootBone},
    node::{
        AnimationNode, ReflectAnimationNode,
        model_space::{WorldToModel, model_joint},
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_foundation::math::Spring;
use concerto_mesh::{JointName, Skeleton};
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};
use log::warn;
use uuid::Uuid;

use crate::leg::{ResolvedLeg, bend_leg, ease, elapsed_after, horizontal, resolve_leg};

const MIN_WEIGHT: f32 = 1.0e-4;
const RELEASED: f32 = 1.0e-3;

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
    pub weight_param: Option<Param<f32>>,
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
    pub fn with_weight_param(mut self, weight_param: impl Into<Param<f32>>) -> Self {
        self.weight_param = Some(weight_param.into());
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
            .and_then(|param| blackboard.read(&param).copied())
            .unwrap_or(1.0);
        let weight = self.weight * scale;
        if weight.is_nan() {
            0.0
        } else {
            weight.clamp(0.0, 1.0)
        }
    }

    fn plant(
        &self,
        leg: &mut LegLock,
        contact: Vec3,
        height: f32,
        motion: &ModelMotion,
        elapsed: f32,
        weight_target: f32,
    ) {
        let Some(previous) = leg.previous else {
            leg.planted = false;
            leg.previous = Some(contact);
            return;
        };
        if elapsed <= 0.0 {
            return;
        }
        let speed = motion.horizontal_speed(previous, contact, elapsed);
        leg.planted = speed < self.plant_speed && height < self.plant_height && weight_target > 0.0;
        leg.previous = Some(contact);
    }

    fn transition(&self, leg: &mut LegLock) -> bool {
        let mut releases = false;
        if leg.phase == LockPhase::Locked {
            let drift = horizontal(leg.lock - leg.animated);
            let stretched = drift.length() > self.unlock_radius;
            if !leg.planted || stretched {
                leg.phase = LockPhase::Releasing;
                leg.release.reset_to(drift);
                leg.waits_for_lift |= stretched;
                releases = true;
            }
        }
        if leg.phase != LockPhase::Locked && leg.planted && !leg.waits_for_lift {
            leg.lock = leg.animated + leg.offset;
            leg.phase = LockPhase::Locked;
        }
        if !leg.planted {
            leg.waits_for_lift = false;
        }
        releases
    }

    fn offset(&self, leg: &mut LegLock, releases: bool, elapsed: f32) -> Vec3 {
        match leg.phase {
            LockPhase::Free => Vec3::ZERO,
            LockPhase::Locked => horizontal(leg.lock - leg.animated),
            LockPhase::Releasing => {
                let offset = if releases {
                    leg.release.value
                } else {
                    ease(&mut leg.release, Vec3::ZERO, self.release_halflife, elapsed)
                };
                if offset.length() < RELEASED {
                    leg.phase = LockPhase::Free;
                    Vec3::ZERO
                } else {
                    offset
                }
            }
        }
    }
}

struct LegPoints {
    contact: Vec3,
    foot: Vec3,
}

fn leg_points(resolved: &ResolvedLegs<'_>, output: &AnimationFrame) -> Option<Vec<LegPoints>> {
    let position = |bone| {
        model_joint(resolved.skeleton, output.joints(), bone)
            .map(|joint| Vec3::from(joint.transform.translation))
    };
    resolved
        .legs
        .iter()
        .map(|leg| {
            Some(LegPoints {
                contact: position(leg.toe.unwrap_or(leg.foot))?,
                foot: position(leg.foot)?,
            })
        })
        .collect()
}

struct ModelMotion {
    translation: Vec3,
    rotation: Quat,
    pivot: Vec3,
    up: Vec3,
}

impl ModelMotion {
    fn of(root: Option<&RootBone>, skeleton: &Skeleton, frame: &AnimationFrame) -> Self {
        let motion = frame.root_motion();
        let up = root.map_or(Vec3::Y, |root| root.up);
        let joint = root.and_then(|root| model_joint(skeleton, frame.joints(), root.index));
        let facing = root
            .zip(joint.as_ref())
            .map(|(root, joint)| joint.rotation * root.rest.rotation.inverse())
            .unwrap_or(Quat::IDENTITY);
        let pivot = joint.map_or(Vec3::ZERO, |joint| {
            let at = Vec3::from(joint.transform.translation);
            at - up * at.dot(up)
        });
        Self {
            translation: facing * motion.translation,
            rotation: facing * motion.rotation * facing.inverse(),
            pivot,
            up,
        }
    }

    fn horizontal_speed(&self, previous: Vec3, contact: Vec3, elapsed: f32) -> f32 {
        let moved =
            self.translation + self.pivot + self.rotation * (contact - self.pivot) - previous;
        (moved - self.up * moved.dot(self.up)).length() / elapsed
    }
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
        let weight = ease(
            &mut state.weight,
            weight_target,
            self.weight_halflife,
            elapsed,
        )
        .clamp(0.0, 1.0);
        for leg in &mut state.legs {
            leg.release.set_halflife(self.release_halflife);
        }

        let Some(points) = leg_points(&resolved, output) else {
            return;
        };
        let motion = ModelMotion::of(context.root(), resolved.skeleton, output);
        let floor = model_to_world.translation.y;
        for (leg, point) in state.legs.iter_mut().zip(&points) {
            let animated = model_to_world.transform_point3(point.contact);
            leg.animated = animated;
            self.plant(
                leg,
                point.contact,
                animated.y - floor,
                &motion,
                elapsed,
                weight_target,
            );
            let releases = self.transition(leg);
            leg.offset = self.offset(leg, releases, elapsed);
            leg.lock.y = animated.y;
            leg.shown = animated + leg.offset * weight;
        }

        if weight.is_nan() || weight < MIN_WEIGHT {
            return;
        }
        for ((leg, lock), point) in resolved.legs.iter().zip(&state.legs).zip(&points) {
            if lock.offset == Vec3::ZERO {
                continue;
            }
            let foot = model_to_world.transform_point3(point.foot);
            let goal = world_to_model
                .transform
                .transform_point3(foot + lock.offset * weight);
            bend_leg(
                resolved.skeleton,
                leg,
                goal,
                Quat::IDENTITY,
                self.softness,
                output,
            );
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
        node::{AnimationNode, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool, JointPose, RootMotion},
        root::AnimationRootBone,
    };
    use concerto_app::{App, schedule_groups::LateUpdate};
    use concerto_ecs::{Entity, entity::hierarchy::ChildOf, world::World};
    use concerto_foundation::{
        assets::AssetId,
        transform::{GlobalTransform, Transform},
    };
    use concerto_mesh::skeleton::Skeleton;
    use concerto_name::{Name, name};
    use concerto_reflect::Reflect;
    use glam::{Affine3A, Mat4, Quat, Vec3};
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{FootLockLeg, FootLockNode, FootLockState, LegLock, LockPhase};
    use crate::testing::{
        Bone, angle, app_with, captured, captured_graph, compile, input_event, insert_asset,
        read_state, skeleton_of, spawn_rigged, write_state,
    };

    const EPS: f32 = 1.0e-4;
    const DT: f32 = 1.0 / 60.0;
    const KNEE_BEND: f32 = 0.1;
    const HALF_LEG: f32 = 0.45;
    const TOE_AHEAD: Vec3 = Vec3::new(0.0, 0.0, 0.15);
    const HIPS: [usize; 2] = [1, 5];
    const FEET: [usize; 2] = [3, 7];
    const TOES: [usize; 2] = [4, 8];
    const STRIDE: Vec3 = Vec3::new(0.0, 0.0, 0.5);
    const PELVIS: Vec3 = Vec3::new(0.0, 0.9, 0.0);
    const TURN: f32 = 0.1;

    fn script_param() -> Name {
        name!("script")
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

        fn moved(&self, by: Vec3) -> Self {
            let mut script = self.clone();
            script.translations[0] += by;
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

    fn leg_bones(names: [&'static str; 4], hip_x: f32, first: u16) -> [Bone; 4] {
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
                    -HALF_LEG * KNEE_BEND.cos(),
                    HALF_LEG * KNEE_BEND.sin(),
                )),
            },
            Bone {
                name: names[2],
                parent: Some(first + 1),
                joint: joint(Vec3::new(
                    0.0,
                    -HALF_LEG * KNEE_BEND.cos(),
                    -HALF_LEG * KNEE_BEND.sin(),
                )),
            },
            Bone {
                name: names[3],
                parent: Some(first + 2),
                joint: joint(TOE_AHEAD),
            },
        ]
    }

    fn rig() -> Skeleton {
        rig_with_pelvis_at(PELVIS)
    }

    fn rig_with_pelvis_at(pelvis: Vec3) -> Skeleton {
        let mut bones = vec![Bone {
            name: "pelvis",
            parent: None,
            joint: JointPose {
                translation: pelvis,
                ..JointPose::IDENTITY
            },
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
        parents: Vec<Option<u16>>,
        model: Option<Entity>,
        model_to_world: Affine3A,
        player: Entity,
        script: Script,
    }

    impl Stage {
        fn new(skeleton: Skeleton, script: Script, node: FootLockNode) -> Self {
            let mut app = app_with::<ScriptedPose>();
            let bone_count = skeleton.bone_count();
            let parents = skeleton.parents.to_vec();
            let world = app.main_mut().world_mut();
            let skeleton = insert_asset(world, AssetId::new(), skeleton);
            let graph = insert_asset(world, AssetId::new(), captured_graph(node, ScriptedPose {}));
            let rigged = spawn_rigged(
                world,
                &skeleton,
                &graph,
                AnimationPlayer::new(bone_count),
                bone_count,
            );
            compile(world);

            let mut stage = Self {
                app,
                bones: rigged.bones,
                parents,
                model: None,
                model_to_world: Affine3A::IDENTITY,
                player: rigged.player,
                script: script.clone(),
            };
            stage.set_param(script_param(), script);
            stage
        }

        fn under(mut self, space: Affine3A) -> Self {
            let root = self.bones[0];
            let world = self.world();
            let model = world.spawn(GlobalTransform::new(Mat4::from(space)));
            world.insert(ChildOf::new(model), root);
            self.model = Some(model);
            self.model_to_world = space;
            self
        }

        fn with_root(mut self) -> Self {
            let root = self.bones[0];
            self.world().insert(AnimationRootBone::default(), root);
            self
        }

        fn move_model(&mut self, space: Affine3A) {
            let model = self.model.expect("the stage has a model");
            self.world()
                .get_component_for_entity_mut::<GlobalTransform>(model)
                .unwrap()
                .set_matrix(Mat4::from(space));
            self.model_to_world = space;
        }

        fn pose(&mut self, script: Script) {
            self.script = script.clone();
            self.set_param(script_param(), script);
        }

        fn step(&mut self, elapsed: f32) {
            let player = self.player;
            write_state::<FootLockNode, _>(self.world(), player, |_, state| {
                state.elapsed += elapsed
            });
            self.run();
        }

        fn read<R>(&mut self, read: impl FnOnce(&FootLockState) -> R) -> R {
            let player = self.player;
            read_state::<FootLockNode, _>(self.world(), player, |_, state| read(state))
                .expect("the graph holds a foot lock node")
        }

        fn legs(&mut self) -> Vec<LegLock> {
            self.read(|state| state.legs().to_vec())
        }

        fn phases(&mut self) -> Vec<LockPhase> {
            self.read(|state| state.legs().iter().map(|leg| leg.phase).collect())
        }

        fn warned(&mut self) -> bool {
            self.read(|state| state.warned)
        }

        fn output_model(&mut self, bone: usize) -> Affine3A {
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
            self.in_model(&locals, bone)
        }

        fn input_model(&self, bone: usize) -> Affine3A {
            let locals: Vec<Affine3A> = (0..self.bones.len())
                .map(|bone| {
                    Affine3A::from_rotation_translation(
                        self.script.rotations[bone],
                        self.script.translations[bone],
                    )
                })
                .collect();
            self.in_model(&locals, bone)
        }

        fn in_model(&self, locals: &[Affine3A], bone: usize) -> Affine3A {
            match self.parents[bone] {
                Some(up) => self.in_model(locals, usize::from(up)) * locals[bone],
                None => locals[bone],
            }
        }

        fn output_world(&mut self, bone: usize) -> Vec3 {
            let model = self.output_model(bone);
            self.model_to_world
                .transform_point3(Vec3::from(model.translation))
        }

        fn input_world(&self, bone: usize) -> Vec3 {
            self.model_to_world
                .transform_point3(Vec3::from(self.input_model(bone).translation))
        }

        fn assert_toes_at(&mut self, locks: &[LegLock]) {
            for (leg, toe) in locks.iter().zip(TOES) {
                let shown = self.output_world(toe);
                assert!(
                    (shown.x - leg.lock.x).abs() < EPS,
                    "{shown} vs {}",
                    leg.lock
                );
                assert!(
                    (shown.z - leg.lock.z).abs() < EPS,
                    "{shown} vs {}",
                    leg.lock
                );
                assert!((shown.y - self.input_world(toe).y).abs() < EPS);
            }
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

    fn rotation_of(transform: Affine3A) -> Quat {
        transform.to_scale_rotation_translation().1
    }

    fn still(node: FootLockNode) -> Stage {
        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node);
        stage.settle();
        stage
    }

    fn locked_in(node: FootLockNode, space: Affine3A) -> Stage {
        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node).under(space);
        stage.settle();
        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Locked; 2]);
        stage
    }

    fn locked(node: FootLockNode) -> Stage {
        locked_in(node, Affine3A::IDENTITY)
    }

    fn walked(turn: Quat, lift: Vec3) -> Stage {
        let skeleton = rig();
        let mut start = Script::rest_of(&skeleton);
        start.rotations[0] = turn;
        let start = Script {
            root_translation: turn.inverse() * (STRIDE + lift),
            ..start
        };
        let mut stage = Stage::new(skeleton, start.clone(), locking()).with_root();
        stage.settle();
        stage.pose(start.moved(lift - STRIDE));
        stage.step(DT);
        stage
    }

    fn turned(pelvis_turn: Quat) -> Stage {
        let skeleton = rig_with_pelvis_at(PELVIS + Vec3::new(0.1, 0.0, 0.0));
        let start = Script {
            root_rotation: Quat::from_rotation_y(TURN),
            ..Script::rest_of(&skeleton)
        };
        let mut stage = Stage::new(skeleton, start.clone(), locking()).with_root();
        stage.settle();
        let mut turned = start;
        turned.rotations[0] = pelvis_turn;
        stage.pose(turned);
        stage.step(DT);
        stage
    }

    struct Resolution {
        node: FootLockNode,
        warns: bool,
    }

    #[test]
    fn a_node_warns_only_for_legs_it_cannot_resolve_and_passes_its_input_through_until_a_foot_locks()
     {
        let rows = [
            Resolution {
                node: FootLockNode::new().with_leg(FootLockLeg::new("missing_foot")),
                warns: true,
            },
            Resolution {
                node: FootLockNode::new().with_leg(FootLockLeg::new("left_hip")),
                warns: true,
            },
            Resolution {
                node: FootLockNode::new()
                    .with_leg(FootLockLeg::new("left_foot").with_toe("missing_toe")),
                warns: true,
            },
            Resolution {
                node: FootLockNode::new(),
                warns: false,
            },
            Resolution {
                node: locking(),
                warns: false,
            },
        ];
        let moved = RootMotion {
            translation: Vec3::new(0.01, 0.0, 0.04),
            rotation: Quat::from_rotation_y(0.03),
        };

        for row in rows {
            let skeleton = rig();
            let script = Script {
                root_translation: moved.translation,
                root_rotation: moved.rotation,
                ..Script::shifted_of(&skeleton)
            };
            let mut stage = Stage::new(skeleton, script, row.node.clone());
            stage.settle();

            stage.assert_input_pose();
            assert_eq!(stage.warned(), row.warns, "{:?}", row.node);
            let player = stage.player;
            let frame = captured(stage.world(), player);
            assert_eq!(frame.root_motion, moved, "{:?}", row.node);
            assert_eq!(frame.events, [input_event()], "{:?}", row.node);
        }
    }

    #[test]
    fn no_leg_locks_on_the_first_evaluate_after_creation_or_after_a_reset() {
        let node = locking();
        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node.clone());
        stage.settle();
        assert_eq!(stage.phases(), [LockPhase::Free; 2]);

        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Locked; 2]);
        let player = stage.player;
        write_state::<FootLockNode, _>(stage.world(), player, |_, state| node.reset(state));
        stage.step(DT);

        assert_eq!(stage.phases(), [LockPhase::Free; 2]);
    }

    struct Plant {
        name: &'static str,
        stage: fn() -> Stage,
        phase: LockPhase,
    }

    #[test]
    fn a_foot_plants_where_its_toe_is_only_while_the_toe_is_slow_and_low_in_the_animations_frame() {
        let rows = [
            Plant {
                name: "still",
                stage: || {
                    let mut stage = still(locking());
                    stage.step(DT);
                    stage
                },
                phase: LockPhase::Locked,
            },
            Plant {
                name: "running in place",
                stage: || {
                    let skeleton = rig();
                    let running = Script {
                        root_translation: STRIDE,
                        ..Script::rest_of(&skeleton)
                    };
                    let mut stage = Stage::new(skeleton, running, locking());
                    stage.settle();
                    stage.step(DT);
                    stage
                },
                phase: LockPhase::Free,
            },
            Plant {
                name: "walking against the root motion",
                stage: || walked(Quat::IDENTITY, Vec3::ZERO),
                phase: LockPhase::Locked,
            },
            Plant {
                name: "walking with a pitched root",
                stage: || walked(Quat::from_rotation_x(20.0_f32.to_radians()), Vec3::ZERO),
                phase: LockPhase::Locked,
            },
            Plant {
                name: "walking with a lifting root",
                stage: || walked(Quat::IDENTITY, Vec3::new(0.0, 0.02, 0.0)),
                phase: LockPhase::Locked,
            },
            Plant {
                name: "held while the root turns about itself",
                stage: || turned(Quat::from_rotation_y(-TURN)),
                phase: LockPhase::Locked,
            },
            Plant {
                name: "carried round by the root's turn",
                stage: || turned(Quat::IDENTITY),
                phase: LockPhase::Free,
            },
            Plant {
                name: "still above the plant height",
                stage: || {
                    let skeleton = rig();
                    let raised = Script::rest_of(&skeleton).moved(Vec3::new(0.0, 0.2, 0.0));
                    let mut stage = Stage::new(skeleton, raised, locking());
                    stage.settle();
                    stage.step(DT);
                    stage
                },
                phase: LockPhase::Free,
            },
        ];

        for row in rows {
            let mut stage = (row.stage)();

            assert_eq!(stage.phases(), [row.phase; 2], "{}", row.name);
            if row.phase == LockPhase::Locked {
                for (leg, toe) in stage.legs().into_iter().zip(TOES) {
                    let toe = stage.input_world(toe);
                    assert!(leg.lock.distance(toe) < EPS, "{}", row.name);
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        #[test]
        fn a_held_foot_reaches_its_weighted_lock_or_stretches_its_leg_straight_toward_it(
            yaw in -std::f32::consts::PI..std::f32::consts::PI,
            height in -1.0f32..1.0,
            heading in 0.0f32..std::f32::consts::TAU,
            slide in 0.0f32..0.18,
            weight in 0.1f32..=1.0,
        ) {
            let leg = 2.0 * HALF_LEG;
            let space = Affine3A::from_rotation_translation(
                Quat::from_rotation_y(yaw),
                Vec3::new(0.0, height, 0.0),
            );
            let mut stage = locked_in(locking().with_weight(weight), space);
            let by = Vec3::new(heading.cos(), 0.0, heading.sin()) * slide;

            stage.move_model(Affine3A::from_translation(by) * space);
            stage.step(DT);

            prop_assert_eq!(stage.phases(), [LockPhase::Locked; 2]);
            for ((lock, foot), hip) in stage.legs().into_iter().zip(FEET).zip(HIPS) {
                let goal = stage.input_world(foot) + (lock.lock - lock.animated) * weight;
                let hip = stage.output_world(hip);
                let reach = goal - hip;
                let shown = stage.output_world(foot);
                if reach.length() < leg - 1.0e-3 {
                    prop_assert!(shown.distance(goal) < 1.0e-3, "{shown} vs {goal}");
                } else if reach.length() > leg + 1.0e-3 {
                    let stretched = hip + reach.normalize() * leg;
                    prop_assert!(shown.distance(stretched) < 1.0e-3, "{shown} vs {stretched}");
                }
                let turned = angle(
                    rotation_of(stage.output_model(foot)),
                    rotation_of(stage.input_model(foot)),
                );
                prop_assert!(turned < 1.0e-3, "the foot turned {turned}");
            }
        }
    }

    #[test]
    fn a_plant_that_ends_releases_from_the_lock_and_eases_back() {
        let mut stage = locked(locking());
        let rest = stage.script.clone();
        let lock = stage.legs()[0].lock;

        stage.pose(rest.moved(Vec3::new(0.0, 0.0, 0.04)));
        stage.step(DT);
        let leg = stage.legs()[0];
        assert_eq!(leg.phase, LockPhase::Releasing);
        assert!(leg.lock.distance(lock) < EPS, "{:?} vs {lock:?}", leg.lock);
        assert!(
            leg.shown.distance(lock) < EPS,
            "{:?} vs {lock:?}",
            leg.shown
        );
        let releasing = stage.legs();
        stage.assert_toes_at(&releasing);
        let released = leg.shown - leg.animated;

        let mut reach = 0.09;
        stage.pose(rest.moved(Vec3::new(0.0, 0.0, reach)));
        stage.step(0.1);
        let leg = stage.legs()[0];
        assert_eq!(leg.phase, LockPhase::Releasing);
        let halved = leg.shown - leg.animated;
        assert!(
            halved.distance(released * 0.5) < 1.0e-3,
            "{halved:?} vs {released:?}"
        );

        let freed = (0..20).any(|_| {
            reach += 0.05;
            stage.pose(rest.moved(Vec3::new(0.0, 0.0, reach)));
            stage.step(0.1);
            stage.phases()[0] == LockPhase::Free
        });
        assert!(freed);
        let leg = stage.legs()[0];
        assert!(leg.shown.distance(leg.animated) < EPS);
    }

    #[test]
    fn a_foot_dragged_past_the_unlock_radius_waits_for_its_next_plant() {
        let mut stage = locked(locking());
        let rest = stage.script.clone();

        stage.move_model(Affine3A::from_translation(Vec3::new(0.25, 0.0, 0.0)));
        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Releasing; 2]);

        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Releasing; 2]);

        stage.pose(rest.moved(Vec3::new(0.0, 0.0, 0.1)));
        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Releasing; 2]);

        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Locked; 2]);
    }

    #[test]
    fn a_foot_planted_again_while_releasing_locks_where_it_is_shown() {
        for weight in [1.0, 0.5] {
            let mut stage = locked(locking().with_weight(weight));
            let rest = stage.script.clone();
            let old = stage.legs()[0].lock;

            stage.pose(rest.moved(Vec3::new(0.0, 0.0, 0.05)));
            stage.step(DT);
            assert_eq!(stage.phases(), [LockPhase::Releasing; 2], "weight {weight}");
            let released = stage.legs()[0].shown;

            stage.pose(rest.moved(Vec3::new(0.0, 0.0, 0.1)));
            stage.step(0.05);
            assert_eq!(stage.phases(), [LockPhase::Releasing; 2], "weight {weight}");
            let shown = stage.legs()[0].shown;
            assert!(shown.distance(released) > 1.0e-3, "weight {weight}");
            let toe = stage.output_world(TOES[0]);

            stage.step(DT);
            let leg = stage.legs()[0];
            assert_eq!(leg.phase, LockPhase::Locked, "weight {weight}");
            assert!(leg.shown.distance(shown) < EPS, "weight {weight}");
            let unweighted = leg.animated + (shown - leg.animated) / weight;
            assert!(leg.lock.distance(unweighted) < EPS, "weight {weight}");
            assert!(leg.lock.distance(old) > 1.0e-3, "weight {weight}");
            assert!(stage.output_world(TOES[0]).distance(toe) < EPS);

            stage.step(DT);
            assert!(stage.output_world(TOES[0]).distance(toe) < EPS);
        }
    }

    #[test]
    fn an_evaluate_with_no_time_keeps_the_plant_and_the_next_step_measures_from_the_old_contact() {
        let mut stage = locked(locking());
        let rest = stage.script.clone();

        stage.pose(rest.moved(Vec3::new(0.0, 0.0, 0.1)));
        stage.run();
        assert_eq!(stage.phases(), [LockPhase::Locked; 2]);

        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Releasing; 2]);
    }

    #[test]
    fn weight_zero_leaves_the_input_pose_and_a_zero_weight_param_plants_nothing() {
        let weight_param = name!("lock_weight");
        let node = locking()
            .with_weight_param(weight_param)
            .with_weight_halflife(0.0);

        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node.clone()).under(Affine3A::IDENTITY);
        stage.set_param(weight_param, 1.0_f32);
        stage.settle();
        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Locked; 2]);
        stage.move_model(Affine3A::from_translation(Vec3::new(0.02, 0.0, 0.0)));
        stage.set_param(weight_param, 0.0_f32);
        stage.step(DT);
        stage.assert_input_pose();
        assert_eq!(stage.phases(), [LockPhase::Releasing; 2]);

        let skeleton = rig();
        let script = Script::rest_of(&skeleton);
        let mut stage = Stage::new(skeleton, script, node);
        stage.set_param(weight_param, 0.0_f32);
        stage.settle();
        stage.step(DT);
        assert_eq!(stage.phases(), [LockPhase::Free; 2]);
    }
}
