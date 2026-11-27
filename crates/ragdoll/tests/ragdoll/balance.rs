use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    node::{
        AnimationClipNode, AnimationNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine, AnimationStateMachineState},
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
    Balance, BalanceNode, BalanceSettings, BalanceState, BehaviourEnded, BodyDrive, BodySense,
    BodySettled, BodyState, CharacterRagdoll, LimpNode, Outcome, PhysicalState, RagdollBody,
    RagdollMode, RagdollPlugin,
    messages::{Steadied, Toppled},
};
use glam::{Quat, Vec3};

#[derive(Resource, Default)]
struct Ended(Vec<BehaviourEnded>);

fn record(on: On<BehaviourEnded>, mut ended: ResMut<Ended>) {
    ended.0.push(*on.signal());
}

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
    entries: Vec<Name>,
}

impl Stage {
    pub fn new() -> Self {
        Self::placed(Transform::default())
    }

    pub fn placed(placement: Transform) -> Self {
        Self::build(placement, None)
    }

    /// A body in `RagdollMode::Graph` whose animation graph balances it with `node` over the standing pose, moving to the state `steady` when it succeeds and `fallen`, where it goes limp, when it gives up.
    pub fn balancing(node: BalanceNode) -> Self {
        Self::build(
            Transform::default(),
            Some(Box::new(move |server| balance_graph(server, node))),
        )
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
        app.add_listener(record).insert_resource(Ended::default());
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
        match graph {
            Some(graph) => world.insert(
                (
                    CharacterRagdoll::new(asset).with_mode(RagdollMode::Graph),
                    BodySense::default(),
                    AnimationHandleComponent::new(graph),
                ),
                character,
            ),
            None => world.insert(
                (
                    CharacterRagdoll::new(asset),
                    BodyDrive::default(),
                    BodySense::default(),
                    PhysicalState::default(),
                ),
                character,
            ),
        }
        let mut stage = Self {
            app,
            humanoid,
            character,
            pose: humanoid_skeleton().rest_pose.to_vec(),
            entries: Vec::new(),
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
            if let Some(state) = self.current_state()
                && self.entries.last() != Some(&state)
            {
                self.entries.push(state);
            }
        }
    }

    fn player(&self) -> Option<&AnimationPlayer> {
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.character)
    }

    fn current_state(&self) -> Option<Name> {
        let graphs = self.app.get_resource::<AssetStore<AnimationGraph>>()?;
        let mut current = None;
        self.player()?.for_each_state::<AnimationStateMachine>(
            graphs,
            |_, state: &AnimationStateMachineState, _| {
                current = current.or(state.current_state_name());
            },
        );
        current
    }

    /// The states the graph's machine has been in, in the order it entered them.
    pub fn entries(&self) -> &[Name] {
        &self.entries
    }

    fn node_stepping(&self) -> bool {
        let (Some(graphs), Some(player)) = (
            self.app.get_resource::<AssetStore<AnimationGraph>>(),
            self.player(),
        ) else {
            return false;
        };
        let mut stepping = false;
        player.for_each_state::<BalanceNode>(graphs, |_, state: &BalanceState, _| {
            stepping |= state.stepping();
        });
        stepping
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

    pub fn ended(&self) -> Vec<BehaviourEnded> {
        self.app.get_resource::<Ended>().unwrap().0.clone()
    }

    pub fn balance(&self) -> Option<Balance> {
        self.app
            .main()
            .world()
            .get_component_for_entity::<Balance>(self.character)
            .cloned()
    }

    pub fn insert<C: concerto_ecs::Component>(&mut self, component: C) {
        self.app
            .main_mut()
            .world_mut()
            .insert(component, self.character);
    }
}

fn standing_clip(skeleton: &Skeleton) -> AnimationClip {
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

fn over_clip(node: impl AnimationNode, clip: &AssetHandle<AnimationClip>) -> AnimationGraph {
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
            to.to(
                name!("steady"),
                AnimationFSMTrigger::on_message::<Steadied>(),
                0.0,
            )
            .to(
                name!("fallen"),
                AnimationFSMTrigger::on_message::<Toppled>(),
                0.0,
            );
        })
        .state(name!("steady"), steady, |_| {})
        .state(name!("fallen"), fallen, |_| {})
        .build(),
    )
}

#[derive(Debug)]
struct Watched {
    steps: u32,
    travelled: Vec3,
    pelvis_height: f32,
}

fn watch(
    stage: &mut Stage,
    impulse: Vec3,
    seconds: f32,
    ended: impl Fn(&Stage) -> bool,
    stepping: impl Fn(&Stage) -> bool,
) -> Watched {
    stage.run(15);
    let start = stage.part("pelvis");
    stage.push("spine_03", impulse);
    let mut steps = 0;
    let mut was_stepping = false;
    let mut pelvis = start;
    for _ in 0..(seconds / 0.033) as usize {
        if ended(stage) {
            break;
        }
        pelvis = stage.part("pelvis");
        let now_stepping = stepping(stage);
        if now_stepping && !was_stepping {
            steps += 1;
        }
        was_stepping = now_stepping;
        stage.run(1);
    }
    Watched {
        steps,
        travelled: pelvis - start,
        pelvis_height: pelvis.y,
    }
}

#[derive(Debug)]
struct Pushed {
    outcome: Option<Outcome>,
    steps: u32,
    travelled: Vec3,
    pelvis_height: f32,
    state: BodyState,
}

fn pushed(balance: Balance, impulse: Vec3, seconds: f32) -> Pushed {
    let mut stage = Stage::new();
    stage.insert(balance);
    let watched = watch(
        &mut stage,
        impulse,
        seconds,
        |stage| !stage.ended().is_empty(),
        |stage| stage.balance().is_some_and(|balance| balance.stepping()),
    );
    Pushed {
        outcome: stage.ended().first().map(|ended| ended.outcome),
        steps: watched.steps,
        travelled: watched.travelled,
        pelvis_height: watched.pelvis_height,
        state: stage.state(),
    }
}

#[derive(Debug)]
struct Graphed {
    entries: Vec<Name>,
    watched: Watched,
}

fn graphed(node: BalanceNode, impulse: Vec3, seconds: f32) -> Graphed {
    let mut stage = Stage::balancing(node);
    let watched = watch(
        &mut stage,
        impulse,
        seconds,
        |stage| stage.entries().len() > 1,
        Stage::node_stepping,
    );
    stage.run(30);
    Graphed {
        entries: stage.entries().to_vec(),
        watched,
    }
}

#[test]
fn a_balancing_body_stands_absorbs_a_push_steps_to_catch_itself_or_gives_up() {
    let patient = Balance::default().with_settle_seconds(f32::MAX);
    let standing = pushed(patient, Vec3::ZERO, 10.0);
    assert_eq!(standing.outcome, None, "an unpushed body keeps balancing");
    assert_eq!(standing.steps, 0);
    assert!(
        standing.travelled.length() < 0.05,
        "an unpushed body stands where it is for ten seconds, it drifted {}",
        standing.travelled
    );
    assert_eq!(standing.state, BodyState::Reacting);

    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let light = pushed(Balance::default(), direction * 12.0, 10.0);
        assert_eq!(
            (light.outcome, light.steps),
            (Some(Outcome::Succeeded), 0),
            "a light push towards {direction} is absorbed without a step: {light:?}"
        );
        assert_eq!(light.state, BodyState::Animated);

        let medium = pushed(Balance::default(), direction * 70.0, 10.0);
        assert!(
            medium.outcome == Some(Outcome::Succeeded)
                && (1..=2).contains(&medium.steps)
                && medium.pelvis_height > 0.9
                && medium.travelled.dot(direction) > 0.0,
            "a medium push towards {direction} takes a step or two that way and the body ends standing: {medium:?}"
        );

        let hard = pushed(Balance::default(), direction * 300.0, 10.0);
        assert_eq!(
            (hard.outcome, hard.state),
            (Some(Outcome::GaveUp), BodyState::Falling),
            "a hard push towards {direction} ends in giving up: {hard:?}"
        );
    }

    let patient = BalanceNode::default()
        .with_settings(BalanceSettings::default().with_settle_seconds(f32::MAX));
    let standing = graphed(patient, Vec3::ZERO, 10.0);
    assert_eq!(
        standing.entries,
        [name!("balance")],
        "an unpushed body keeps balancing in its graph"
    );
    assert_eq!(standing.watched.steps, 0);
    assert!(
        standing.watched.travelled.length() < 0.05,
        "an unpushed body balanced by its graph stands where it is, it drifted {}",
        standing.watched.travelled
    );

    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let light = graphed(BalanceNode::default(), direction * 12.0, 10.0);
        assert!(
            light.entries == [name!("balance"), name!("steady")] && light.watched.steps == 0,
            "a light push towards {direction} is absorbed without a step and the graph moves on to its steady state once and stays: {light:?}"
        );

        let medium = graphed(BalanceNode::default(), direction * 70.0, 10.0);
        assert!(
            medium.entries == [name!("balance"), name!("steady")]
                && (1..=2).contains(&medium.watched.steps)
                && medium.watched.pelvis_height > 0.9
                && medium.watched.travelled.dot(direction) > 0.0,
            "a medium push towards {direction} takes a step or two that way, the body ends standing and the graph steady: {medium:?}"
        );

        let hard = graphed(BalanceNode::default(), direction * 300.0, 10.0);
        assert_eq!(
            hard.entries,
            [name!("balance"), name!("fallen")],
            "a hard push towards {direction} ends in giving up, and the graph moves on to its fallen state once and stays: {hard:?}"
        );
    }
}
