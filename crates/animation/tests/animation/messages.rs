use concerto_animation::{
    evaluation::AnimationGraphContext,
    graph::AnimationGraph,
    message::{AnimationMessage, SendAnimationMessage},
    node::{
        AnimationNode, InputSlots, NodeOutcome, ReflectAnimationNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{AnimationAppExt, AnimationPlugin},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity, ResMut,
    component::scene::SceneEntityRef,
    reflect::AppTypeRegistry,
    resource::Resource,
    signal::{EntitySignal, On, Signal, listener::IntoListener},
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    world::World,
};
use concerto_foundation::{
    assets::{Asset, AssetId, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::Transform,
};
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
use concerto_name::{Name, name};
use concerto_reflect::Reflect;
use glam::Mat4;
use uuid::Uuid;

#[derive(Clone, Default, Reflect)]
struct Poke {
    strength: f32,
}

impl AnimationMessage for Poke {}

#[derive(Clone, Default, Reflect)]
struct Other {}

impl AnimationMessage for Other {}

struct Marker(u32);

#[derive(Clone)]
struct Counted {
    count: u32,
}

impl Signal for Counted {}

impl EntitySignal for Counted {}

#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct Counter {}

#[derive(Default)]
struct Count {
    received: u32,
    marker: Option<u32>,
}

impl AnimationNode for Counter {
    type State = Count;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::None
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> Count {
        Count::default()
    }

    fn update(&self, _: &mut Count, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut Count,
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        _: &[EvaluatedFrame],
        _: &mut FramePool,
        _: &mut AnimationFrame,
    ) {
    }

    fn receive(&self, state: &mut Count, message: &dyn Reflect, context: &AnimationGraphContext) {
        if message
            .downcast_ref::<Poke>()
            .is_some_and(|poke| poke.strength > 0.0)
        {
            state.received += 1;
            state.marker = context.extra::<Marker>().map(|marker| marker.0);
            context.emit(Counted {
                count: state.received,
            });
        }
    }

    fn outcome(&self, state: &Count) -> Option<NodeOutcome> {
        (state.received >= 2).then_some(NodeOutcome::Succeeded)
    }

    fn is_finished(&self, _: &Count) -> bool {
        false
    }

    fn reset(&self, state: &mut Count) {
        *state = Count::default();
    }
}

fn insert<T: Asset>(app: &mut App, asset: T) -> AssetHandle<T> {
    let id = AssetId::new();
    app.main_mut()
        .world_mut()
        .get_resource_mut::<AssetStore<T>>()
        .unwrap()
        .insert(id, asset);
    AssetHandle::weak(id)
}

fn graph(app: &mut App, mut graph: AnimationGraph) -> AssetHandle<AnimationGraph> {
    {
        let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
        graph.make_live(&registry, None).unwrap();
    }
    insert(app, graph)
}

fn stored(graph: AnimationGraph) -> AnimationGraph {
    bincode::deserialize(&bincode::serialize(&graph).unwrap()).unwrap()
}

struct Heard {
    entity: Entity,
    count: u32,
}

#[derive(Default, Resource)]
struct HeardCounts(Vec<Heard>);

fn hear_counted(on: On<Counted>, mut heard: ResMut<HeardCounts>) {
    heard.0.push(Heard {
        entity: on.entity(),
        count: on.signal().count,
    });
}

#[derive(Debug, PartialEq)]
struct CounterSaw {
    received: u32,
    marker: Option<u32>,
}

fn saw(received: u32) -> CounterSaw {
    CounterSaw {
        received,
        marker: Some(7),
    }
}

struct Stage {
    app: App,
    character: Entity,
    bodiless: Entity,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.insert_resource(HeardCounts::default());
        app.register_plugin(AnimationPlugin);
        app.register_animation_node::<Counter>();
        app.register_animation_message::<Poke>();
        app.register_animation_message::<Other>();
        app.main_mut().world_mut().register_component::<Transform>();
        let idle = graph(&mut app, AnimationGraph::new());
        let poked = graph(&mut app, AnimationGraph::from_node(Counter {}));
        let machine_poked = poked.clone();
        let elsewhere = graph(&mut app, AnimationGraph::new());
        let machine =
            AnimationStateMachine::from_initial_state(name!("Idle"), idle, |transition| {
                transition.to(
                    name!("Poked"),
                    AnimationFSMTrigger::on_message::<Poke>(),
                    0.1,
                );
            })
            .state(name!("Poked"), machine_poked, |transition| {
                transition.to(name!("Idle"), AnimationFSMTrigger::OnSucceeded, 0.0);
                transition.to(
                    name!("Elsewhere"),
                    AnimationFSMTrigger::on_message::<Other>(),
                    0.1,
                );
            })
            .state(name!("Elsewhere"), elsewhere, |_| {})
            .build();
        let machine = graph(&mut app, stored(AnimationGraph::from_node(machine)));
        let skeleton = insert(
            &mut app,
            Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["root".into()],
                vec![JointPose::IDENTITY],
                vec![Uuid::from_u128(1)],
            )
            .unwrap(),
        );
        let world = app.main_mut().world_mut();
        let bone = world.spawn(Transform::IDENTITY);
        let character = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(machine),
            SkeletonComponent::new(skeleton, vec![SceneEntityRef::Entity(bone)], None),
            hear_counted.into_listener(),
        ));
        let bodiless = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(poked),
            hear_counted.into_listener(),
        ));
        Self {
            app,
            character,
            bodiless,
        }
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn start(&mut self) {
        let world = self.world();
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world
            .get_resource_mut::<Time>()
            .unwrap()
            .set_time_scale(0.0);
        self.frame();
        self.frame();
    }

    fn frame(&mut self) {
        let world = self.world();
        world.get_resource_mut::<Time>().unwrap().update();
        world.run_schedule(LateUpdate);
    }

    fn player(&mut self) -> &mut AnimationPlayer {
        let character = self.character;
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(character)
            .unwrap()
    }

    fn in_state(&mut self, state: Name) -> bool {
        let character = self.character;
        let world = self.world();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        world
            .get_component_for_entity::<AnimationPlayer>(character)
            .unwrap()
            .in_state(graphs, &state)
    }

    fn counters(&mut self) -> Vec<CounterSaw> {
        let character = self.character;
        let world = self.world();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let mut counts = Vec::new();
        world
            .get_component_for_entity::<AnimationPlayer>(character)
            .unwrap()
            .for_each_state::<Counter>(graphs, |_, state, _| {
                counts.push(CounterSaw {
                    received: state.received,
                    marker: state.marker,
                });
            });
        counts
    }

    fn heard_by(&mut self, entity: Entity) -> Vec<u32> {
        let heard = &self.world().get_resource::<HeardCounts>().unwrap().0;
        heard
            .iter()
            .filter(|heard| heard.entity == entity)
            .map(|heard| heard.count)
            .collect()
    }
}

#[test]
fn messages_move_the_graph_and_nodes_answer_with_outcomes_and_signals() {
    let mut stage = Stage::new();
    stage.start();
    stage.player().set_extra(Marker(7));
    assert!(stage.in_state(name!("Idle")));

    stage.player().send(Poke { strength: 1.0 });
    stage.player().send(Other {});
    stage.frame();
    assert!(stage.in_state(name!("Poked")));
    assert!(!stage.in_state(name!("Elsewhere")));
    assert_eq!(stage.counters(), [saw(1)]);

    stage.player().send(Poke { strength: 1.0 });
    stage.frame();
    assert_eq!(stage.counters(), [saw(2)]);
    assert!(stage.in_state(name!("Idle")));
    let character = stage.character;
    assert_eq!(stage.heard_by(character), [1, 2]);

    stage.world().trigger(SendAnimationMessage {
        character,
        message: Poke { strength: 1.0 },
    });
    stage.frame();
    assert!(stage.in_state(name!("Poked")));
    assert_eq!(stage.counters(), [saw(1)]);
    assert_eq!(stage.heard_by(character), [1, 2, 1]);

    let bodiless = stage.bodiless;
    stage.world().trigger(SendAnimationMessage {
        character: bodiless,
        message: Poke { strength: 1.0 },
    });
    stage.frame();
    assert_eq!(stage.heard_by(bodiless), [1]);
}
