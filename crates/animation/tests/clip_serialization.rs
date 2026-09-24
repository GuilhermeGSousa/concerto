//! Covers AnimationClip round-tripping through bincode so it can be imported
//! as a standalone sub-asset addressable as "file.gltf#animation/0".
use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use glam::Vec3;
use uuid::Uuid;

#[test]
fn animation_clip_round_trips_through_bincode() {
    let bone_id = Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    let mut clip = AnimationClip::default();
    clip.add_channel(
        bone_id,
        AnimationChannel::new(
            vec![0.0, 0.5, 1.0],
            AnimationChanelOutput::Translation(vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
            ]),
        ),
    );

    let bytes = bincode::serialize(&clip).expect("AnimationClip must serialize");
    let decoded: AnimationClip =
        bincode::deserialize(&bytes).expect("AnimationClip must round-trip");

    let channels = decoded
        .get_channels(&bone_id)
        .expect("the channel must survive keyed by the same bone id");
    assert_eq!(
        channels.len(),
        1,
        "one channel was added, one must come back"
    );
}

#[test]
fn skeleton_round_trips_through_bincode() {
    use concerto_mesh::skeleton::Skeleton;
    use glam::Mat4;

    let skeleton = Skeleton::from(vec![Mat4::IDENTITY, Mat4::from_translation(Vec3::X)]);
    let bytes = bincode::serialize(&skeleton).expect("Skeleton must serialize");
    let decoded: Skeleton = bincode::deserialize(&bytes).expect("Skeleton must round-trip");

    assert_eq!(
        decoded.inverse_bindposes.len(),
        2,
        "both inverse bind poses must survive the round-trip"
    );
}
