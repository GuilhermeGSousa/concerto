use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton};
use concerto_motion_matching::channel::{
    build_channels, extract_features, BodyMotion, FeatureChannel, FrameContext,
    ReflectFeatureChannel,
};
use concerto_motion_matching::{BakeError, StoredChannel, TrajectorySample};
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

fn stored(channel: &ConstantChannel) -> StoredChannel {
    StoredChannel {
        type_name: "ConstantChannel".into(),
        value: to_value(channel).unwrap(),
    }
}

#[test]
fn registration_carries_the_feature_channel_type_data() {
    let registry = registry();
    let registration = registry.get_by_name("ConstantChannel").unwrap();
    assert!(registration.data::<ReflectFeatureChannel>().is_some());
}

#[test]
fn a_stored_channel_builds_and_extracts() {
    let channel = ConstantChannel {
        value: 2.0,
        scale: 0.5,
    };
    let built = build_channels(&[stored(&channel)], &registry()).unwrap();
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].dims(), 3);

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
    built[0].extract(&frame, &mut out);
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
    let channels: Vec<Box<dyn FeatureChannel>> = vec![
        Box::new(TwinChannel { first: 9.0 }),
        Box::new(ConstantChannel {
            value: 1.0,
            scale: 2.0,
        }),
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
    let stored = stored(&channel);
    let registry = registry();
    let built = build_channels(std::slice::from_ref(&stored), &registry).unwrap();
    let data = registry
        .get_by_name("ConstantChannel")
        .unwrap()
        .data::<ReflectFeatureChannel>()
        .copied()
        .unwrap();
    assert_eq!((data.to_value)(built[0].as_ref()).unwrap(), stored.value);
}

#[test]
fn an_unregistered_type_name_is_named_in_the_error() {
    let missing = StoredChannel {
        type_name: "NoSuchChannel".into(),
        value: to_value(&ConstantChannel::default()).unwrap(),
    };
    let error = build_channels(&[missing], &registry()).err().unwrap();
    assert_eq!(
        error,
        BakeError::UnregisteredChannel("NoSuchChannel".into())
    );
}

#[test]
fn a_stored_value_that_does_not_apply_is_a_channel_value_error() {
    let unknown_field = StoredChannel {
        type_name: "ConstantChannel".into(),
        value: ReflectedValue::Struct(vec![("missing".into(), to_value(&1.0f32).unwrap())]),
    };
    let error = build_channels(&[unknown_field], &registry()).err().unwrap();
    match error {
        BakeError::ChannelValue { type_name, reason } => {
            assert_eq!(type_name, "ConstantChannel");
            assert!(reason.contains("missing"), "{reason}");
        }
        other => panic!("expected ChannelValue, got {other:?}"),
    }

    let wrong_kind = StoredChannel {
        type_name: "ConstantChannel".into(),
        value: ReflectedValue::Struct(vec![("value".into(), ReflectedValue::Struct(Vec::new()))]),
    };
    let error = build_channels(&[wrong_kind], &registry()).err().unwrap();
    assert!(
        matches!(&error, BakeError::ChannelValue { type_name, .. } if type_name == "ConstantChannel"),
        "{error:?}"
    );
}
