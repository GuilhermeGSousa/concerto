use std::path::Path;

use concerto_animation::TransitionBlender;
use concerto_animation::clip::{
    AnimationChanelOutput, AnimationChannel, AnimationClip, ClipEventData,
};
use concerto_animation::evaluation::AnimationGraphContext;
use concerto_animation::event::{AnimationEvent, ClipEvent, ReflectClipEvent};
use concerto_animation::graph::AnimationGraph;
use concerto_animation::node::blend_space::BlendSpace2DNode;
use concerto_animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto_animation::node::{
    AnimationClipNode, AnimationNode, ErasedAnimationNode, ReflectAnimationNode,
};
use concerto_animation::plugin::register_animation_types;
use concerto_animation::pose::{EvaluatedPose, Pose, PosePool};
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_ecs::world::World;
use concerto_foundation::assets::AssetId;
use concerto_foundation::assets::asset_server::{AssetServer, handle_asset_load_events};
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_foundation::assets::content::{
    ContentAssetHeader, check_encoding, decode_content_asset, write_content_asset,
};
use concerto_foundation::assets::handle::{AssetHandle, LiveLeaves};
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistry, ValueWarning, to_value};
use uuid::Uuid;

#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
struct Gain {
    amount: f32,
}

impl AnimationNode for Gain {
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
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        _: &[EvaluatedPose],
        _: &mut PosePool,
        _: &mut Pose,
    ) {
    }

    fn reset(&self, _: &mut ()) {}
}

#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(ClipEvent)]
struct Footstep {
    strength: f32,
}

impl ClipEvent for Footstep {}

#[derive(Clone, Default, Reflect)]
#[reflect(ClipEvent)]
struct Impact {
    pose: AssetHandle<AnimationClip>,
}

impl ClipEvent for Impact {}

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

fn clip(name: &str) -> AssetHandle<AnimationClip> {
    AssetHandle::weak(AssetId::from_path(name))
}

fn registry(with_gain: bool) -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    register_animation_types(&mut registry);
    if with_gain {
        registry.register::<Gain>();
    }
    registry
}

fn live_type_names(graph: &AnimationGraph) -> Vec<&'static str> {
    graph
        .live_nodes()
        .map(|node| node.as_reflect().type_name())
        .collect()
}

fn live_node<T: AnimationNode>(graph: &AnimationGraph) -> Option<&T> {
    graph
        .live_nodes()
        .find_map(|node: &dyn ErasedAnimationNode| node.as_reflect().downcast_ref::<T>())
}

fn state_machine() -> AnimationStateMachine {
    let state = |name: &str| AssetHandle::weak(AssetId::from_path(name));
    AnimationStateMachine::from_initial_state(name!("idle"), state("idle_graph"), |transition| {
        transition.to(
            name!("walk"),
            AnimationFSMTrigger::on_bool(name!("go"), true),
            0.2,
        );
    })
    .state(name!("walk"), state("walk_graph"), |transition| {
        transition.to(
            name!("run"),
            AnimationFSMTrigger::on_non_zero_vec(name!("move")),
            0.15,
        );
    })
    .state(name!("run"), state("run_graph"), |transition| {
        transition.to(name!("idle"), AnimationFSMTrigger::OnAnimationEnd, 0.1);
    })
    .blender(TransitionBlender::Inertialization)
    .build()
}

fn mixed_graph() -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_input(Gain { amount: 2.5 }, |_| {})
        .with_input(
            AnimationClipNode::new(clip("outer_pose"))
                .with_play_rate(1.5)
                .with_play_rate_param(name!("draw_speed")),
            |_| {},
        )
        .with_input(state_machine(), |_| {})
        .with_blend_space_2d_input(name!("movement"), |ctx| {
            ctx.animation_clip_input(clip("idle"), glam::Vec2::ZERO)
                .animation_clip_input(clip("walk"), glam::Vec2::new(0.0, 1.0));
        });
    graph
}

fn round_trip(graph: &AnimationGraph) -> AnimationGraph {
    let bytes = bincode::serialize(graph).expect("serialize");
    bincode::deserialize(&bytes).expect("deserialize")
}

#[test]
fn a_registered_custom_node_survives_a_round_trip_next_to_built_ins() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    assert!(!restored.is_live());
    restored.make_live(&registry, None).unwrap();

    assert!(restored.is_live());
    assert!(restored.load_warnings().is_empty());
    assert_eq!(restored.iter().count(), 7);
    assert_eq!(live_node::<Gain>(&restored).unwrap().amount, 2.5);
    assert_eq!(
        live_node::<AnimationClipNode>(&restored)
            .unwrap()
            .clip()
            .id(),
        AssetId::from_path("outer_pose"),
        "clip AssetId survived the round trip"
    );
}

#[test]
fn a_blend_space_keeps_its_input_param_and_points() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    restored.make_live(&registry, None).unwrap();

    let blend_space = live_node::<BlendSpace2DNode>(&restored).expect("blend space survived");
    assert_eq!(blend_space.input().param, name!("movement"));
    assert_eq!(
        blend_space.points(),
        &[glam::Vec2::ZERO, glam::Vec2::new(0.0, 1.0)]
    );
}

#[test]
fn a_state_machine_keeps_trigger_payloads_blender_and_blend_times() {
    let registry = registry(true);
    let mut restored = round_trip(&mixed_graph());
    restored.make_live(&registry, None).unwrap();

    let machine = live_node::<AnimationStateMachine>(&restored).expect("state machine survived");
    let triggers: Vec<&AnimationFSMTrigger> = machine.triggers().collect();

    let bool_equals = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::BoolEquals { param, value } => Some((*param, *value)),
            _ => None,
        })
        .expect("BoolEquals trigger survived");
    assert_eq!(bool_equals, (name!("go"), true));

    let vec2_non_zero = triggers
        .iter()
        .find_map(|trigger| match trigger {
            AnimationFSMTrigger::Vec2NonZero { param } => Some(*param),
            _ => None,
        })
        .expect("Vec2NonZero trigger survived");
    assert_eq!(vec2_non_zero, name!("move"));
    assert!(
        triggers
            .iter()
            .any(|trigger| matches!(trigger, AnimationFSMTrigger::OnAnimationEnd))
    );

    assert_eq!(machine.blender(), TransitionBlender::Inertialization);
    assert_eq!(
        machine.blend_times().collect::<Vec<_>>(),
        vec![0.2, 0.15, 0.1]
    );
}

#[test]
fn a_graph_built_in_code_and_one_loaded_from_its_bytes_match() {
    let registry = registry(true);
    let mut built = mixed_graph();
    built.make_live(&registry, None).unwrap();
    let mut loaded = round_trip(&built);
    loaded.make_live(&registry, None).unwrap();

    assert_eq!(
        built.nodes().collect::<Vec<_>>(),
        loaded.nodes().collect::<Vec<_>>()
    );
    assert_eq!(live_type_names(&built), live_type_names(&loaded));
    assert_eq!(built.iter().collect::<Vec<_>>().len(), 7);
    assert!(
        built
            .nodes()
            .any(|node| node.type_name() == std::any::type_name::<Gain>())
    );
}

#[test]
fn an_unregistered_node_type_loads_with_a_warning_and_is_kept_on_save() {
    let bytes = bincode::serialize(&mixed_graph()).unwrap();
    let mut loaded: AnimationGraph = bincode::deserialize(&bytes).unwrap();
    loaded.make_live(&registry(false), None).unwrap();

    assert!(loaded.is_live());
    let warnings = loaded.load_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].path.contains("Gain"), "{:?}", warnings[0]);
    assert!(live_node::<Gain>(&loaded).is_none());
    assert!(live_node::<AnimationClipNode>(&loaded).is_some());
    assert!(live_node::<AnimationStateMachine>(&loaded).is_some());
    assert!(
        live_type_names(&loaded)
            .iter()
            .any(|name| name.ends_with("PlaceholderNode"))
    );

    assert_eq!(bincode::serialize(&loaded).unwrap(), bytes);
}

#[test]
fn a_placeholder_builds_once_its_type_is_registered() {
    let bytes = bincode::serialize(&mixed_graph()).unwrap();
    let mut loaded: AnimationGraph = bincode::deserialize(&bytes).unwrap();
    loaded.make_live(&registry(false), None).unwrap();
    assert!(live_node::<Gain>(&loaded).is_none());

    loaded.make_live(&registry(true), None).unwrap();

    assert!(loaded.load_warnings().is_empty());
    assert_eq!(live_node::<Gain>(&loaded).unwrap().amount, 2.5);
    assert_eq!(bincode::serialize(&loaded).unwrap(), bytes);
}

#[test]
fn a_clip_handle_in_a_node_comes_back_strong() {
    let mut server = AssetServer::new();
    server.register_asset(&AssetStore::<AnimationClip>::new());
    server.register_asset(&AssetStore::<AnimationGraph>::new());

    let mut graph = AnimationGraph::from_node(AnimationClipNode::new(clip("outer_pose")));
    graph.make_live(&registry(false), Some(&server)).unwrap();

    assert!(graph.load_warnings().is_empty());
    let handle = live_node::<AnimationClipNode>(&graph).unwrap().clip();
    assert!(matches!(handle, AssetHandle::Strong(..)));
    assert_eq!(handle.id(), AssetId::from_path("outer_pose"));
}

fn play_rate_param_of(value: &ReflectedValue) -> &ReflectedValue {
    let ReflectedValue::Struct(fields) = value else {
        panic!("a clip node is stored as a struct");
    };
    fields
        .iter()
        .find_map(|(name, value)| (name == "play_rate_param").then_some(value))
        .expect("play_rate_param is stored")
}

fn load_clip_node(
    registry: &TypeRegistry,
    stored: ReflectedValue,
) -> (ReflectedValue, Vec<ValueWarning>) {
    let data = *registry
        .get_type_data::<ReflectAnimationNode>(std::any::TypeId::of::<AnimationClipNode>())
        .unwrap();
    let mut leaves = LiveLeaves::from_parts(registry, None);
    let (built, warnings) = (data.from_value)(&stored, &mut leaves);
    ((data.to_value)(&*built).unwrap().unwrap(), warnings)
}

#[test]
fn a_stored_clip_node_without_a_play_rate_param_loads_with_no_param_and_no_warning() {
    let registry = registry(false);
    let node = AnimationClipNode::new(clip("outer_pose")).with_play_rate(1.5);
    let ReflectedValue::Struct(mut fields) = to_value(&node).unwrap() else {
        panic!("a clip node is stored as a struct");
    };
    fields.retain(|(name, _)| name != "play_rate_param");

    let (built, warnings) = load_clip_node(&registry, ReflectedValue::Struct(fields));

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        play_rate_param_of(&built),
        &to_value(&None::<Name>).unwrap()
    );
    assert_eq!(built, to_value(&node).unwrap());
}

#[test]
fn a_stored_clip_node_whose_play_rate_param_is_a_string_loads_with_one_warning_and_no_param() {
    let registry = registry(false);
    let node = AnimationClipNode::new(clip("outer_pose")).with_play_rate(1.5);
    let ReflectedValue::Struct(mut fields) = to_value(&node).unwrap() else {
        panic!("a clip node is stored as a struct");
    };
    for (name, value) in &mut fields {
        if name == "play_rate_param" {
            *value = to_value(&String::from("draw_speed")).unwrap();
        }
    }

    let (built, warnings) = load_clip_node(&registry, ReflectedValue::Struct(fields));

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(
        warnings[0].reason,
        "stored data is a different kind of value"
    );
    assert_eq!(
        play_rate_param_of(&built),
        &to_value(&None::<Name>).unwrap()
    );
    assert_eq!(built, to_value(&node).unwrap());
}

#[test]
fn animation_clip_encoding_matches_its_fixture() {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(7),
        AnimationChannel::new(
            vec![0.0, 0.5],
            AnimationChanelOutput::Rotation(vec![
                glam::Quat::IDENTITY,
                glam::Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
            ]),
        ),
    );
    clip.set_frame_rate(30.0);
    clip.add_event(
        0.25,
        AnimationEvent {
            name: name!("footstep_left"),
        },
    );

    check_encoding(&clip, encoding_dir());
}

#[test]
fn a_version_1_clip_migrates_to_a_clip_with_no_events() {
    let payload = std::fs::read(encoding_dir().join("AnimationClip.v1.bin")).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path("content/clips/turn.gasset"),
        references: Vec::new(),
        kind: "AnimationClip".to_string(),
        kind_version: 1,
        provenance: None,
    };
    let bytes = write_content_asset(&header, &payload).unwrap();

    let (_, clip) = decode_content_asset::<AnimationClip>(&bytes).expect("migrates");

    assert!(clip.events().is_empty());
    assert_eq!(clip.duration(), 0.5);
    assert_eq!(clip.frame_rate(), Some(30.0));
    assert_eq!(
        clip.get_channels(&Uuid::from_u128(7)).map(Vec::len),
        Some(1)
    );
}

fn round_trip_clip(clip: &AnimationClip) -> AnimationClip {
    bincode::deserialize(&bincode::serialize(clip).unwrap()).unwrap()
}

fn live_event<T: ClipEvent>(clip: &AnimationClip, index: usize) -> Option<&T> {
    match &clip.events()[index].event {
        ClipEventData::Live { value, .. } => value.downcast_ref::<T>(),
        ClipEventData::Stored { .. } => None,
    }
}

#[test]
fn a_loaded_animation_event_is_stored_until_made_live() {
    let mut clip = AnimationClip::default();
    clip.add_event(
        0.25,
        AnimationEvent {
            name: name!("footstep_left"),
        },
    );
    let mut loaded = round_trip_clip(&clip);
    assert!(matches!(
        loaded.events()[0].event,
        ClipEventData::Stored { .. }
    ));

    let warnings = loaded.make_live(&registry(false), None);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(loaded.events()[0].time, 0.25);
    assert_eq!(
        live_event::<AnimationEvent>(&loaded, 0),
        Some(&AnimationEvent {
            name: name!("footstep_left"),
        })
    );
}

#[test]
fn a_registered_game_event_is_made_live_with_its_stored_fields() {
    let mut clip = AnimationClip::default();
    clip.add_event(0.5, Footstep { strength: 2.5 });
    let mut loaded = round_trip_clip(&clip);
    let mut registry = registry(false);
    registry.register::<Footstep>();

    let warnings = loaded.make_live(&registry, None);

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        live_event::<Footstep>(&loaded, 0),
        Some(&Footstep { strength: 2.5 })
    );
}

#[test]
fn an_unregistered_stored_event_is_dropped_with_one_warning_naming_its_type() {
    let mut clip = AnimationClip::default();
    clip.add_event(0.1, Footstep { strength: 1.0 });
    clip.add_event(
        0.2,
        AnimationEvent {
            name: name!("land"),
        },
    );
    let mut loaded = round_trip_clip(&clip);

    let warnings = loaded.make_live(&registry(false), None);

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].path, std::any::type_name::<Footstep>());
    assert_eq!(loaded.events().len(), 1);
    assert_eq!(loaded.events()[0].time, 0.2);
    assert!(live_event::<AnimationEvent>(&loaded, 0).is_some());
}

struct ClipLoader {
    server: AssetServer,
    world: World,
}

impl ClipLoader {
    fn new(registry: TypeRegistry) -> Self {
        let mut server = AssetServer::new();
        let store = AssetStore::<AnimationClip>::new();
        server.register_asset(&store);
        let shared = AppTypeRegistry::default();
        *shared.write() = registry;
        server.set_type_registry(shared);
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        Self { server, world }
    }

    fn load(&mut self, clip: AnimationClip) -> &AnimationClip {
        let handle = self.server.add(clip);
        handle_asset_load_events(&mut self.world);
        let store = self
            .world
            .get_resource::<AssetStore<AnimationClip>>()
            .unwrap();
        store.get(&handle).expect("the clip loaded")
    }
}

#[test]
fn a_clip_added_to_the_asset_server_makes_its_stored_events_live() {
    let mut clip = AnimationClip::default();
    clip.add_event(
        0.25,
        AnimationEvent {
            name: name!("footstep_left"),
        },
    );
    let mut loader = ClipLoader::new(registry(false));

    let loaded = loader.load(round_trip_clip(&clip));

    assert_eq!(
        live_event::<AnimationEvent>(loaded, 0),
        Some(&AnimationEvent {
            name: name!("footstep_left"),
        })
    );
}

#[test]
fn a_handle_inside_a_game_event_comes_back_strong_when_its_clip_loads() {
    let mut clip = AnimationClip::default();
    clip.add_event(
        0.5,
        Impact {
            pose: self::clip("impact_pose"),
        },
    );
    let mut registry = registry(false);
    registry.register::<Impact>();
    let mut loader = ClipLoader::new(registry);

    let loaded = loader.load(round_trip_clip(&clip));

    let pose = &live_event::<Impact>(loaded, 0)
        .expect("the event is live")
        .pose;
    assert!(matches!(pose, AssetHandle::Strong(..)));
    assert_eq!(pose.id(), AssetId::from_path("impact_pose"));
}

#[test]
fn animation_graph_encoding_matches_its_fixture() {
    check_encoding(&mixed_graph(), encoding_dir());
}
