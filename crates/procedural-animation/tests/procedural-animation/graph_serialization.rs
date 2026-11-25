use concerto_animation::graph::AnimationGraph;
use concerto_animation::ik::Hinge;
use concerto_animation::mask::BoneMask;
use concerto_animation::node::{AnimationNode, ErasedAnimationNode};
use concerto_animation::plugin::AnimationPlugin;
use concerto_app::{App, plugins::AssetManagerPlugin};
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_name::name;
use concerto_procedural_animation::{
    AimBone, AimNode, FootLockLeg, FootLockNode, FootPlacementLeg, FootPlacementNode, LookAtBone,
    LookAtEye, LookAtNode, Muzzle, ProceduralAnimationPlugin, ProceduralMotionChannel,
    ProceduralMotionNode, Waveform,
};

fn round_trip(graph: &AnimationGraph) -> AnimationGraph {
    let bytes = bincode::serialize(graph).expect("serialize");
    bincode::deserialize(&bytes).expect("deserialize")
}

fn live_node<T: AnimationNode>(graph: &AnimationGraph) -> Option<&T> {
    graph
        .live_nodes()
        .find_map(|node: &dyn ErasedAnimationNode| node.as_reflect().downcast_ref::<T>())
}

fn foot_placement() -> FootPlacementNode {
    let node = FootPlacementNode::new("Hips")
        .with_leg(
            FootPlacementLeg::new("LeftFoot")
                .with_toe("LeftToe")
                .with_hinge(Hinge::Rest),
        )
        .with_leg(FootPlacementLeg::new("RightFoot"))
        .with_max_step_up(0.4)
        .with_max_step_down(0.6)
        .with_max_pelvis_drop(0.3)
        .with_max_foot_angle(0.4)
        .with_foot_halflife(0.07)
        .with_pelvis_halflife(0.12)
        .with_rotation_halflife(0.08)
        .with_softness(0.02)
        .with_weight(0.8)
        .with_weight_param(name!("foot_placement_weight"))
        .with_weight_halflife(0.25)
        .with_ignore_param(name!("foot_placement_ignore"));
    let defaults = FootPlacementNode::default();
    assert_ne!(node.pelvis, defaults.pelvis);
    assert_ne!(node.max_step_up, defaults.max_step_up);
    assert_ne!(node.max_step_down, defaults.max_step_down);
    assert_ne!(node.max_pelvis_drop, defaults.max_pelvis_drop);
    assert_ne!(node.max_foot_angle, defaults.max_foot_angle);
    assert_ne!(node.foot_halflife, defaults.foot_halflife);
    assert_ne!(node.pelvis_halflife, defaults.pelvis_halflife);
    assert_ne!(node.rotation_halflife, defaults.rotation_halflife);
    assert_ne!(node.softness, defaults.softness);
    assert_ne!(node.weight, defaults.weight);
    assert_ne!(node.weight_halflife, defaults.weight_halflife);
    node
}

fn look_at() -> LookAtNode {
    let node = LookAtNode {
        chain: vec![
            LookAtBone {
                bone: "spine_02".into(),
                share: 0.3,
                yaw_limit: 0.5,
                pitch_limit: 0.25,
                halflife: 0.2,
            },
            LookAtBone {
                bone: "head".into(),
                share: 0.7,
                yaw_limit: 0.9,
                pitch_limit: 0.4,
                halflife: 0.1,
            },
        ],
        eyes: vec![
            LookAtEye {
                bone: "eye_l".into(),
                yaw_limit: 0.6,
                pitch_limit: 0.3,
                halflife: 0.02,
            },
            LookAtEye {
                bone: "eye_r".into(),
                yaw_limit: 0.55,
                pitch_limit: 0.35,
                halflife: 0.04,
            },
        ],
        forward: glam::Vec3::new(0.0, 0.1, 1.0),
        up: glam::Vec3::new(0.1, 1.0, 0.0),
        target_param: name!("gaze_target").into(),
        weight: 0.8,
        weight_param: Some(name!("gaze_weight").into()),
        weight_halflife: 0.35,
        max_angle: 1.5,
    };
    assert_ne!(node, LookAtNode::default());
    node
}

fn aim() -> AimNode {
    let node = AimNode::new(
        name!("aim_target"),
        Muzzle::new("hand_r")
            .with_offset(glam::Vec3::new(0.02, 0.05, 0.3))
            .with_direction(glam::Vec3::new(0.0, 0.1, 1.0)),
    )
    .with_bone(
        AimBone::new("spine_02")
            .with_share(0.4)
            .with_limits(0.6, 0.3),
    )
    .with_bone(
        AimBone::new("spine_03")
            .with_share(0.6)
            .with_limits(0.5, 0.35),
    )
    .with_up(glam::Vec3::new(0.1, 1.0, 0.0))
    .with_weight(0.8)
    .with_weight_param(name!("aim_weight"))
    .with_weight_halflife(0.35)
    .with_reacquire_halflife(0.15)
    .with_reacquire_param(name!("aim_reacquire"));
    let defaults = AimNode::default();
    assert_ne!(node.muzzle, Muzzle::default());
    assert_ne!(node.chain[0], AimBone::default());
    assert_ne!(node.up, defaults.up);
    assert_ne!(node.target_param, defaults.target_param);
    assert_ne!(node.weight, defaults.weight);
    assert_ne!(node.weight_halflife, defaults.weight_halflife);
    assert_ne!(node.reacquire_halflife, defaults.reacquire_halflife);
    node
}

fn procedural_motion() -> ProceduralMotionNode {
    let mask = |path: &str| AssetHandle::<BoneMask>::weak(AssetId::from_path(path));
    let changed = |channel: ProceduralMotionChannel, path: &str, waveform: Waveform| {
        let changed = channel
            .clone()
            .with_waveform(waveform)
            .with_frequency(channel.frequency * 1.5)
            .with_amplitude(channel.amplitude + glam::Vec3::new(0.01, 0.02, 0.03))
            .with_magnitude_param(name!("wounded"))
            .with_magnitude_halflife(0.3);
        let changed = ProceduralMotionChannel {
            mask: mask(path),
            ..changed
        };
        assert_ne!(changed.mask, channel.mask);
        assert_ne!(changed.waveform, channel.waveform);
        assert_ne!(changed.frequency, channel.frequency);
        assert_ne!(changed.amplitude, channel.amplitude);
        assert_ne!(changed.magnitude_param, channel.magnitude_param);
        assert_ne!(changed.magnitude_halflife, channel.magnitude_halflife);
        changed
    };
    ProceduralMotionNode::new()
        .with_channel(changed(
            ProceduralMotionChannel::breathing(mask("chest")),
            "spine",
            Waveform::Noise { octaves: 3 },
        ))
        .with_channel(changed(
            ProceduralMotionChannel::sway(mask("arms")),
            "upper_body",
            Waveform::Sine,
        ))
        .with_channel(changed(
            ProceduralMotionChannel::tremor(mask("hands")),
            "fingers",
            Waveform::Noise { octaves: 5 },
        ))
        .with_seed(7)
        .with_seed_param(name!("character_seed"))
}

fn foot_lock() -> FootLockNode {
    let node = FootLockNode::new()
        .with_leg(
            FootLockLeg::new("LeftFoot")
                .with_toe("LeftToe")
                .with_hinge(Hinge::Rest),
        )
        .with_leg(FootLockLeg::new("RightFoot"))
        .with_plant_speed(0.2)
        .with_plant_height(0.08)
        .with_unlock_radius(0.25)
        .with_release_halflife(0.12)
        .with_softness(0.02)
        .with_weight(0.8)
        .with_weight_param(name!("foot_lock_weight"))
        .with_weight_halflife(0.25);
    let defaults = FootLockNode::default();
    assert_ne!(node.legs, defaults.legs);
    assert_ne!(node.plant_speed, defaults.plant_speed);
    assert_ne!(node.plant_height, defaults.plant_height);
    assert_ne!(node.unlock_radius, defaults.unlock_radius);
    assert_ne!(node.release_halflife, defaults.release_halflife);
    assert_ne!(node.softness, defaults.softness);
    assert_ne!(node.weight, defaults.weight);
    assert_ne!(node.weight_param, defaults.weight_param);
    assert_ne!(node.weight_halflife, defaults.weight_halflife);
    node
}

#[test]
fn a_graph_of_every_node_the_plugin_registers_keeps_every_field_through_a_round_trip() {
    let mut app = App::new();
    app.register_plugin(AssetManagerPlugin);
    app.register_plugin(AnimationPlugin);
    app.register_plugin(ProceduralAnimationPlugin);
    let registry = app.get_resource::<AppTypeRegistry>().unwrap().read();
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(foot_placement())
        .input(foot_lock())
        .input(look_at())
        .input(aim())
        .add_input(procedural_motion());
    graph.make_live(&registry, None).unwrap();

    let mut restored = round_trip(&graph);
    restored.make_live(&registry, None).unwrap();

    assert!(
        restored.load_warnings().is_empty(),
        "{:?}",
        restored.load_warnings()
    );
    assert_eq!(
        live_node::<FootPlacementNode>(&restored),
        Some(&foot_placement())
    );
    assert_eq!(live_node::<FootLockNode>(&restored), Some(&foot_lock()));
    assert_eq!(live_node::<LookAtNode>(&restored), Some(&look_at()));
    assert_eq!(live_node::<AimNode>(&restored), Some(&aim()));
    assert_eq!(
        live_node::<ProceduralMotionNode>(&restored),
        Some(&procedural_motion())
    );
}
