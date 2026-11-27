use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    message::AnimationMessage,
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
use concerto_ecs::signal::{
    On,
    lifecycle::Add,
    listener::{IntoListener, Listener},
};
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
    BalanceNode, BalanceSettings, BalanceState, BodyDown, BodySettled, CharacterRagdoll, Dead,
    LimpNode, MotorCommand, RagdollBody, RagdollPlugin, StaggerNode, StaggerState,
    messages::{Steadied, Toppled},
};
use concerto_reflect::Reflect;
use glam::{Quat, Vec3};

#[derive(Resource, Default)]
struct Settles(usize);

#[derive(Resource, Default)]
struct Downs(usize);

#[derive(Resource, Default)]
struct Deaths(usize);

fn count_deaths(_: On<Add<Dead>>, mut deaths: ResMut<Deaths>) {
    deaths.0 += 1;
}

fn count_downs() -> Listener<BodyDown> {
    (|_: On<BodyDown>, mut downs: ResMut<Downs>| {
        downs.0 += 1;
    })
    .into_listener()
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Steady,
    Fallen,
}

impl Stage {
    /// A body placed at `placement` whose animation graph is `build`'s.
    pub fn placed_graphed(
        placement: Transform,
        build: impl FnOnce(&AssetServer) -> AnimationGraph + 'static,
    ) -> Self {
        Self::build(placement, Some(Box::new(build)))
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
            .add_listener(count_deaths)
            .insert_resource(Deaths::default())
            .insert_resource(Settles::default())
            .insert_resource(Downs::default());
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
        world.register_component::<Listener<BodyDown>>();
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let humanoid = spawn_humanoid(world, skeleton, placement);
        let character = humanoid.skeleton;
        world.insert((CharacterRagdoll::new(asset), count_downs()), character);
        if let Some(graph) = graph {
            world.insert(AnimationHandleComponent::new(graph), character);
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

    fn current_state(&self) -> Option<Name> {
        let graphs = self.app.get_resource::<AssetStore<AnimationGraph>>()?;
        let player = self
            .app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.character)?;
        let mut current = None;
        player.for_each_state::<AnimationStateMachine>(
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

    pub fn animated(&self) -> bool {
        self.app
            .main()
            .world()
            .get_component_for_entity::<RagdollBody>(self.character)
            .unwrap()
            .is_animated(self.player())
    }

    pub fn settles(&self) -> usize {
        self.app.get_resource::<Settles>().unwrap().0
    }

    pub fn deaths(&self) -> usize {
        self.app.get_resource::<Deaths>().unwrap().0
    }

    pub fn downs(&self) -> usize {
        self.app.get_resource::<Downs>().unwrap().0
    }

    /// What the graph last asked of the motor on `bone`.
    pub fn motor(&self, bone: &str) -> MotorCommand {
        let index = humanoid_skeleton().bone_index(bone.into()).unwrap();
        self.player().drive_attributes().get::<MotorCommand>(index)
    }

    pub fn set_param<T: Reflect>(&mut self, key: Name, value: T) {
        self.app
            .main_mut()
            .world_mut()
            .get_component_for_entity_mut::<AnimationPlayer>(self.character)
            .unwrap()
            .set_param(key, value);
    }

    fn player(&self) -> &AnimationPlayer {
        self.app
            .main()
            .world()
            .get_component_for_entity::<AnimationPlayer>(self.character)
            .unwrap()
    }

    pub fn in_state(&self, state: Name) -> bool {
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

    pub fn outcome(&self) -> Option<Ended> {
        if self.in_state(name!("steady")) {
            Some(Ended::Steady)
        } else if self.in_state(name!("fallen")) {
            Some(Ended::Fallen)
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
struct Pushed {
    entries: Vec<Name>,
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
    stage.run(30);
    Pushed {
        entries: stage.entries().to_vec(),
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
    assert_eq!(
        standing.entries,
        [name!("balance")],
        "an unpushed body keeps balancing"
    );
    assert_eq!(standing.steps, 0);
    assert!(
        standing.travelled.length() < 0.05,
        "an unpushed body stands where it is for ten seconds, it drifted {}",
        standing.travelled
    );

    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let light = pushed(BalanceNode::default(), direction * 12.0, 10.0);
        assert_eq!(
            light.entries,
            [name!("balance"), name!("steady")],
            "a light push towards {direction} is absorbed and the graph moves on to its steady state once and stays: {light:?}"
        );
        assert_eq!(
            light.steps, 0,
            "a light push towards {direction} is absorbed without a step: {light:?}"
        );

        let medium = pushed(BalanceNode::default(), direction * 70.0, 10.0);
        assert!(
            medium.entries == [name!("balance"), name!("steady")]
                && (1..=2).contains(&medium.steps)
                && medium.pelvis_height > 0.9
                && medium.travelled.dot(direction) > 0.0,
            "a medium push towards {direction} takes a step or two that way and the body ends standing: {medium:?}"
        );

        let hard = pushed(BalanceNode::default(), direction * 300.0, 10.0);
        assert_eq!(
            hard.entries,
            [name!("balance"), name!("fallen")],
            "a hard push towards {direction} ends in giving up, and the graph moves on to its fallen state once and stays: {hard:?}"
        );
    }
}
