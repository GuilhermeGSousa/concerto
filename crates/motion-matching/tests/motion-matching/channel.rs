use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::channel::{
    extract_features, BodyMotion, FeatureChannel, FeatureChannelData, FrameContext,
    ReflectFeatureChannel,
};
use concerto_motion_matching::{BakeError, TrajectorySample};
use concerto_reflect::{to_value, Reflect, ReflectedValue, TypeRegistry};

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct ConstantChannel {
    value: f32,
    scale: f32,
}

impl FeatureChannel for ConstantChannel {
    fn dims(&self) -> usize {
        3
    }

    fn extract(&self, frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.value;
        out[1] = self.scale;
        out[2] = frame.trajectory.len() as f32;
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<ConstantChannel>();
    registry
}

fn stored(channel: &ConstantChannel) -> FeatureChannelData {
    FeatureChannelData::Stored {
        type_name: "ConstantChannel".into(),
        value: to_value(channel).unwrap(),
    }
}

fn loaded(mut channel: FeatureChannelData) -> Result<FeatureChannelData, BakeError> {
    channel.load(&registry())?;
    Ok(channel)
}

#[test]
fn a_stored_channel_builds_and_extracts() {
    let channel = ConstantChannel {
        value: 2.0,
        scale: 0.5,
    };
    let built = loaded(stored(&channel)).unwrap();
    let built = built.channel().unwrap();
    assert_eq!(built.dims(), 3);

    let skeleton = Skeleton::new(vec![], vec![], vec![], vec![], vec![]).unwrap();
    let locals: Vec<JointPose> = Vec::new();
    let models: Vec<Transform> = Vec::new();
    let velocities = Vec::new();
    let trajectory = vec![TrajectorySample::default(); 2];
    let frame = FrameContext {
        skeleton: &skeleton,
        local_pose: &locals,
        model_pose: &models,
        model_velocities: &velocities,
        simulation: Transform::default(),
        body: BodyMotion::default(),
        trajectory: &trajectory,
    };
    let mut out = [0.0; 3];
    built.extract(&frame, &mut out);
    assert_eq!(out, [2.0, 0.5, 2.0]);
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(FeatureChannel)]
struct TwinChannel {
    first: f32,
}

impl FeatureChannel for TwinChannel {
    fn dims(&self) -> usize {
        2
    }

    fn extract(&self, _frame: &FrameContext, out: &mut [f32]) {
        out[0] = self.first;
        out[1] = -self.first;
    }
}

#[test]
fn extract_features_lays_channels_out_in_list_order() {
    let channels: [&dyn FeatureChannel; 2] = [
        &TwinChannel { first: 9.0 },
        &ConstantChannel {
            value: 1.0,
            scale: 2.0,
        },
    ];
    let skeleton = Skeleton::new(vec![], vec![], vec![], vec![], vec![]).unwrap();
    let frame = FrameContext {
        skeleton: &skeleton,
        local_pose: &[],
        model_pose: &[],
        model_velocities: &[],
        simulation: Transform::default(),
        body: BodyMotion::default(),
        trajectory: &[],
    };
    let mut out = [f32::NAN; 5];
    extract_features(&channels, &frame, &mut out);
    assert_eq!(out, [9.0, -9.0, 1.0, 2.0, 0.0]);
}

#[test]
fn a_built_channel_stores_back_unchanged() {
    let channel = ConstantChannel {
        value: -1.5,
        scale: 4.0,
    };
    let value = to_value(&channel).unwrap();
    let built = loaded(stored(&channel)).unwrap();
    let data = registry()
        .get_by_name("ConstantChannel")
        .unwrap()
        .data::<ReflectFeatureChannel>()
        .copied()
        .unwrap();
    assert_eq!((data.to_value)(built.channel().unwrap()).unwrap(), value);
}

#[test]
fn an_unregistered_type_name_is_named_in_the_error() {
    let missing = FeatureChannelData::Stored {
        type_name: "NoSuchChannel".into(),
        value: to_value(&ConstantChannel::default()).unwrap(),
    };
    let error = loaded(missing).unwrap_err();
    assert_eq!(
        error,
        BakeError::UnregisteredChannel("NoSuchChannel".into())
    );
}

#[test]
fn a_stored_value_that_does_not_apply_is_a_channel_value_error() {
    let unknown_field = FeatureChannelData::Stored {
        type_name: "ConstantChannel".into(),
        value: ReflectedValue::Struct(vec![("missing".into(), to_value(&1.0f32).unwrap())]),
    };
    let error = loaded(unknown_field).unwrap_err();
    match error {
        BakeError::ChannelValue { type_name, reason } => {
            assert_eq!(type_name, "ConstantChannel");
            assert!(reason.contains("missing"), "{reason}");
        }
        other => panic!("expected ChannelValue, got {other:?}"),
    }

    let wrong_kind = FeatureChannelData::Stored {
        type_name: "ConstantChannel".into(),
        value: ReflectedValue::Struct(vec![("value".into(), ReflectedValue::Struct(Vec::new()))]),
    };
    let error = loaded(wrong_kind).unwrap_err();
    assert!(
        matches!(&error, BakeError::ChannelValue { type_name, .. } if type_name == "ConstantChannel"),
        "{error:?}"
    );
}

#[test]
fn a_live_channel_is_named_by_its_short_type_name() {
    let channel = FeatureChannelData::live(ConstantChannel::default());
    assert_eq!(channel.type_name(), "ConstantChannel");
}

#[test]
fn a_live_channel_decodes_stored_and_loads_back_equal() {
    let live = FeatureChannelData::live(ConstantChannel {
        value: 3.0,
        scale: -2.0,
    });
    let bytes = bincode::serialize(&live).unwrap();
    let decoded: FeatureChannelData = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(decoded, FeatureChannelData::Stored { .. }));
    assert_eq!(decoded, live);

    let reloaded = loaded(decoded).unwrap();
    assert!(matches!(reloaded, FeatureChannelData::Live { .. }));
    assert_eq!(reloaded, live);
}

#[test]
fn a_cloned_live_channel_stays_live() {
    let live = FeatureChannelData::live(TwinChannel { first: 4.0 });
    let clone = live.clone();
    assert_eq!(clone.channel().unwrap().dims(), 2);
    assert_eq!(clone, live);
}

#[test]
fn a_channel_that_was_never_loaded_has_no_live_channel() {
    let channel = stored(&ConstantChannel::default());
    assert_eq!(
        channel.channel().err(),
        Some(BakeError::UnloadedChannel("ConstantChannel".into()))
    );
}
