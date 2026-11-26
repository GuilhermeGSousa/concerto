use std::{any::TypeId, thread, time::Duration};

use concerto_animation::{
    TransitionBlender,
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    evaluation::{AnimationGraphContext, DriveContext},
    event::AnimationEvent,
    graph::AnimationGraph,
    message::{
        AnimationMessage, ClipEnded, MessageType, SendAnimationMessage, SendReflectedMessage,
    },
    node::{
        AnimationClipNode, AnimationNode, AnimationPlayMode, InputSlots, NodeOutcome, PlayInSlot,
        ReflectAnimationNode, SlotEntry, SlotNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{AnimationAppExt, AnimationPlugin},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
    retarget::{RetargetChain, RetargetNode, RetargetRig},
    root::AnimationRootBone,
};
use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity, IntoSystem, ResMut, System,
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
use concerto_reflect::{
    Reflect, ReflectDefault, ReflectedValue, apply_value, json_to_value, to_json, to_value,
};
use glam::{Mat4, Vec3};
use uuid::Uuid;

use crate::attributes::{Amount, drive_players};

const LONGEST_FRAME: Duration = Duration::from_millis(250);

#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
struct Poke {
    strength: f32,
}

impl AnimationMessage for Poke {}

#[derive(Clone, Default, Reflect)]
struct Other {}

impl AnimationMessage for Other {}

#[derive(Clone, Default, Reflect)]
struct Ring {}

impl AnimationMessage for Ring {}

#[derive(Clone, Default, Reflect)]
struct Rang {}

impl AnimationMessage for Rang {}

struct Marker(u32);

#[derive(Clone)]
struct Counted {
    count: u32,
}

impl Signal for Counted {}

impl EntitySignal for Counted {}

#[derive(Default, Reflect)]
#[reflect(AnimationNode, Default)]
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

    fn drive(
        &self,
        _: &mut Count,
        _: &(),
        _: &DriveContext<'_>,
        _: &[Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = inputs.first() {
            output.attributes_mut().copy_from(input.frame.attributes());
        }
        let bone_count = output.len();
        output
            .attributes_mut()
            .channel_mut::<Amount>(bone_count)
            .fill(Amount(1.0));
    }

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> Count {
        Count::default()
    }

    fn update(&self, state: &mut Count, _: f32, _: &(), context: &AnimationGraphContext<'_>) {
        for _ in context.message::<Poke>().filter(|poke| poke.strength > 0.0) {
            state.received += 1;
            state.marker = context.extra::<Marker>().map(|marker| marker.0);
            context.emit(Counted {
                count: state.received,
            });
        }
        if context.message::<Ring>().next().is_some() {
            context.raise(Rang {});
        }
    }

    fn evaluate(
        &self,
        _: &mut Count,
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if let Some(input) = evaluated_inputs.first() {
            output.copy_from(&input.frame);
        }
        if let Some(joint) = output.joint_mut(0) {
            joint.translation.y = 2.0;
        }
        let bone_count = output.len();
        output
            .attributes_mut()
            .channel_mut::<Amount>(bone_count)
            .fill(Amount(1.0));
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

fn event_named(event: &str) -> AnimationEvent {
    AnimationEvent {
        name: Name::new(event),
    }
}

fn evented_clip(event: &str, at: f32, length: f32) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, length],
            AnimationChanelOutput::from_translation([[0.0; 3], [0.0; 3]].into_iter()),
        ),
    );
    clip.add_event(at, event_named(event));
    clip
}

fn waiting_machine(
    app: &mut App,
    clip: AnimationClip,
    trigger: AnimationFSMTrigger,
) -> AnimationGraph {
    let clip = insert(app, clip);
    let playing = graph(app, AnimationGraph::from_node(AnimationClipNode::new(clip)));
    let moved = graph(app, AnimationGraph::new());
    let machine =
        AnimationStateMachine::from_initial_state(name!("Waiting"), playing, |transition| {
            transition.to(name!("Moved"), trigger, 0.0);
        })
        .state(name!("Moved"), moved, |_| {})
        .build();
    stored(AnimationGraph::from_node(machine))
}

fn spawn_waiting(app: &mut App, clip: AnimationClip, trigger: AnimationFSMTrigger) -> Entity {
    let machine = waiting_machine(app, clip, trigger);
    let machine = graph(app, machine);
    app.main_mut().world_mut().spawn((
        AnimationPlayer::new(1),
        AnimationHandleComponent::new(machine),
    ))
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
    slotted: Entity,
    slot_bone: Entity,
    jsoned: Entity,
    retargeted: Entity,
    nested: Entity,
    fading: Entity,
    restarted: Entity,
}

fn slot_node() -> SlotNode {
    SlotNode::new()
        .with_entry(
            SlotEntry::new(name!("count"), Counter {})
                .with_fade_in(0.2)
                .with_fade_out(0.2)
                .with_on(MessageType::of::<Poke>()),
        )
        .with_entry(
            SlotEntry::new(name!("snap"), Counter {})
                .with_fade_in(0.0)
                .with_fade_out(0.0)
                .with_on(MessageType::of::<Other>())
                .with_out(MessageType::of::<Rang>()),
        )
}

fn slot_graph(clip: AssetHandle<AnimationClip>, slot: SlotNode) -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(slot)
        .add_input(AnimationClipNode::new(clip));
    stored(graph)
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
        app.register_animation_message::<Ring>();
        app.register_animation_message::<Rang>();
        app.main_mut().world_mut().register_component::<Transform>();
        let idle = graph(&mut app, AnimationGraph::new());
        let poked = graph(&mut app, AnimationGraph::from_node(Counter {}));
        let machine_poked = poked.clone();
        let retargeted_poked = poked.clone();
        let elsewhere = graph(&mut app, AnimationGraph::new());
        let machine =
            AnimationStateMachine::from_initial_state(name!("Idle"), idle.clone(), |transition| {
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
        let deep =
            AnimationStateMachine::from_initial_state(name!("Deep"), poked.clone(), |_| {}).build();
        let deep = graph(&mut app, stored(AnimationGraph::from_node(deep)));
        let mut short = AnimationClip::default();
        short.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(
                vec![0.0, 0.2],
                AnimationChanelOutput::from_translation([[0.0; 3], [0.0; 3]].into_iter()),
            ),
        );
        let short = insert(&mut app, short);
        let ending = graph(
            &mut app,
            stored(AnimationGraph::from_node(
                AnimationClipNode::new(short).with_play_mode(AnimationPlayMode::PlayOnce),
            )),
        );
        let nested =
            AnimationStateMachine::from_initial_state(name!("Inside"), deep, |transition| {
                transition.to(name!("Out"), AnimationFSMTrigger::on_message::<Rang>(), 0.0);
            })
            .state(name!("Out"), ending.clone(), |transition| {
                transition.to(
                    name!("Done"),
                    AnimationFSMTrigger::on_message::<ClipEnded>(),
                    0.0,
                );
            })
            .state(name!("Done"), idle.clone(), |_| {})
            .build();
        let nested = graph(&mut app, stored(AnimationGraph::from_node(nested)));
        let fading = AnimationStateMachine::from_initial_state(
            name!("Ringing"),
            poked.clone(),
            |transition| {
                transition
                    .to(
                        name!("Away"),
                        AnimationFSMTrigger::on_message::<Poke>(),
                        1.0,
                    )
                    .to(
                        name!("Stale"),
                        AnimationFSMTrigger::on_message::<Rang>(),
                        0.0,
                    );
            },
        )
        .state(name!("Away"), idle.clone(), |transition| {
            transition
                .to(
                    name!("Fired"),
                    AnimationFSMTrigger::on_message::<Rang>(),
                    0.0,
                )
                .to(
                    name!("Ringing"),
                    AnimationFSMTrigger::on_message::<Other>(),
                    1.0,
                );
        })
        .state(name!("Fired"), idle.clone(), |_| {})
        .state(name!("Stale"), idle.clone(), |_| {})
        .build();
        let fading = graph(&mut app, stored(AnimationGraph::from_node(fading)));
        let attack =
            AnimationStateMachine::from_initial_state(name!("Attack"), ending, |transition| {
                transition.to(
                    name!("Recovered"),
                    AnimationFSMTrigger::on_message::<ClipEnded>(),
                    0.0,
                );
            })
            .state(name!("Recovered"), idle.clone(), |_| {})
            .build();
        let attack = graph(&mut app, stored(AnimationGraph::from_node(attack)));
        let restarted =
            AnimationStateMachine::from_initial_state(name!("Holding"), attack, |transition| {
                transition.to(
                    name!("Elsewhere"),
                    AnimationFSMTrigger::on_message::<Poke>(),
                    0.1,
                );
            })
            .state(name!("Elsewhere"), idle.clone(), |transition| {
                transition.to(
                    name!("Holding"),
                    AnimationFSMTrigger::on_message::<Other>(),
                    0.1,
                );
            })
            .blender(TransitionBlender::Inertialization)
            .build();
        let restarted = graph(&mut app, stored(AnimationGraph::from_node(restarted)));
        let clip = insert(&mut app, AnimationClip::default());
        let json = {
            let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
            let json = to_json(&slot_node(), &registry);
            assert_eq!(
                json["entries"][0]["node"],
                serde_json::json!({ "type": std::any::type_name::<Counter>(), "value": {} })
            );
            let stored = json_to_value(&json, TypeId::of::<SlotNode>(), &registry).unwrap();
            let mut rebuilt = SlotNode::new();
            assert!(apply_value(&mut rebuilt, &stored).is_empty());
            assert_eq!(
                to_json(&rebuilt, &registry),
                json,
                "a slot's JSON form, its entries' out types included, reads back as written"
            );
            rebuilt
        };
        let jsoned = graph(&mut app, slot_graph(clip.clone(), json));
        let slotted = graph(&mut app, slot_graph(clip, slot_node()));
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
        let standing = insert(
            &mut app,
            Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["root".into()],
                vec![JointPose {
                    translation: Vec3::Y,
                    ..JointPose::IDENTITY
                }],
                vec![Uuid::from_u128(2)],
            )
            .unwrap(),
        );
        let rig = insert(
            &mut app,
            RetargetRig::new()
                .with_skeleton(standing.clone())
                .with_root("root")
                .with_chain(RetargetChain::new("body", "root", "root")),
        );
        let retarget = graph(
            &mut app,
            AnimationGraph::from_node(RetargetNode::new(retargeted_poked, rig.clone(), rig)),
        );
        let world = app.main_mut().world_mut();
        let bone = world.spawn(Transform::IDENTITY);
        let character = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(machine),
            SkeletonComponent::new(skeleton.clone(), vec![SceneEntityRef::Entity(bone)], None),
            hear_counted.into_listener(),
        ));
        let bodiless = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(poked),
            hear_counted.into_listener(),
        ));
        let slot_bone = world.spawn((Transform::IDENTITY, AnimationRootBone::default()));
        let slotted = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(slotted),
            SkeletonComponent::new(
                skeleton.clone(),
                vec![SceneEntityRef::Entity(slot_bone)],
                None,
            ),
            hear_counted.into_listener(),
        ));
        let json_bone = world.spawn((Transform::IDENTITY, AnimationRootBone::default()));
        let jsoned = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(jsoned),
            SkeletonComponent::new(
                skeleton.clone(),
                vec![SceneEntityRef::Entity(json_bone)],
                None,
            ),
            hear_counted.into_listener(),
        ));
        let standing_bone = world.spawn(Transform::IDENTITY);
        let retargeted = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(retarget),
            SkeletonComponent::new(standing, vec![SceneEntityRef::Entity(standing_bone)], None),
            hear_counted.into_listener(),
        ));
        let nested = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(nested),
        ));
        let restarted = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(restarted),
        ));
        let fading_bone = world.spawn(Transform::IDENTITY);
        let fading = world.spawn((
            AnimationPlayer::new(1),
            AnimationHandleComponent::new(fading),
            SkeletonComponent::new(skeleton, vec![SceneEntityRef::Entity(fading_bone)], None),
        ));
        Self {
            app,
            character,
            bodiless,
            slotted,
            slot_bone,
            jsoned,
            retargeted,
            nested,
            fading,
            restarted,
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

    fn advance(&mut self, seconds: f32) {
        let world = self.world();
        let time = world.get_resource_mut::<Time>().unwrap();
        time.set_time_scale(seconds / LONGEST_FRAME.as_secs_f32());
        if seconds > 0.0 {
            thread::sleep(LONGEST_FRAME + Duration::from_millis(10));
        }
        time.update();
        world.run_schedule(LateUpdate);
    }

    fn player_of(&mut self, entity: Entity) -> &mut AnimationPlayer {
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(entity)
            .unwrap()
    }

    fn slot_player(&mut self) -> &mut AnimationPlayer {
        let slotted = self.slotted;
        self.player_of(slotted)
    }

    fn slot_amount(&mut self) -> f32 {
        self.slot_player().attributes().get::<Amount>(0).0
    }

    fn amount_of(&mut self, entity: Entity) -> f32 {
        self.world()
            .get_component_for_entity::<AnimationPlayer>(entity)
            .unwrap()
            .attributes()
            .get::<Amount>(0)
            .0
    }

    fn driven_amount(&mut self) -> f32 {
        let slotted = self.slotted;
        let world = self.world();
        let mut system = drive_players.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        world
            .get_component_for_entity::<AnimationPlayer>(slotted)
            .unwrap()
            .drive_attributes()
            .get::<Amount>(0)
            .0
    }

    fn slot_behaviour(&mut self) -> String {
        let slotted = self.slotted;
        let world = self.world();
        let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
        let mut reported = String::new();
        world
            .get_component_for_entity::<AnimationPlayer>(slotted)
            .unwrap()
            .for_each_node(graphs, |visit| {
                if let Some(behaviour) = &visit.report.behaviour {
                    reported = behaviour.describe();
                }
            });
        reported
    }

    fn slot_lift(&mut self) -> f32 {
        let bone = self.slot_bone;
        self.world()
            .get_component_for_entity::<Transform>(bone)
            .unwrap()
            .translation
            .y
    }

    fn player(&mut self) -> &mut AnimationPlayer {
        let character = self.character;
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(character)
            .unwrap()
    }

    fn in_state(&mut self, state: Name) -> bool {
        let character = self.character;
        self.in_state_of(character, state)
    }

    fn in_state_of(&mut self, character: Entity, state: Name) -> bool {
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
    stage.frame();
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

    stage.player_of(bodiless).set_paused(true);
    stage.player_of(bodiless).send(Poke { strength: 1.0 });
    stage.frame();
    assert_eq!(stage.heard_by(bodiless), [1]);
    stage.player_of(bodiless).set_paused(false);
    stage.frame();
    stage.frame();
    assert_eq!(
        stage.heard_by(bodiless),
        [1, 2],
        "a message sent while paused is read once the player resumes, and only once"
    );

    let retargeted = stage.retargeted;
    stage.player_of(retargeted).send(Poke { strength: 1.0 });
    stage.frame();
    assert_eq!(
        stage.heard_by(retargeted),
        [1],
        "a message reaches a node inside a retarget"
    );

    let nested = stage.nested;
    stage.player_of(nested).send(Ring {});
    stage.frame();
    assert!(
        stage.in_state_of(nested, name!("Inside")),
        "a raised message moves an outer machine on the next frame, not the one it was raised in"
    );
    stage.frame();
    assert!(
        stage.in_state_of(nested, name!("Out")),
        "a message raised two levels down moves the outer machine"
    );

    let fading = stage.fading;
    stage.player_of(fading).send(Poke { strength: 1.0 });
    stage.frame();
    assert!(stage.in_state_of(fading, name!("Away")));
    stage.player_of(fading).send(Ring {});
    stage.frame();
    stage.frame();
    assert!(
        stage.in_state_of(fading, name!("Away")) && !stage.in_state_of(fading, name!("Fired")),
        "a state fading out raises, and its machine does not move on it"
    );
    stage.player_of(fading).send(Other {});
    stage.frame();
    stage.frame();
    assert!(
        stage.in_state_of(fading, name!("Ringing")) && !stage.in_state_of(fading, name!("Stale")),
        "a state re-entered within its fade does not see what it raised before"
    );

    stage.advance(0.1);
    assert!(stage.in_state_of(nested, name!("Out")));
    stage.advance(0.15);
    assert!(stage.in_state_of(nested, name!("Out")));
    stage.advance(0.0);
    assert!(
        stage.in_state_of(nested, name!("Done")),
        "OnMessage(ClipEnded) leaves a state once its clip, played once, stops"
    );

    let restarted = stage.restarted;
    assert!(stage.in_state_of(restarted, name!("Recovered")));
    stage.player_of(restarted).send(Poke { strength: 1.0 });
    stage.advance(0.0);
    stage.player_of(restarted).send(Other {});
    stage.advance(0.0);
    assert!(
        stage.in_state_of(restarted, name!("Attack")),
        "re-entering a state starts the machine inside it from its entry state"
    );
    stage.advance(0.1);
    stage.advance(0.15);
    stage.player_of(restarted).send(Poke { strength: 1.0 });
    stage.advance(0.0);
    stage.player_of(restarted).send(Other {});
    stage.advance(0.0);
    stage.advance(0.1);
    stage.advance(0.15);
    stage.advance(0.0);
    assert!(
        stage.in_state_of(restarted, name!("Recovered")),
        "a machine left on the frame after its clip ended plays that clip again when re-entered and moves on"
    );

    let slotted = stage.slotted;
    stage.advance(0.0);
    assert_eq!(stage.slot_amount(), 0.0);

    let jsoned = stage.jsoned;
    stage.slot_player().send(Poke { strength: 1.0 });
    stage
        .world()
        .get_component_for_entity_mut::<AnimationPlayer>(jsoned)
        .unwrap()
        .send(Poke { strength: 1.0 });
    stage.advance(0.1);
    let fading_in = stage.slot_amount();
    assert_eq!(
        stage.amount_of(jsoned),
        fading_in,
        "an entry made from its JSON form plays like one built in code"
    );
    assert_eq!(stage.heard_by(jsoned), [1]);
    assert_eq!(
        stage.driven_amount(),
        fading_in,
        "the drive pass fades an entry's attributes like the pose"
    );
    assert!(
        (fading_in - 0.5).abs() < 0.05,
        "half way through a 0.2 s fade in the amount is {fading_in}"
    );
    assert_eq!(stage.heard_by(slotted), [1]);
    assert!(
        stage.slot_behaviour().starts_with("playing count 0.5"),
        "the slot reports its playing entry with its fade: {}",
        stage.slot_behaviour()
    );

    stage.slot_player().send(Poke { strength: 1.0 });
    stage.advance(0.1);
    assert_eq!(
        stage.heard_by(slotted),
        [1, 1],
        "a repeat poke restarts the entry"
    );
    assert!((stage.slot_amount() - 1.0).abs() < 0.05);
    assert!(
        (stage.slot_lift() - 2.0).abs() < 0.05,
        "an entry's root joint shows through the slot"
    );

    stage.slot_player().send(Poke { strength: 1.0 });
    stage.slot_player().send(Poke { strength: 1.0 });
    stage.advance(0.1);
    assert_eq!(
        stage.heard_by(slotted),
        [1, 1, 1, 2],
        "two pokes in one frame restart the entry once and both reach it"
    );
    let fading_out = stage.slot_amount();
    assert!(
        (fading_out - 0.5).abs() < 0.05,
        "half way through a 0.2 s fade out the amount is {fading_out}"
    );

    stage.advance(0.1);
    assert_eq!(stage.slot_amount(), 0.0);
    assert_eq!(
        stage.slot_behaviour(),
        "no entry playing (running)",
        "a faded out entry leaves the report"
    );
    stage.advance(0.1);
    assert_eq!(stage.slot_amount(), 0.0);
    assert_eq!(stage.heard_by(slotted), [1, 1, 1, 2]);

    stage.slot_player().send(PlayInSlot {
        entry: name!("count"),
    });
    stage.advance(0.1);
    let played = stage.slot_amount();
    assert!(
        (played - 0.5).abs() < 0.05,
        "a play message starts the entry, and the amount is {played}"
    );

    stage.slot_player().send(Other {});
    stage.advance(0.0);
    assert_eq!(
        stage.slot_amount(),
        1.0,
        "an entry with no fade in is at full weight at once"
    );

    stage.slot_player().send(Ring {});
    stage.advance(0.0);
    assert!(
        (stage.slot_amount() - played).abs() < 0.05,
        "an entry fades out when its node raises one of its out types"
    );
    stage.slot_player().send(Other {});
    stage.advance(0.0);
    assert_eq!(
        stage.slot_amount(),
        1.0,
        "an entry that faded out on its out type starts again on its next message"
    );

    stage.advance(0.3);
    let slotted = stage.slotted;
    let heard = stage.heard_by(slotted).len();
    let value = to_value(&Poke { strength: 1.0 }).unwrap();
    stage.world().trigger(SendReflectedMessage {
        character: slotted,
        type_path: "nope".to_owned(),
        value: value.clone(),
    });
    stage.advance(0.1);
    assert_eq!(
        stage.heard_by(slotted).len(),
        heard,
        "a type that is not registered is dropped"
    );
    stage.world().trigger(SendReflectedMessage {
        character: slotted,
        type_path: std::any::type_name::<Poke>().to_owned(),
        value,
    });
    stage.advance(0.1);
    assert!(
        stage.heard_by(slotted).len() > heard,
        "a reflected message of a registered type reaches the player"
    );

    let wanted = AnimationFSMTrigger::on_message_value(&event_named("jump_apex"));
    let apex = spawn_waiting(
        &mut stage.app,
        evented_clip("jump_apex", 0.1, 0.2),
        wanted.clone(),
    );
    let other = spawn_waiting(
        &mut stage.app,
        evented_clip("other", 0.1, 0.2),
        wanted.clone(),
    );
    let first = spawn_waiting(&mut stage.app, evented_clip("jump_apex", 0.0, 10.0), wanted);
    stage.frame();
    stage.frame();
    stage.frame();
    assert!(
        stage.in_state_of(first, name!("Moved")),
        "an event keyed at the start of a clip that just started is raised on its first update"
    );
    stage.advance(0.15);
    assert!(
        stage.in_state_of(apex, name!("Waiting")),
        "a clip event crossed in a frame moves its machine on the next"
    );
    stage.frame();
    assert!(stage.in_state_of(apex, name!("Moved")));
    assert!(
        stage.in_state_of(other, name!("Waiting")),
        "a clip event of another name does not match the wanted value"
    );

    let mistyped = AnimationFSMTrigger::OnMessage {
        message: MessageType::of::<AnimationEvent>(),
        value: Some(ReflectedValue::Struct(vec![(
            "nope".to_owned(),
            ReflectedValue::Struct(Vec::new()),
        )])),
    };
    let mut checked = waiting_machine(&mut stage.app, AnimationClip::default(), mistyped);
    let registry = stage.app.get_resource::<AppTypeRegistry>().unwrap().read();
    checked.make_live(&registry, None).unwrap();
    assert!(
        checked
            .load_warnings()
            .iter()
            .any(|warning| warning.reason.contains("no such field")),
        "a wanted value naming a field its type lacks warns at load: {:?}",
        checked.load_warnings()
    );
}
