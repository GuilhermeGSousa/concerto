use std::{thread, time::Duration};

use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    mask::{BoneMask, BoneMaskBranch},
    message::MessageType,
    node::{
        AnimationClipNode, AnimationNode, SlotEntry, SlotNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::AnimationPlugin,
};
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::entity::Entity;
use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
};
use concerto_foundation::time::Time;
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_mesh::Skeleton;
use concerto_name::{Name, name};
use concerto_physics::carried::Carried;
use concerto_physics::collider::Collider;
use concerto_physics::layers::CollisionLayers;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::ragdoll::{BodyPose, RagdollId};
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};
use concerto_ragdoll::messages::{Hit, Recovered};
use concerto_ragdoll::testing::{
    SpawnedHumanoid, humanoid_skeleton, reference_humanoid, spawn_humanoid,
};
use concerto_ragdoll::{
    BRACED, BodyHit, CharacterRagdoll, FlinchNode, HoldNode, LimpNode, MotorCommand, PartStrength,
    PhysicsAmount, RagdollBody, RagdollPlugin,
};
use glam::{Quat, Vec3};

const FRAME: Duration = Duration::from_millis(15);
const STRIDE: f32 = 0.025;
const FADE: f32 = 0.25;
const BLEND: f32 = 0.3;
const LIMP_BLEND: f32 = 0.5;
const LEFT_LEG: [&str; 3] = ["thigh_l", "calf_l", "foot_l"];

struct Swing {
    bone: &'static str,
    axis: Vec3,
    angle: f32,
}

const SWINGS: [Swing; 5] = [
    Swing {
        bone: "thigh_l",
        axis: Vec3::X,
        angle: 0.4,
    },
    Swing {
        bone: "thigh_r",
        axis: Vec3::X,
        angle: -0.4,
    },
    Swing {
        bone: "upperarm_l",
        axis: Vec3::Z,
        angle: 0.5,
    },
    Swing {
        bone: "upperarm_r",
        axis: Vec3::Z,
        angle: 0.5,
    },
    Swing {
        bone: "spine_02",
        axis: Vec3::Y,
        angle: 0.2,
    },
];

fn walk_clip(skeleton: &Skeleton) -> AnimationClip {
    let times = vec![0.0, 0.25, 0.5, 0.75, 1.0];
    let mut clip = AnimationClip::default();
    for swing in &SWINGS {
        let bone = skeleton.bone_index(Name::from(swing.bone)).unwrap();
        let turns = times
            .iter()
            .map(|time| {
                Quat::from_axis_angle(
                    swing.axis,
                    (time * std::f32::consts::TAU).sin() * swing.angle,
                )
                .to_array()
            })
            .collect::<Vec<_>>();
        clip.add_channel(
            skeleton.bone_ids[bone],
            AnimationChannel::new(
                times.clone(),
                AnimationChanelOutput::from_rotation(turns.into_iter()),
            ),
        );
    }
    clip
}

fn over_clip(node: impl AnimationNode, clip: &AssetHandle<AnimationClip>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node)
        .input(AnimationClipNode::new(clip.clone()));
    graph
}

#[derive(Default)]
struct Drift {
    stretch: f32,
    turn: f32,
}

impl Drift {
    fn max(self, other: Drift) -> Drift {
        Drift {
            stretch: self.stretch.max(other.stretch),
            turn: self.turn.max(other.turn),
        }
    }
}

struct Stage {
    app: App,
    humanoid: SpawnedHumanoid,
    capsule: Entity,
    skeleton: Skeleton,
    walk: AssetHandle<AnimationGraph>,
    root: Vec3,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(PhysicsPlugin)
            .register_plugin(AnimationPlugin)
            .register_plugin(RagdollPlugin);
        app.finish_plugin_build();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let reference = reference_humanoid();
        let skeleton = humanoid_skeleton();
        let clip = server.add(walk_clip(&skeleton));
        let skeleton_handle = server.add(reference.skeleton);
        let mut ragdoll = reference.ragdoll;
        ragdoll.skeleton = skeleton_handle.clone();
        let ragdoll = server.add(ragdoll);
        let left_leg = server.add(BoneMask::new().with_branch(BoneMaskBranch::new("thigh_l")));
        let walk = server.add(over_clip(
            SlotNode::new().with_entry(
                SlotEntry::new(name!("flinch"), FlinchNode::default())
                    .with_on(MessageType::of::<Hit>())
                    .with_out(MessageType::of::<Recovered>()),
            ),
            &clip,
        ));
        let hold = server.add(over_clip(
            HoldNode::new(BRACED)
                .with_mask(left_leg)
                .with_weight(name!("hurt"))
                .with_strength(name!("grip"))
                .with_fade(FADE),
            &clip,
        ));
        let limp = server.add(over_clip(LimpNode::default(), &clip));
        let machine =
            AnimationStateMachine::from_initial_state(name!("walk"), walk.clone(), |to| {
                to.to(
                    name!("hold"),
                    AnimationFSMTrigger::on_bool(name!("hold"), true),
                    BLEND,
                )
                .to(
                    name!("limp"),
                    AnimationFSMTrigger::on_bool(name!("limp"), true),
                    LIMP_BLEND,
                );
            })
            .state(name!("hold"), hold, |to| {
                to.to(
                    name!("walk"),
                    AnimationFSMTrigger::on_bool(name!("hold"), false),
                    BLEND,
                );
            })
            .state(name!("limp"), limp, |to| {
                to.to(
                    name!("walk"),
                    AnimationFSMTrigger::on_bool(name!("limp"), false),
                    LIMP_BLEND,
                );
            })
            .build();
        let graph = server.add(AnimationGraph::from_node(machine));
        app.update();

        let root = Vec3::new(0.0, 0.02, 0.0);
        let world = app.main_mut().world_mut();
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let capsule = world.spawn((
            RigidBody {
                allowed_dofs: AllowedDofs::TRANSLATION,
                ..RigidBody::default()
            },
            Collider::capsule(0.5, 0.3),
            CollisionLayers::character(),
            Transform::from_translation(root + Vec3::Y * 0.85),
        ));
        let humanoid = spawn_humanoid(world, skeleton_handle, Transform::from_translation(root));
        world.insert(
            (
                CharacterRagdoll::new(ragdoll).with_capsule(capsule),
                AnimationHandleComponent::new(graph),
            ),
            humanoid.skeleton,
        );
        Self {
            app,
            humanoid,
            capsule,
            skeleton,
            walk,
            root,
        }
    }

    fn frame(&mut self) -> f32 {
        self.root.x += STRIDE;
        let world = self.app.main_mut().world_mut();
        world
            .get_component_for_entity_mut::<Transform>(self.humanoid.skeleton)
            .unwrap()
            .translation = self.root;
        thread::sleep(FRAME);
        let time = world.get_resource_mut::<Time>().unwrap();
        time.update();
        let delta = time.delta().as_secs_f32();
        self.app.serve(FixedUpdate);
        self.app.serve(LateFixedUpdate);
        self.app.serve(Update);
        self.app.serve(LateUpdate);
        delta
    }

    fn walk(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame();
        }
    }

    fn player_mut(&mut self) -> &mut AnimationPlayer {
        self.app
            .main_mut()
            .world_mut()
            .get_component_for_entity_mut::<AnimationPlayer>(self.humanoid.skeleton)
            .unwrap()
    }

    fn body(&self) -> &RagdollBody {
        self.app
            .main()
            .world()
            .get_component_for_entity::<RagdollBody>(self.humanoid.skeleton)
            .unwrap()
    }

    fn bone(&self, name: &str) -> usize {
        self.skeleton.bone_index(Name::from(name)).unwrap()
    }

    fn part(&self, name: &str) -> usize {
        self.body().resolved().part_of(name).unwrap()
    }

    fn command(&self, name: &str) -> MotorCommand {
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.humanoid.skeleton)
            .unwrap()
            .drive_attributes()
            .get::<MotorCommand>(self.bone(name))
    }

    fn amount(&self, name: &str) -> f32 {
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.humanoid.skeleton)
            .unwrap()
            .attributes()
            .get::<PhysicsAmount>(self.bone(name))
            .0
    }

    fn slot_line(&self) -> String {
        let graphs = self
            .app
            .get_resource::<AssetStore<AnimationGraph>>()
            .unwrap();
        let mut line = String::new();
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.humanoid.skeleton)
            .unwrap()
            .for_each_node(graphs, |visit| {
                if visit.node.as_reflect().downcast_ref::<SlotNode>().is_some()
                    && let Some(described) = visit.node.describe(visit.state)
                {
                    line = described;
                }
            });
        line
    }

    fn drawn(&self, entity: Entity) -> Vec3 {
        self.app
            .main()
            .world()
            .get_component_for_entity::<GlobalTransform>(entity)
            .unwrap()
            .translation()
    }

    fn amounts(&self) -> Vec<f32> {
        let player = self
            .app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.humanoid.skeleton)
            .unwrap();
        self.body()
            .resolved()
            .bones()
            .iter()
            .map(|bone| player.attributes().get::<PhysicsAmount>(*bone).0)
            .collect()
    }

    fn fade_until(&mut self, done: impl Fn(&[f32]) -> bool) -> Drift {
        let mut before = self.amounts();
        let mut drift = Drift::default();
        for _ in 0..300 {
            if done(&before) {
                return drift;
            }
            let delta = self.frame();
            let after = self.amounts();
            for (now, then) in after.iter().zip(&before) {
                assert!(
                    (now - then).abs() <= delta / LIMP_BLEND + 0.01,
                    "a cross-fade moves every bone's amount over its blend, one went from {then} to {now} in {delta} s"
                );
            }
            let pelvis = self.amount("pelvis");
            if pelvis > 0.2 && pelvis < 0.8 {
                drift = drift.max(self.drift());
            }
            before = after;
        }
        panic!("the cross-fade did not end within 300 frames, its amounts are {before:?}");
    }

    fn trail(&self, name: &str) -> f32 {
        let body = self.body();
        let part = self.part(name);
        self.drawn(body.bone(part).unwrap())
            .distance(body.animated_pose()[part].translation)
    }

    fn drift(&self) -> Drift {
        let body = self.body();
        let alpha = self.app.get_resource::<Time>().unwrap().fixed_alpha();
        let simulated = body.interpolated_pose(alpha);
        let amounts = self.amounts();
        let stretch = self
            .skeleton
            .parents
            .iter()
            .enumerate()
            .filter_map(|(bone, parent)| {
                let parent = (*parent)? as usize;
                let root = body
                    .resolved()
                    .part_of(&self.skeleton.names[bone])
                    .is_some_and(|part| body.resolved().settings().parts[part].parent.is_none());
                if root {
                    return None;
                }
                let drawn = self
                    .drawn(self.humanoid.bones[bone])
                    .distance(self.drawn(self.humanoid.bones[parent]));
                Some((drawn - self.skeleton.rest_pose[bone].translation.length()).abs())
            })
            .fold(0.0, f32::max);
        let turn = (0..body.resolved().part_count())
            .map(|part| {
                let blended = body.animated_pose()[part]
                    .rotation
                    .slerp(simulated[part].rotation, amounts[part]);
                self.app
                    .main()
                    .world()
                    .get_component_for_entity::<GlobalTransform>(body.bone(part).unwrap())
                    .unwrap()
                    .rotation()
                    .angle_between(blended)
            })
            .fold(0.0, f32::max);
        Drift { stretch, turn }
    }

    fn reach(&self, parent: &str, child: &str) -> f32 {
        self.drawn(self.humanoid.bone(parent))
            .distance(self.drawn(self.humanoid.bone(child)))
    }

    fn offsets(&self) -> Vec<Vec3> {
        let mut pose = Vec::new();
        self.ragdoll_pose(&mut pose);
        pose.iter()
            .zip(self.body().animated_pose())
            .map(|(simulated, animated)| simulated.translation - animated.translation)
            .collect()
    }

    fn ragdoll_id(&self) -> RagdollId {
        *self
            .app
            .main()
            .world()
            .get_component_for_entity::<RagdollId>(self.humanoid.skeleton)
            .unwrap()
    }

    fn ragdoll_pose(&self, pose: &mut Vec<BodyPose>) {
        self.app
            .get_resource::<PhysicsState>()
            .unwrap()
            .ragdoll_pose(self.ragdoll_id(), pose);
    }

    fn velocity(&self, part: usize) -> Vec3 {
        let state = self.app.get_resource::<PhysicsState>().unwrap();
        state.linear_velocity(state.ragdoll_bodies(self.ragdoll_id())[part])
    }

    fn hit(&mut self, part: usize, impulse: Vec3) {
        let mut pose = Vec::new();
        self.ragdoll_pose(&mut pose);
        let character = self.humanoid.skeleton;
        self.app.main_mut().world_mut().trigger(BodyHit::new(
            character,
            part,
            pose[part].translation,
            impulse,
        ));
    }

    fn struck_speed(&mut self, part: usize, impulse: Vec3) -> f32 {
        let before = self.velocity(part);
        self.hit(part, impulse);
        let mut gained = 0.0f32;
        for _ in 0..30 {
            self.frame();
            gained = gained.max((before - self.velocity(part)).dot(-impulse.normalize()));
            if gained > 1.0 {
                break;
            }
        }
        gained
    }

    fn carried(&self) -> bool {
        self.app
            .main()
            .world()
            .get_component_for_entity::<Carried>(self.capsule)
            .is_some()
    }

    fn assert_animated(&self, when: &str) {
        let body = self.body();
        for part in 0..body.resolved().part_count() {
            assert!(
                body.is_kinematic(part),
                "{when}: part {:?} is kinematic",
                body.bone_name(part)
            );
            let off = self
                .drawn(body.bone(part).unwrap())
                .distance(body.animated_pose()[part].translation);
            assert!(
                off < 1e-4,
                "{when}: part {:?} is drawn on its animated pose, it was {off} m off",
                body.bone_name(part)
            );
        }
        assert!(!self.carried(), "{when}: the capsule is not carried");
    }
}

#[test]
fn behaviours_on_one_body_blend_per_limb_and_never_cut() {
    let mut stage = Stage::new();
    stage.walk(20);
    stage.assert_animated("walking with no ragdoll node");

    stage.player_mut().set_float_param(name!("hurt"), 0.5);
    stage.player_mut().set_bool_param(name!("hold"), true);
    stage.walk(30);
    let mut trail: f32 = 0.0;
    for _ in 0..20 {
        stage.frame();
        trail = trail.max(stage.trail("foot_l"));
    }
    for bone in LEFT_LEG {
        let amount = stage.amount(bone);
        assert!(
            (amount - 0.5).abs() < 0.01,
            "the held {bone} is drawn by the hold's weight, it drew {amount}"
        );
        assert!(
            !stage.body().is_kinematic(stage.part(bone)),
            "the held {bone} is dynamic"
        );
    }
    assert!(
        trail > 0.01,
        "the held left foot trails its animated place, it trailed at most {trail} m"
    );
    let body = stage.body();
    for part in 0..body.resolved().part_count() {
        let name = body.bone_name(part).unwrap();
        if !LEFT_LEG.contains(&&*name) {
            assert!(body.is_kinematic(part), "the unheld {name} is kinematic");
        }
    }
    assert_eq!(
        stage.amount("ball_l"),
        0.0,
        "a bone with no part takes no physics amount from a mask that covers it"
    );
    let toe = stage.reach("foot_l", "ball_l");
    assert!(
        (toe - Vec3::new(0.0, -0.06, 0.12).length()).abs() < 1e-3,
        "a bone with no part keeps its animated pose under its parent, the toe was {toe} m from the foot"
    );
    let braced = match reference_humanoid()
        .ragdoll
        .profile(BRACED)
        .unwrap()
        .strength_of("thigh_l")
    {
        PartStrength::Powered { stiffness, .. } => stiffness,
        other => panic!("the braced profile powers the thigh, it is {other:?}"),
    };
    let held = stage.command("thigh_l");
    assert!(
        (held.cover - 0.5).abs() < 1e-3 && (held.stiffness - braced).abs() < 1e-3 * braced,
        "the hold covers the thigh by its weight with the braced muscle at full strength, it asked {held:?} of a {braced} N·m/rad muscle"
    );
    stage.player_mut().set_float_param(name!("grip"), 0.5);
    stage.walk(30);
    let weaker = stage.command("thigh_l");
    assert!(
        (weaker.cover - 0.5).abs() < 1e-3
            && (weaker.stiffness - braced * 0.5).abs() < 1e-3 * braced,
        "a strength of 0.5 halves the hold's muscle, it asked {weaker:?} of a {braced} N·m/rad muscle"
    );
    stage.player_mut().set_float_param(name!("grip"), 1.0);

    stage.player_mut().set_float_param(name!("hurt"), 1.0);
    let mut before = stage.amount("thigh_l");
    let mut elapsed = 0.0;
    while elapsed < FADE + 0.2 {
        let delta = stage.frame();
        elapsed += delta;
        let now = stage.amount("thigh_l");
        assert!(
            (now - before).abs() <= delta / FADE + 0.01,
            "a weight raised in one frame moves the amount over the fade, it went from {before} to {now} in {delta} s"
        );
        before = now;
    }
    assert!(
        (before - 1.0).abs() < 0.01,
        "the amount reaches the raised weight, it is {before}"
    );

    stage.player_mut().set_bool_param(name!("hold"), false);
    stage.walk(30);
    stage.player_mut().set_bool_param(name!("limp"), true);
    stage.fade_until(|amounts| amounts.iter().all(|amount| *amount >= 1.0));
    stage.frame();
    let body = stage.body();
    assert!(
        (0..body.resolved().part_count()).all(|part| !body.is_kinematic(part)) && stage.carried(),
        "a limp body has every part dynamic and its capsule carried"
    );
    for part in 0..body.resolved().part_count() {
        let name = body.bone_name(part).unwrap();
        let command = stage.command(&name);
        assert!(
            command.cover == 1.0 && command.torque_limit == 0.0 && command.stiffness == 0.0,
            "a limp {name} is wholly covered with its motor off and the walk's stiffness kept, it asked {command:?}"
        );
    }
    let finger = stage.reach("hand_l", "index_l");
    assert!(
        (finger - 0.08).abs() < 1e-3,
        "a bone with no part keeps its animated pose under its limp parent, the finger was {finger} m from the hand"
    );
    stage.player_mut().set_bool_param(name!("limp"), false);
    let drift = stage.fade_until(|amounts| amounts.iter().all(|amount| *amount <= 0.0));
    assert!(
        drift.stretch < 1e-3,
        "a fallen body blended back keeps every bone's length, one changed by {} m",
        drift.stretch
    );
    assert!(
        drift.turn < 2e-3,
        "a fallen body blended back turns each part's bone by the slerp of its animated and simulated rotations, one was {} rad off",
        drift.turn
    );

    stage.player_mut().set_bool_param(name!("limp"), true);
    stage.fade_until(|amounts| amounts.iter().all(|amount| *amount >= 0.5));
    stage.player_mut().set_bool_param(name!("limp"), false);
    stage.fade_until(|amounts| amounts.iter().all(|amount| *amount <= 0.0));

    stage.player_mut().set_bool_param(name!("hold"), true);
    stage.walk(20);
    assert!(
        !stage.body().is_kinematic(stage.part("foot_l")),
        "the left foot is held again before the swap"
    );
    let walk = AnimationHandleComponent::new(stage.walk.clone());
    let skeleton = stage.humanoid.skeleton;
    stage.app.main_mut().world_mut().insert(walk, skeleton);
    stage.walk(30);
    stage.assert_animated("a second after the graph was swapped mid-hold");
}

#[test]
fn a_hit_flinches_from_a_slot_throws_the_part_back_and_the_body_returns_to_its_animation() {
    let mut stage = Stage::new();
    stage.walk(20);
    stage.assert_animated("walking before the hit");
    let shoulder = stage.part("upperarm_l");
    let feet = [stage.part("foot_l"), stage.part("foot_r")];

    stage.hit(shoulder, Vec3::new(0.0, 0.0, -30.0));
    let mut thrown = 0.0f32;
    for index in 0..10 {
        stage.frame();
        let offsets = stage.offsets();
        thrown = thrown.max(-offsets[shoulder].z);
        if index > 1 {
            for foot in feet {
                assert!(
                    stage.body().is_kinematic(foot) && offsets[foot].length() < 2.0 * STRIDE + 5e-3,
                    "a foot below the pelvis follows the animation during a flinch, it was {} m off",
                    offsets[foot].length()
                );
            }
        }
    }
    assert!(
        thrown > 0.03,
        "the hit shoulder is thrown back along the impulse, it went {thrown} m"
    );
    assert!(
        stage.amount("upperarm_l") > 0.99 && !stage.body().is_kinematic(shoulder),
        "the flinching shoulder is drawn from physics and dynamic"
    );

    let mut seconds = 0.0;
    while stage.amount("upperarm_l") > 0.9 {
        seconds += stage.frame();
        assert!(seconds < 2.0, "the flinch settles within two seconds");
    }
    stage.hit(shoulder, Vec3::new(0.0, 0.0, -30.0));
    stage.walk(2);
    assert!(
        stage.amount("upperarm_l") > 0.99,
        "a second hit restarts the flinch at full amount, it is {}",
        stage.amount("upperarm_l")
    );

    let mut lasted = 0.0;
    while stage.amounts().iter().any(|amount| *amount > 0.0) {
        lasted += stage.frame();
        assert!(lasted < 2.0, "the flinch ends within two seconds");
    }
    assert!(
        lasted > 0.7 && lasted < 1.2,
        "a flinch lasts its hold and settle from the last hit, it lasted {lasted} s"
    );
    stage.walk(3);
    stage.assert_animated("after the flinch settled");
    assert_eq!(
        stage.slot_line(),
        "no entry playing",
        "the flinch raises Recovered once it settles, and its slot entry ends on it"
    );

    stage.player_mut().set_bool_param(name!("limp"), true);
    stage.fade_until(|amounts| amounts.iter().all(|amount| *amount >= 1.0));
    let calf = stage.part("calf_l");
    let impulse = Vec3::new(0.0, 0.0, -100.0);
    assert!(
        stage.struck_speed(calf, impulse) > 1.0,
        "a hit on a limp body takes its impulse"
    );
}
