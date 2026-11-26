use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    message::AnimationMessage,
    node::{
        AnimationClipNode, AnimationNode, NodeOutcome,
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
use concerto_ecs::resource::{ResMut, Resource};
use concerto_ecs::signal::On;
use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_name::{Name, name};
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::testing::{
    SpawnedHumanoid, humanoid_skeleton, reference_humanoid, spawn_humanoid,
};
use concerto_ragdoll::{
    BalanceNode, BalanceSettings, BalanceState, BodyDrive, BodySense, BodySettled, BodyState,
    CharacterRagdoll, LimpNode, PhysicalState, RagdollBody, RagdollPlugin, StaggerNode,
    StaggerState,
};
use glam::{Quat, Vec3};

#[derive(Resource, Default)]
struct Settles(usize);

fn count_settles(_: On<BodySettled>, mut settles: ResMut<Settles>) {
    settles.0 += 1;
}

type GraphBuilder = Box<dyn FnOnce(&AssetServer) -> AnimationGraph>;

pub struct Stage {
    pub app: App,
    pub humanoid: SpawnedHumanoid,
    pub character: Entity,
    pub pose: Vec<JointPose>,
}

impl Stage {
    pub fn new() -> Self {
        Self::placed(Transform::default())
    }

    pub fn placed(placement: Transform) -> Self {
        Self::build(placement, None)
    }

    /// A body whose animation graph balances it with `node` over the standing pose, moving to the state `steady` when it succeeds and `fallen`, where it goes limp, when it gives up.
    pub fn balancing(node: BalanceNode) -> Self {
        Self::graphed(move |server| balance_graph(server, node))
    }

    /// A body whose animation graph is `build`'s.
    pub fn graphed(build: impl FnOnce(&AssetServer) -> AnimationGraph + 'static) -> Self {
        Self::build(Transform::default(), Some(Box::new(build)))
    }

    fn build(placement: Transform, graph: Option<GraphBuilder>) -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(PhysicsPlugin);
        if graph.is_some() {
            app.register_plugin(AnimationPlugin);
        }
        app.register_plugin(RagdollPlugin);
        app.add_listener(count_settles)
            .insert_resource(Settles::default());
        app.finish_plugin_build();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let reference = reference_humanoid();
        let skeleton = server.add(reference.skeleton);
        let mut asset = reference.ragdoll;
        asset.skeleton = skeleton.clone();
        let asset = server.add(asset);
        let graph = graph.map(|build| server.add(build(&server)));
        app.update();
        let world = app.main_mut().world_mut();
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let humanoid = spawn_humanoid(world, skeleton, placement);
        let character = humanoid.skeleton;
        world.insert(
            (
                CharacterRagdoll::new(asset),
                BodyDrive::default(),
                BodySense::default(),
                PhysicalState::default(),
            ),
            character,
        );
        if let Some(graph) = graph {
            world.insert(AnimationHandleComponent::new(graph), character);
        }
        let mut stage = Self {
            app,
            humanoid,
            character,
            pose: humanoid_skeleton().rest_pose.to_vec(),
        };
        stage.run(5);
        stage
    }

    /// Holds the arms half down and the legs apart.
    pub fn spread_limbs(&mut self) {
        self.turn(&[
            ("upperarm_l", -0.9),
            ("upperarm_r", 0.9),
            ("thigh_l", 0.35),
            ("thigh_r", -0.35),
        ]);
    }

    fn turn(&mut self, bones: &[(&str, f32)]) {
        let skeleton = humanoid_skeleton();
        for &(bone, angle) in bones {
            let index = skeleton.bone_index(bone.into()).unwrap();
            self.pose[index].rotation = Quat::from_rotation_z(angle);
        }
    }

    pub fn run(&mut self, frames: usize) {
        for _ in 0..frames {
            let world = self.app.main_mut().world_mut();
            for (bone, local) in self.humanoid.bones.iter().zip(self.pose.iter()) {
                let transform = world
                    .get_component_for_entity_mut::<Transform>(*bone)
                    .unwrap();
                transform.translation = local.translation;
                transform.rotation = local.rotation;
            }
            self.app.serve(FixedUpdate);
            self.app.serve(LateFixedUpdate);
            self.app.serve(Update);
            self.app.serve(LateUpdate);
        }
    }

    pub fn push(&mut self, bone: &str, impulse: Vec3) {
        let world = self.app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(self.character)
            .unwrap();
        let part = world
            .get_component_for_entity::<RagdollBody>(self.character)
            .unwrap()
            .resolved()
            .part_of(bone)
            .unwrap();
        let state = self.app.get_resource_mut::<PhysicsState>().unwrap();
        let body = state.ragdoll_bodies(ragdoll)[part];
        state.add_impulse(body, impulse);
    }

    pub fn part(&self, bone: &str) -> Vec3 {
        let world = self.app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(self.character)
            .unwrap();
        let part = world
            .get_component_for_entity::<RagdollBody>(self.character)
            .unwrap()
            .resolved()
            .part_of(bone)
            .unwrap();
        let mut pose = Vec::new();
        self.app
            .get_resource::<PhysicsState>()
            .unwrap()
            .ragdoll_pose(ragdoll, &mut pose);
        pose[part].translation
    }

    pub fn state(&self) -> BodyState {
        self.app
            .main()
            .world()
            .get_component_for_entity::<PhysicalState>(self.character)
            .unwrap()
            .state
    }

    pub fn settles(&self) -> usize {
        self.app.get_resource::<Settles>().unwrap().0
    }

    fn player(&self) -> &AnimationPlayer {
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.character)
            .unwrap()
    }

    fn in_state(&self, state: Name) -> bool {
        let graphs = self
            .app
            .get_resource::<AssetStore<AnimationGraph>>()
            .unwrap();
        self.player().in_state(graphs, &state)
    }

    pub fn send(&mut self, message: impl AnimationMessage) {
        self.app
            .main_mut()
            .world_mut()
            .get_component_for_entity_mut::<AnimationPlayer>(self.character)
            .unwrap()
            .send(message);
    }

    pub fn outcome(&self) -> Option<NodeOutcome> {
        if self.in_state(name!("steady")) {
            Some(NodeOutcome::Succeeded)
        } else if self.in_state(name!("fallen")) {
            Some(NodeOutcome::Failed)
        } else {
            None
        }
    }

    pub fn stepping(&self) -> bool {
        let graphs = self
            .app
            .get_resource::<AssetStore<AnimationGraph>>()
            .unwrap();
        let mut stepping = false;
        self.player()
            .for_each_state::<BalanceNode>(graphs, |_, state: &BalanceState, _| {
                stepping |= state.stepping();
            });
        self.player()
            .for_each_state::<StaggerNode>(graphs, |_, state: &StaggerState, _| {
                stepping |= state.stepping();
            });
        stepping
    }

    pub fn insert<C: concerto_ecs::Component>(&mut self, component: C) {
        self.app
            .main_mut()
            .world_mut()
            .insert(component, self.character);
    }
}

pub fn standing_clip(skeleton: &Skeleton) -> AnimationClip {
    let pelvis = skeleton.bone_index(Name::from("pelvis")).unwrap();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        skeleton.bone_ids[pelvis],
        AnimationChannel::new(
            vec![0.0, 1.0],
            AnimationChanelOutput::from_rotation(
                [Quat::IDENTITY.to_array(), Quat::IDENTITY.to_array()].into_iter(),
            ),
        ),
    );
    clip
}

pub fn over_clip(node: impl AnimationNode, clip: &AssetHandle<AnimationClip>) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node)
        .input(AnimationClipNode::new(clip.clone()));
    graph
}

fn balance_graph(server: &AssetServer, node: BalanceNode) -> AnimationGraph {
    let clip = server.add(standing_clip(&humanoid_skeleton()));
    let balance = server.add(over_clip(node, &clip));
    let steady = server.add(AnimationGraph::from_node(AnimationClipNode::new(
        clip.clone(),
    )));
    let fallen = server.add(over_clip(LimpNode::default(), &clip));
    AnimationGraph::from_node(
        AnimationStateMachine::from_initial_state(name!("balance"), balance, |to| {
            to.to(name!("steady"), AnimationFSMTrigger::OnSucceeded, 0.0)
                .to(name!("fallen"), AnimationFSMTrigger::OnFailed, 0.0);
        })
        .state(name!("steady"), steady, |_| {})
        .state(name!("fallen"), fallen, |_| {})
        .build(),
    )
}

#[derive(Debug)]
struct Pushed {
    outcome: Option<NodeOutcome>,
    steps: u32,
    travelled: Vec3,
    pelvis_height: f32,
}

fn pushed(balance: BalanceNode, impulse: Vec3, seconds: f32) -> Pushed {
    let mut stage = Stage::balancing(balance);
    stage.run(15);
    let start = stage.part("pelvis");
    stage.push("spine_03", impulse);
    let mut steps = 0;
    let mut stepping = false;
    let mut pelvis = start;
    for _ in 0..(seconds / 0.033) as usize {
        if stage.outcome().is_some() {
            break;
        }
        pelvis = stage.part("pelvis");
        if stage.stepping() && !stepping {
            steps += 1;
        }
        stepping = stage.stepping();
        stage.run(1);
    }
    Pushed {
        outcome: stage.outcome(),
        steps,
        travelled: pelvis - start,
        pelvis_height: pelvis.y,
    }
}

#[test]
fn a_balancing_body_stands_absorbs_a_push_steps_to_catch_itself_or_gives_up() {
    let patient = BalanceNode::default()
        .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX));
    let standing = pushed(patient, Vec3::ZERO, 10.0);
    assert_eq!(standing.outcome, None, "an unpushed body keeps balancing");
    assert_eq!(standing.steps, 0);
    assert!(
        standing.travelled.length() < 0.05,
        "an unpushed body stands where it is for ten seconds, it drifted {}",
        standing.travelled
    );

    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let light = pushed(BalanceNode::default(), direction * 12.0, 10.0);
        assert_eq!(
            (light.outcome, light.steps),
            (Some(NodeOutcome::Succeeded), 0),
            "a light push towards {direction} is absorbed without a step and the graph moves on to its steady state: {light:?}"
        );

        let medium = pushed(BalanceNode::default(), direction * 70.0, 10.0);
        assert!(
            medium.outcome == Some(NodeOutcome::Succeeded)
                && (1..=2).contains(&medium.steps)
                && medium.pelvis_height > 0.9
                && medium.travelled.dot(direction) > 0.0,
            "a medium push towards {direction} takes a step or two that way and the body ends standing: {medium:?}"
        );

        let hard = pushed(BalanceNode::default(), direction * 300.0, 10.0);
        assert_eq!(
            hard.outcome,
            Some(NodeOutcome::Failed),
            "a hard push towards {direction} ends in giving up, and the graph moves on to its fallen state: {hard:?}"
        );
    }
}
