use std::{thread, time::Duration};

use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    mask::{BoneMask, BoneMaskBranch},
    node::{
        AnimationClipNode, AnimationNode,
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
use concerto_foundation::assets::{asset_server::AssetServer, handle::AssetHandle};
use concerto_foundation::time::Time;
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_mesh::Skeleton;
use concerto_name::{Name, name};
use concerto_physics::carried::Carried;
use concerto_physics::collider::Collider;
use concerto_physics::layers::CollisionLayers;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};
use concerto_ragdoll::testing::{
    SpawnedHumanoid, humanoid_skeleton, reference_humanoid, spawn_humanoid,
};
use concerto_ragdoll::{
    BRACED, CharacterRagdoll, HoldNode, LimpNode, PhysicsAmount, RagdollBody, RagdollPlugin,
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
        let walk = server.add(AnimationGraph::from_node(AnimationClipNode::new(
            clip.clone(),
        )));
        let hold = server.add(over_clip(
            HoldNode::new(BRACED)
                .with_mask(left_leg)
                .with_weight(name!("hurt"))
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

    fn trail(&self, name: &str) -> f32 {
        let body = self.body();
        let part = self.part(name);
        self.drawn(body.bone(part).unwrap())
            .distance(body.animated_pose()[part].translation)
    }

    fn reach(&self, parent: &str, child: &str) -> f32 {
        self.drawn(self.humanoid.bone(parent))
            .distance(self.drawn(self.humanoid.bone(child)))
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
    let finger = stage.reach("hand_l", "index_l");
    assert!(
        (toe - Vec3::new(0.0, -0.06, 0.12).length()).abs() < 1e-3 && (finger - 0.08).abs() < 1e-3,
        "bones with no part keep their animated pose under their parent, the toe was {toe} m from the foot and the finger {finger} m from the hand"
    );

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
    let mut before = stage.amounts();
    let mut limp = true;
    for _ in 0..300 {
        if limp && before.iter().all(|amount| *amount >= 1.0) {
            let body = stage.body();
            assert!(
                (0..body.resolved().part_count()).all(|part| !body.is_kinematic(part))
                    && stage.carried(),
                "a limp body has every part dynamic and its capsule carried"
            );
            stage.player_mut().set_bool_param(name!("limp"), false);
            limp = false;
        }
        let delta = stage.frame();
        let after = stage.amounts();
        for (now, then) in after.iter().zip(&before) {
            assert!(
                (now - then).abs() <= delta / LIMP_BLEND + 0.01,
                "a cross-fade moves every bone's amount over its blend, one went from {then} to {now} in {delta} s"
            );
        }
        before = after;
        if !limp && before.iter().all(|amount| *amount <= 0.0) {
            break;
        }
    }
    assert!(
        !limp && before.iter().all(|amount| *amount <= 0.0),
        "the body went limp and came back to its walk"
    );

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
