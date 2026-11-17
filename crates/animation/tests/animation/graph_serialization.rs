use std::path::Path;

use concerto_animation::TransitionBlender;
use concerto_animation::clip::{
    Additive, AdditiveBase, AnimationChanelOutput, AnimationChannel, AnimationClip, ClipEventData,
    ClipRange,
};
use concerto_animation::evaluation::AnimationGraphContext;
use concerto_animation::event::{AnimationEvent, ClipEvent, ReflectClipEvent};
use concerto_animation::graph::{
    AnimationGraph, AnimationNodeData, AnimationNodeIndex, ConnectError,
};
use concerto_animation::mask::{BoneMask, BoneMaskBranch};
use concerto_animation::node::blend_space::BlendSpace2DNode;
use concerto_animation::node::state_machine::{AnimationFSMTrigger, AnimationStateMachine};
use concerto_animation::node::{
    AdditiveNode, AimOffsetNode, AimPoint, AnimationClipNode, AnimationNode, AnimationPlayMode,
    BlendNode, ErasedAnimationNode, IkGoal, InputSlots, LayerNode, LayerSpace,
    ReflectAnimationNode, TipRotation, TwoBoneIkNode,
};
use concerto_animation::plugin::register_animation_types;
use concerto_animation::pose::{AnimationFrame, EvaluatedFrame, FramePool};
use concerto_animation::retarget::{
    BoneCorrection, ReferencePose, RetargetChain, RetargetGoal, RetargetGoalSettings, RetargetNode,
    RetargetRig,
};
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
use proptest::prelude::*;
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

    fn input_slots(&self) -> InputSlots {
        InputSlots::None
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

    fn update(&self, _: &mut (), _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn evaluate(
        &self,
        _: &mut (),
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        _: &[EvaluatedFrame],
        _: &mut FramePool,
        _: &mut AnimationFrame,
    ) {
    }

    fn reset(&self, _: &mut ()) {}
}

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
        .add_input(Gain { amount: 2.5 })
        .add_input(
            AnimationClipNode::new(clip("outer_pose"))
                .with_play_rate(1.5)
                .with_play_rate_param(name!("draw_speed")),
        )
        .add_input(state_machine())
        .add_blend_space_2d_input_with(name!("movement"), |ctx| {
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
fn a_mixed_graph_loaded_from_its_bytes_matches_the_one_built_in_code_node_for_node() {
    let registry = registry(true);
    let mut built = mixed_graph();
    built.make_live(&registry, None).unwrap();
    let mut restored = round_trip(&mixed_graph());
    assert!(!restored.is_live());

    restored.make_live(&registry, None).unwrap();

    assert!(restored.is_live());
    assert!(restored.load_warnings().is_empty());
    assert_eq!(restored.iter().count(), 7);
    assert_eq!(
        built.nodes().collect::<Vec<_>>(),
        restored.nodes().collect::<Vec<_>>()
    );
    assert_eq!(live_type_names(&built), live_type_names(&restored));
    assert_eq!(live_node::<Gain>(&restored).unwrap().amount, 2.5);
    assert_eq!(
        live_node::<AnimationClipNode>(&restored)
            .unwrap()
            .clip()
            .id(),
        AssetId::from_path("outer_pose")
    );
    let blend_space = live_node::<BlendSpace2DNode>(&restored).expect("blend space survived");
    assert_eq!(blend_space.input().param, name!("movement").into());
    assert_eq!(
        blend_space.points(),
        &[glam::Vec2::ZERO, glam::Vec2::new(0.0, 1.0)]
    );
    let machine = live_node::<AnimationStateMachine>(&restored).expect("state machine survived");
    let triggers: Vec<&AnimationFSMTrigger> = machine.triggers().collect();
    assert!(triggers.iter().any(|trigger| matches!(
        trigger,
        AnimationFSMTrigger::BoolEquals { param, value: true } if *param == name!("go").into()
    )));
    assert!(triggers.iter().any(|trigger| matches!(
        trigger,
        AnimationFSMTrigger::Vec2NonZero { param } if *param == name!("move").into()
    )));
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
fn an_unregistered_node_type_loads_as_a_placeholder_with_a_warning_and_builds_once_registered() {
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

    loaded.make_live(&registry(true), None).unwrap();

    assert!(loaded.load_warnings().is_empty());
    assert_eq!(live_node::<Gain>(&loaded).unwrap().amount, 2.5);
    assert_eq!(bincode::serialize(&loaded).unwrap(), bytes);
}

fn restored<T: AnimationNode + PartialEq + std::fmt::Debug>(graph: &mut AnimationGraph, node: &T) {
    graph.make_live(&registry(false), None).unwrap();

    let mut restored = round_trip(graph);
    restored.make_live(&registry(false), None).unwrap();

    assert!(
        restored.load_warnings().is_empty(),
        "{:?}",
        restored.load_warnings()
    );
    assert_eq!(live_node::<T>(&restored), Some(node));
    assert_eq!(
        graph.nodes().collect::<Vec<_>>(),
        restored.nodes().collect::<Vec<_>>()
    );
}

#[test]
fn the_ik_layer_and_aim_offset_nodes_keep_every_field_and_their_inputs_through_a_round_trip() {
    let ik = TwoBoneIkNode {
        tip: "hand_l".into(),
        target: IkGoal::Bone {
            bone: "hand_r".into(),
            translation: glam::Vec3::new(0.05, -0.02, 0.1),
            rotation: glam::Quat::from_rotation_y(0.4),
        },
        pole: Some(IkGoal::Param {
            param: name!("elbow_pole").into(),
        }),
        tip_rotation: TipRotation::FromTarget,
        softness: 0.03,
        weight: 0.75,
        weight_param: Some(name!("grip_weight").into()),
    };
    assert_ne!(ik, TwoBoneIkNode::default());
    restored(&mut AnimationGraph::from_node(ik.clone()), &ik);

    let layer = LayerNode::new(AssetHandle::weak(AssetId::from_path("upper_body")))
        .with_space(LayerSpace::Local)
        .with_weight(0.6)
        .with_weight_param(name!("strength"));
    assert_ne!(layer, LayerNode::default());
    let mut layered = AnimationGraph::new();
    layered.result_node().add_layer_input(
        layer.clone(),
        AnimationClipNode::new(clip("locomotion")),
        AnimationClipNode::new(clip("aim")),
    );
    assert_eq!(layered.iter().count(), 4);
    restored(&mut layered, &layer);

    let aim = AimOffsetNode::new(name!("aim_target"), "Spine2")
        .with_point(AimPoint::new(clip("aim_up"), 0.0, 0.6))
        .with_point(AimPoint::new(clip("aim_left"), -0.8, 0.1))
        .with_forward(glam::Vec3::X)
        .with_up(glam::Vec3::Z)
        .with_weight(0.7)
        .with_weight_param(name!("aim_weight"))
        .with_weight_halflife(0.3)
        .with_angle_halflife(0.05);
    assert_ne!(aim, AimOffsetNode::default());
    restored(&mut AnimationGraph::from_node(aim.clone()), &aim);
}

#[test]
fn an_additive_node_keeps_every_field_through_a_round_trip_and_its_clip_comes_back_strong() {
    let node = AdditiveNode::new(clip("cold_overlay"))
        .with_weight(0.6)
        .with_weight_param(name!("cold"))
        .with_play_mode(AnimationPlayMode::PlayOnce)
        .with_start_time(0.25)
        .with_play_rate(1.5)
        .with_play_rate_param(name!("shiver_speed"));
    assert_ne!(node, AdditiveNode::default());
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node.clone())
        .add_input(AnimationClipNode::new(clip("idle")));
    graph.make_live(&registry(false), None).unwrap();

    let mut restored = round_trip(&graph);
    restored.make_live(&registry(false), None).unwrap();

    assert!(
        restored.load_warnings().is_empty(),
        "{:?}",
        restored.load_warnings()
    );
    assert_eq!(live_node::<AdditiveNode>(&restored), Some(&node));
    assert_eq!(restored.iter().count(), 3);

    let mut server = AssetServer::new();
    server.register_asset(&AssetStore::<AnimationClip>::new());
    server.register_asset(&AssetStore::<AnimationGraph>::new());
    let mut loaded = round_trip(&graph);
    loaded.make_live(&registry(false), Some(&server)).unwrap();

    assert!(loaded.load_warnings().is_empty());
    let handle = &live_node::<AdditiveNode>(&loaded).unwrap().clip;
    assert!(matches!(handle, AssetHandle::Strong(..)));
    assert_eq!(handle.id(), AssetId::from_path("cold_overlay"));
}

fn retarget_node_handle_ids(node: &RetargetNode) -> [AssetId; 3] {
    [node.graph.id(), node.source.id(), node.target.id()]
}

#[test]
fn a_retarget_node_keeps_every_field_through_a_round_trip_and_its_handles_come_back_strong() {
    let node = RetargetNode::new(
        AssetHandle::weak(AssetId::from_path("lafan1_locomotion")),
        AssetHandle::weak(AssetId::from_path("lafan1_rig")),
        AssetHandle::weak(AssetId::from_path("ual1_rig")),
    )
    .with_goal(
        RetargetGoalSettings::new("foot_l")
            .with_weight(0.25)
            .with_softness(0.05),
    );
    let mut graph = AnimationGraph::from_node(node.clone());
    graph.make_live(&registry(false), None).unwrap();

    let mut restored = round_trip(&graph);
    restored.make_live(&registry(false), None).unwrap();

    assert!(
        restored.load_warnings().is_empty(),
        "{:?}",
        restored.load_warnings()
    );
    assert_eq!(
        retarget_node_handle_ids(live_node::<RetargetNode>(&restored).unwrap()),
        retarget_node_handle_ids(&node)
    );
    assert_eq!(
        live_node::<RetargetNode>(&restored).unwrap().goals,
        node.goals
    );

    let mut server = AssetServer::new();
    server.register_asset(&AssetStore::<AnimationClip>::new());
    server.register_asset(&AssetStore::<AnimationGraph>::new());
    server.register_asset(&AssetStore::<RetargetRig>::new());
    let mut loaded = round_trip(&graph);
    loaded.make_live(&registry(false), Some(&server)).unwrap();

    assert!(loaded.load_warnings().is_empty());
    let loaded = live_node::<RetargetNode>(&loaded).unwrap();
    assert!(matches!(loaded.graph, AssetHandle::Strong(..)));
    assert!(matches!(loaded.source, AssetHandle::Strong(..)));
    assert!(matches!(loaded.target, AssetHandle::Strong(..)));
    assert_eq!(
        retarget_node_handle_ids(loaded),
        retarget_node_handle_ids(&node)
    );
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

struct LoadedNode {
    built: ReflectedValue,
    warnings: Vec<ValueWarning>,
}

fn load_clip_node(registry: &TypeRegistry, stored: ReflectedValue) -> LoadedNode {
    let data = *registry
        .get_type_data::<ReflectAnimationNode>(std::any::TypeId::of::<AnimationClipNode>())
        .unwrap();
    let mut leaves = LiveLeaves::from_parts(registry, None);
    let (built, warnings) = (data.from_value)(&stored, &mut leaves);
    LoadedNode {
        built: (data.to_value)(&*built).unwrap().unwrap(),
        warnings,
    }
}

struct StoredRate {
    name: &'static str,
    play_rate_param: Option<ReflectedValue>,
    warnings: usize,
    loaded: AnimationClipNode,
}

#[test]
fn a_stored_play_rate_param_loads_as_no_param_when_missing_or_a_string() {
    let registry = registry(false);
    let node = || AnimationClipNode::new(clip("outer_pose")).with_play_rate(1.5);
    let draw_speed = to_value(&Some(name!("draw_speed"))).unwrap();
    let cases = [
        StoredRate {
            name: "missing",
            play_rate_param: None,
            warnings: 0,
            loaded: node(),
        },
        StoredRate {
            name: "a string",
            play_rate_param: Some(to_value(&String::from("draw_speed")).unwrap()),
            warnings: 1,
            loaded: node(),
        },
    ];

    for case in cases {
        let ReflectedValue::Struct(mut fields) = to_value(&node()).unwrap() else {
            panic!("a clip node is stored as a struct");
        };
        fields.retain(|(name, _)| name != "play_rate_param");
        let stored_name = case.play_rate_param.as_ref() == Some(&draw_speed);
        if let Some(value) = case.play_rate_param {
            fields.push(("play_rate_param".to_owned(), value));
        }

        let loaded = load_clip_node(&registry, ReflectedValue::Struct(fields));

        assert_eq!(
            loaded.warnings.len(),
            case.warnings,
            "{}: {:?}",
            case.name,
            loaded.warnings
        );
        let expected = if stored_name {
            draw_speed.clone()
        } else {
            to_value(&None::<Name>).unwrap()
        };
        assert_eq!(
            play_rate_param_of(&loaded.built),
            &expected,
            "{}",
            case.name
        );
        assert_eq!(
            loaded.built,
            to_value(&case.loaded).unwrap(),
            "{}",
            case.name
        );
    }
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
    clip.set_additive(Additive::LocalSpace(
        AdditiveBase::new(AssetHandle::weak(AssetId::from_bytes(
            Uuid::from_u128(11).into_bytes(),
        )))
        .with_base_range(ClipRange::new(0.0, 1.0))
        .with_range(ClipRange::new(0.25, 0.5))
        .with_root(Uuid::from_u128(7)),
    ));

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
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let bytes = write_content_asset(&header, &payload).unwrap();

    let (_, clip) = decode_content_asset::<AnimationClip>(&bytes).expect("migrates");

    assert_eq!(clip.additive(), &Additive::None);
    assert!(clip.events().is_empty());
    assert_eq!(clip.duration(), 0.5);
    assert_eq!(clip.frame_rate(), Some(30.0));
    assert_eq!(
        clip.get_channels(&Uuid::from_u128(7)).map(Vec::len),
        Some(1)
    );
}

#[test]
fn a_version_2_clip_migrates_to_a_clip_that_is_not_additive() {
    let payload = std::fs::read(encoding_dir().join("AnimationClip.v2.bin")).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path("content/clips/turn.gasset"),
        references: Vec::new(),
        kind: "AnimationClip".to_string(),
        kind_version: 2,
        provenance: None,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let bytes = write_content_asset(&header, &payload).unwrap();

    let (_, clip) = decode_content_asset::<AnimationClip>(&bytes).expect("migrates");

    assert_eq!(clip.additive(), &Additive::None);
    assert_eq!(clip.events().len(), 1);
    assert_eq!(clip.events()[0].time, 0.25);
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
fn a_clip_added_to_the_asset_server_makes_its_stored_events_live_with_their_handles_strong() {
    let mut clip = AnimationClip::default();
    clip.add_event(
        0.25,
        AnimationEvent {
            name: name!("footstep_left"),
        },
    );
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

    assert_eq!(
        live_event::<AnimationEvent>(loaded, 0),
        Some(&AnimationEvent {
            name: name!("footstep_left"),
        })
    );
    let pose = &live_event::<Impact>(loaded, 1)
        .expect("the event is live")
        .pose;
    assert!(matches!(pose, AssetHandle::Strong(..)));
    assert_eq!(pose.id(), AssetId::from_path("impact_pose"));
}

#[test]
fn animation_graph_encoding_matches_its_fixture() {
    check_encoding(&mixed_graph(), encoding_dir());
}

#[test]
fn bone_mask_encoding_matches_its_fixture() {
    let mask = BoneMask::new()
        .with_preview_skeleton(AssetHandle::weak(AssetId::from_bytes(
            Uuid::from_u128(13).into_bytes(),
        )))
        .with_branch(BoneMaskBranch::new("Spine1"))
        .with_branch(
            BoneMaskBranch::new("LeftArm")
                .with_weight(0.5)
                .with_depth(2)
                .with_descendants(false),
        );

    check_encoding(&mask, encoding_dir());
}

fn version_1_rig() -> RetargetRig {
    RetargetRig::new()
        .with_skeleton(AssetHandle::weak(AssetId::from_bytes(
            Uuid::from_u128(13).into_bytes(),
        )))
        .with_root("pelvis")
        .with_motion_root("root")
        .with_chain(RetargetChain::new("spine", "spine_01", "spine_03"))
        .with_chain(RetargetChain::new("head", "Head", "Head"))
        .with_reference(ReferencePose::ClipFrame {
            clip: AssetHandle::weak(AssetId::from_bytes(Uuid::from_u128(14).into_bytes())),
            time: 0.25,
        })
        .with_correction(BoneCorrection::new(
            "upperarm_l",
            glam::Quat::from_xyzw(0.0, 0.0, 0.5, 0.75),
        ))
}

#[test]
fn retarget_rig_encoding_matches_its_fixture() {
    let rig = version_1_rig().with_goal(RetargetGoal::new("foot_l", "foot_l"));

    check_encoding(&rig, encoding_dir());
}

#[test]
fn a_version_1_retarget_rig_migrates_to_a_rig_with_no_goals() {
    let payload = std::fs::read(encoding_dir().join("RetargetRig.v1.bin")).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path("content/rigs/ual1.gasset"),
        references: Vec::new(),
        kind: "RetargetRig".to_string(),
        kind_version: 1,
        provenance: None,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let bytes = write_content_asset(&header, &payload).unwrap();

    let (_, rig) = decode_content_asset::<RetargetRig>(&bytes).expect("migrates");

    assert_eq!(rig, version_1_rig());
    assert!(rig.goals.is_empty());
}

struct Refusal {
    parent: AnimationNodeIndex,
    child: AnimationNodeIndex,
    error: ConnectError,
}

#[test]
fn connect_refuses_what_would_stop_the_graph_being_a_tree_under_its_result() {
    let mut graph = AnimationGraph::new();
    let result = graph.result_index();
    let blend = graph.insert_node(AnimationNodeData::of(&BlendNode {}));
    let gain = graph.insert_node(AnimationNodeData::from_live(Box::new(Gain { amount: 1.0 })));
    let loose = graph.insert_node(AnimationNodeData::of(&BlendNode {}));
    graph.connect(result, blend).unwrap();
    graph.connect(blend, gain).unwrap();

    let cases = [
        Refusal {
            parent: loose,
            child: gain,
            error: ConnectError::ChildHasParent { child: gain },
        },
        Refusal {
            parent: loose,
            child: result,
            error: ConnectError::ChildIsResult,
        },
        Refusal {
            parent: loose,
            child: loose,
            error: ConnectError::Cycle,
        },
    ];

    for case in cases {
        assert_eq!(graph.connect(case.parent, case.child), Err(case.error));
    }
    graph.connect(gain, loose).unwrap();
    assert_eq!(
        graph.connect(loose, blend),
        Err(ConnectError::ChildHasParent { child: blend })
    );

    assert_eq!(graph.inputs(result), vec![blend]);
    assert_eq!(graph.inputs(blend), vec![gain]);
    assert_eq!(graph.inputs(gain), vec![loose]);
    assert!(graph.inputs(loose).is_empty());
}

#[derive(Clone, Debug)]
struct ForestNode {
    gain: bool,
    parent: Option<prop::sample::Index>,
}

#[derive(Clone, Debug)]
struct ForestLink {
    parent: usize,
    child: usize,
}

#[derive(Clone, Debug)]
struct Forest {
    nodes: Vec<ForestNode>,
    links: Vec<ForestLink>,
}

#[derive(Debug, PartialEq)]
struct ReadNode {
    type_name: String,
    inputs: Vec<usize>,
}

fn forest() -> impl Strategy<Value = Forest> {
    let node = (
        any::<bool>(),
        prop::option::of(any::<prop::sample::Index>()),
    )
        .prop_map(|(gain, parent)| ForestNode { gain, parent });
    prop::collection::vec(node, 0..16).prop_flat_map(|nodes| {
        let links: Vec<ForestLink> = nodes
            .iter()
            .enumerate()
            .filter_map(|(position, node)| {
                Some(ForestLink {
                    parent: node.parent?.index(position + 1),
                    child: position + 1,
                })
            })
            .collect();
        Just(links).prop_shuffle().prop_map(move |links| Forest {
            nodes: nodes.clone(),
            links,
        })
    })
}

fn read(graph: &AnimationGraph) -> Vec<ReadNode> {
    graph
        .node_indices()
        .map(|index| ReadNode {
            type_name: graph.node(index).unwrap().type_name().to_owned(),
            inputs: graph
                .inputs(index)
                .into_iter()
                .map(|input| input.index())
                .collect(),
        })
        .collect()
}

proptest! {
    #[test]
    fn a_graph_built_from_any_forest_saves_and_loads_as_that_forest_with_every_input_in_its_slot(
        forest in forest(),
    ) {
        let mut graph = AnimationGraph::new();
        let mut indices = vec![graph.result_index()];
        let mut expected = vec![ReadNode {
            type_name: graph.node(graph.result_index()).unwrap().type_name().to_owned(),
            inputs: Vec::new(),
        }];
        for node in &forest.nodes {
            let data = if node.gain {
                AnimationNodeData::of(&Gain { amount: 0.5 })
            } else {
                AnimationNodeData::of(&BlendNode {})
            };
            expected.push(ReadNode {
                type_name: data.type_name().to_owned(),
                inputs: Vec::new(),
            });
            indices.push(graph.insert_node(data));
        }
        for link in &forest.links {
            graph.connect(indices[link.parent], indices[link.child]).unwrap();
            expected[link.parent].inputs.push(indices[link.child].index());
        }

        prop_assert_eq!(&read(&graph), &expected);
        prop_assert_eq!(&read(&round_trip(&graph)), &expected);
    }
}
