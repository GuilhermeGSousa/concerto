use std::{thread, time::Duration};

use concerto_animation::{
    attribute::{PoseAttribute, PoseAttributes},
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    evaluation::AnimationGraphContext,
    event::{AnimationEvent, AnimationSignal},
    graph::{AnimationGraph, AnimationNodeIndex},
    mask::{BoneMask, BoneMaskBranch},
    node::{
        AdditiveNode, AnimationClipNode, AnimationNode, BlendNode, InputSlots, LayerNode,
        ReflectAnimationNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{AnimationAppExt, AnimationPlugin},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
    transition::TransitionBlender,
};
use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity,
    component::scene::SceneEntityRef,
    reflect::AppTypeRegistry,
    resource::{ResMut, Resource},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
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
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

#[derive(Resource, Default)]
struct Heard(usize);

const BONES: usize = 3;
const ALL_BONES: [usize; BONES] = [0, 1, 2];
const LONGEST_FRAME: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Amount(f32);

impl PoseAttribute for Amount {
    fn neutral() -> Self {
        Amount(0.0)
    }

    fn blend(&mut self, self_weight: f32, other: &Self, other_weight: f32) {
        let total = self_weight + other_weight;
        if total > 0.0 {
            self.0 = (self.0 * self_weight + other.0 * other_weight) / total;
        }
    }
}

#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct SetAmount {
    value: f32,
    rate: f32,
    bones: Vec<usize>,
}

impl SetAmount {
    fn on(value: f32, bones: &[usize]) -> Self {
        Self {
            value,
            rate: 0.0,
            bones: bones.to_vec(),
        }
    }

    fn rising(mut self, rate: f32) -> Self {
        self.rate = rate;
        self
    }
}

impl AnimationNode for SetAmount {
    type State = f32;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn create_state(&self, _: &(), _: &AnimationGraphContext<'_>) -> f32 {
        0.0
    }

    fn update(&self, elapsed: &mut f32, delta_time: f32, _: &(), _: &AnimationGraphContext<'_>) {
        *elapsed += delta_time;
    }

    fn evaluate(
        &self,
        elapsed: &mut f32,
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
        let bone_count = output.len();
        let amounts = output.attributes_mut().channel_mut::<Amount>(bone_count);
        for bone in &self.bones {
            amounts[*bone] = Amount(self.value + self.rate * *elapsed);
        }
    }

    fn reset(&self, elapsed: &mut f32) {
        *elapsed = 0.0;
    }

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }
}

fn standing(height: f32) -> JointPose {
    JointPose {
        translation: Vec3::new(0.0, height, 0.0),
        ..JointPose::IDENTITY
    }
}

fn skeleton() -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; BONES],
        vec![None, Some(0), Some(1)],
        vec!["root".into(), "spine".into(), "arm".into()],
        vec![standing(1.0), standing(0.5), standing(0.5)],
        (0..BONES as u128).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

fn moving_clip() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, 10.0],
            AnimationChanelOutput::from_translation(
                [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]].into_iter(),
            ),
        ),
    );
    clip
}

struct Stage {
    app: App,
    skeleton: AssetHandle<Skeleton>,
    clip: AssetHandle<AnimationClip>,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.register_plugin(AnimationPlugin);
        app.register_animation_node::<SetAmount>();
        app.main_mut().world_mut().register_component::<Transform>();
        let mut stage = Self {
            app,
            skeleton: AssetHandle::default(),
            clip: AssetHandle::default(),
        };
        stage.skeleton = stage.insert(skeleton());
        stage.clip = stage.insert(moving_clip());
        stage
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn insert<T: Asset>(&mut self, asset: T) -> AssetHandle<T> {
        let id = AssetId::new();
        self.world()
            .get_resource_mut::<AssetStore<T>>()
            .unwrap()
            .insert(id, asset);
        AssetHandle::weak(id)
    }

    fn graph(&mut self, mut graph: AnimationGraph) -> AssetHandle<AnimationGraph> {
        {
            let registry = self.app.get_resource::<AppTypeRegistry>().unwrap().read();
            graph.make_live(&registry, None).unwrap();
        }
        self.insert(graph)
    }

    fn clip_node(&self) -> AnimationClipNode {
        AnimationClipNode::new(self.clip.clone())
    }

    fn player(&mut self, graph: AnimationGraph) -> Entity {
        let graph = self.graph(graph);
        let skeleton = self.skeleton.clone();
        let world = self.world();
        let bones = (0..BONES)
            .map(|_| SceneEntityRef::Entity(world.spawn(Transform::IDENTITY)))
            .collect();
        world.spawn((
            AnimationPlayer::new(BONES),
            AnimationHandleComponent::new(graph),
            SkeletonComponent::new(skeleton, bones, None),
        ))
    }

    fn machine(
        &mut self,
        from: AnimationGraph,
        to: AnimationGraph,
        blender: TransitionBlender,
        blend_time: f32,
    ) -> Entity {
        let from = self.graph(from);
        let to = self.graph(to);
        let machine =
            AnimationStateMachine::from_initial_state(name!("from"), from, |transition| {
                transition.to(
                    name!("to"),
                    AnimationFSMTrigger::on_bool(name!("go"), true),
                    blend_time,
                );
            })
            .state(name!("to"), to, |_| {})
            .blender(blender)
            .build();
        self.player(AnimationGraph::from_node(machine))
    }

    fn start(&mut self) {
        let world = self.world();
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);
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

    fn set_param(&mut self, player: Entity, key: Name, value: bool) {
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(player)
            .unwrap()
            .set_bool_param(key, value);
    }

    fn set_weight(&mut self, player: Entity, node: AnimationNodeIndex, weight: f32) {
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(player)
            .unwrap()
            .set_node_weight(&node, weight);
    }

    fn there_and_back(
        &mut self,
        there: AnimationGraph,
        back: AnimationGraph,
        blend_time: f32,
    ) -> Entity {
        let there = self.graph(there);
        let back = self.graph(back);
        let machine = AnimationStateMachine::from_initial_state(name!("a"), there, |transition| {
            transition.to(
                name!("b"),
                AnimationFSMTrigger::on_bool(name!("go"), true),
                blend_time,
            );
        })
        .state(name!("b"), back, |transition| {
            transition.to(
                name!("a"),
                AnimationFSMTrigger::on_bool(name!("go"), false),
                blend_time,
            );
        })
        .build();
        self.player(AnimationGraph::from_node(machine))
    }

    fn turned_clip(&mut self, angle: f32, event_at: Option<f32>) -> AssetHandle<AnimationClip> {
        let turn = Quat::from_rotation_z(angle).to_array();
        let mut clip = AnimationClip::default();
        clip.add_channel(
            Uuid::from_u128(2),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_rotation([turn, turn].into_iter()),
            ),
        );
        if let Some(time) = event_at {
            clip.add_event(
                time,
                AnimationEvent {
                    name: name!("step"),
                },
            );
        }
        self.insert(clip)
    }

    fn hear_events(&mut self, player: Entity) {
        let world = self.world();
        world.insert_resource(Heard::default());
        world.register_component::<Listener<AnimationSignal<AnimationEvent>>>();
        let hear = (|_: On<AnimationSignal<AnimationEvent>>, mut heard: ResMut<Heard>| {
            heard.0 += 1;
        })
        .into_listener();
        world.insert(hear, player);
    }

    fn heard(&mut self) -> usize {
        self.world().get_resource::<Heard>().unwrap().0
    }

    fn rotation(&mut self, player: Entity, bone: usize) -> Quat {
        let world = self.world();
        let entity = world
            .get_component_for_entity::<SkeletonComponent>(player)
            .unwrap()
            .bones()[bone]
            .entity()
            .unwrap();
        world
            .get_component_for_entity::<Transform>(entity)
            .unwrap()
            .rotation
    }

    fn swap_to_unloaded(&mut self, player: Entity) {
        let unloaded = AnimationHandleComponent::new(AssetHandle::weak(AssetId::new()));
        self.world().insert(unloaded, player);
    }

    fn pause(&mut self, player: Entity) {
        self.world()
            .get_component_for_entity_mut::<AnimationPlayer>(player)
            .unwrap()
            .set_paused(true);
    }

    fn amounts(&mut self, player: Entity) -> Vec<f32> {
        let attributes = self
            .world()
            .get_component_for_entity::<AnimationPlayer>(player)
            .unwrap()
            .attributes();
        per_bone(attributes)
    }
}

fn per_bone(attributes: &PoseAttributes) -> Vec<f32> {
    ALL_BONES
        .iter()
        .map(|bone| attributes.get::<Amount>(*bone).0)
        .collect()
}

fn single(node: impl AnimationNode) -> AnimationGraph {
    AnimationGraph::from_node(node)
}

#[test]
fn attributes_travel_with_the_pose_through_every_blend() {
    let mut stage = Stage::new();

    let mut blended = AnimationGraph::new();
    let blend = blended.result_node().input(BlendNode {}).index();
    let set = blended.add_node(SetAmount::on(1.0, &[1]), blend).index();
    let clip = blended.add_node(stage.clip_node(), blend).index();
    let blend_player = stage.player(blended);

    let mask = stage.insert(BoneMask::new().with_branch(BoneMaskBranch::new("arm")));
    let mut layered = AnimationGraph::new();
    let clip_node = stage.clip_node();
    layered.result_node().add_layer_input(
        LayerNode::new(mask).with_weight(0.5),
        clip_node,
        SetAmount::on(1.0, &ALL_BONES),
    );
    let layer_player = stage.player(layered);

    let clip_graph = single(stage.clip_node());
    let cross_fade = stage.machine(
        clip_graph,
        single(SetAmount::on(1.0, &ALL_BONES)),
        TransitionBlender::CrossFade,
        1.0,
    );
    let clip_graph = single(stage.clip_node());
    let inertialized = stage.machine(
        single(SetAmount::on(1.0, &ALL_BONES)),
        clip_graph,
        TransitionBlender::Inertialization,
        0.1,
    );

    let mut additive = AnimationGraph::new();
    additive
        .result_node()
        .input(AdditiveNode::new(AssetHandle::weak(AssetId::new())))
        .add_input(SetAmount::on(1.0, &ALL_BONES));
    let additive_player = stage.player(additive);

    let rising = stage.machine(
        single(SetAmount::on(0.0, &ALL_BONES)),
        single(SetAmount::on(0.0, &ALL_BONES).rising(1.0)),
        TransitionBlender::Inertialization,
        0.2,
    );

    stage.start();
    stage.set_weight(blend_player, set, 0.25);
    stage.set_weight(blend_player, clip, 0.75);
    stage.advance(0.0);

    assert_eq!(stage.amounts(blend_player), [0.0, 0.25, 0.0]);
    assert_eq!(stage.amounts(layer_player), [0.0, 0.0, 0.5]);
    assert_eq!(stage.amounts(additive_player), [1.0; BONES]);

    stage.set_param(cross_fade, name!("go"), true);
    stage.set_param(inertialized, name!("go"), true);
    stage.set_param(rising, name!("go"), true);
    stage.advance(0.0);
    assert_eq!(stage.amounts(inertialized), [1.0; BONES]);

    stage.advance(0.01);
    let right_after = stage.amounts(inertialized)[0];
    assert!(
        right_after > 0.9 && right_after < 1.0,
        "right after the transition the source's amount is {right_after}"
    );

    stage.advance(0.49);
    let halfway = stage.amounts(cross_fade)[0];
    assert!(
        (halfway - 0.5).abs() < 0.02,
        "halfway through the cross-fade the amount is {halfway}"
    );

    stage.advance(0.5);
    let settled = stage.amounts(inertialized)[0];
    assert!(
        settled < 0.01,
        "a second after the transition the amount is {settled}"
    );
    let following = stage.amounts(rising)[0];
    assert!(
        (following - 0.96875).abs() < 0.01,
        "a second into a transition to an amount rising by 1 a second, with the source's share down to 1/32, the amount is {following}"
    );

    stage.swap_to_unloaded(blend_player);
    stage.advance(0.0);
    stage.advance(0.0);
    assert_eq!(
        stage.amounts(blend_player),
        [0.0; BONES],
        "a player swapped to a graph that is not loaded keeps no attributes"
    );
    stage.pause(layer_player);
    stage.advance(0.0);
    assert_eq!(
        stage.amounts(layer_player),
        [0.0, 0.0, 0.5],
        "a paused player keeps the attributes it last evaluated"
    );

    let blend_time = 2.0;
    let there = stage.turned_clip(0.8, Some(0.45));
    let back = stage.turned_clip(-0.8, None);
    let interrupted = stage.there_and_back(
        single(AnimationClipNode::new(there)),
        single(AnimationClipNode::new(back)),
        blend_time,
    );
    stage.hear_events(interrupted);
    stage.advance(0.0);
    stage.set_param(interrupted, name!("go"), true);
    stage.advance(0.0);
    for _ in 0..5 {
        stage.advance(0.1);
    }
    stage.set_param(interrupted, name!("go"), false);
    let mut before = stage.rotation(interrupted, 2);
    for _ in 0..3 {
        stage.advance(0.1);
        let now = stage.rotation(interrupted, 2);
        assert!(
            now.angle_between(before) <= 0.1 / blend_time * 1.6 + 0.01,
            "a cross-fade sent back to the state it left fades from where it was, the joint turned {} rad in a frame",
            now.angle_between(before)
        );
        before = now;
    }
    assert_eq!(
        stage.heard(),
        1,
        "the clip event of the state left and re-entered fires once"
    );
}
