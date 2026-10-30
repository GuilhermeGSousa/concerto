use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use concerto_animation::clip::AnimationClip;
use concerto_asset_import::{ImportContext, ImportError, ImportOutputs, Importer};
use concerto_bvh::bvh_importer::BvhImporter;
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::assets::AssetId;
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use concerto_reflect::{Reflect, apply_value};
use concerto_scene::scene::{Scene, SceneNode};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

const TWO_JOINTS: &str = "HIERARCHY
ROOT Hips
{
\tOFFSET 0.0 90.0 0.0
\tCHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
\tJOINT Spine
\t{
\t\tOFFSET 0.0 10.0 0.0
\t\tCHANNELS 3 Xrotation Yrotation Zrotation
\t\tEnd Site
\t\t{
\t\t\tOFFSET 0.0 5.0 0.0
\t\t}
\t}
}
MOTION
Frames: 3
Frame Time: 0.0333333
0 90 0 0 0 0 0 0 0
1 90 0 10 0 0 5 0 0
2 90 0 20 0 0 10 0 0
";

fn write_source(name: &str, text: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("bvh_importer");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn import(name: &str, text: &str) -> Result<(), ImportError> {
    let path = write_source(name, text);
    let mut ctx = ImportContext::new(PathBuf::from(name));
    BvhImporter.import(&path, &mut ctx)
}

fn assert_malformed(name: &str, text: &str, expected: &str) {
    match import(name, text) {
        Err(ImportError::MalformedSource { message, .. }) => assert!(
            message.contains(expected),
            "expected {expected:?} in {message:?}"
        ),
        other => panic!("expected MalformedSource for {name}, got {other:?}"),
    }
}

#[test]
fn a_well_formed_file_parses() {
    import("two_joints.bvh", TWO_JOINTS).expect("the two-joint fixture should parse");
}

#[test]
fn crlf_line_endings_parse() {
    import("crlf.bvh", &TWO_JOINTS.replace('\n', "\r\n")).expect("CRLF should parse");
}

#[test]
fn supports_the_bvh_extension() {
    assert_eq!(BvhImporter.supported_extensions(), &["bvh"]);
}

#[test]
fn a_non_positive_or_nan_frame_time_is_rejected() {
    for value in ["0", "-0.03", "NaN", "inf", "abc"] {
        let text = TWO_JOINTS.replace("0.0333333", value);
        assert_malformed("frame_time.bvh", &text, "Frame Time");
    }
}

#[test]
fn a_joint_name_containing_a_slash_is_rejected() {
    let text = TWO_JOINTS.replace("JOINT Spine", "JOINT Spine/Upper");
    assert_malformed("slash.bvh", &text, "'/'");
}

#[test]
fn an_unknown_channel_is_rejected() {
    let text = TWO_JOINTS.replace(
        "Zrotation Yrotation Xrotation",
        "Zrotation Yrotation Wrotation",
    );
    assert_malformed("channel.bvh", &text, "Wrotation");
}

#[test]
fn a_frame_line_with_the_wrong_value_count_is_rejected() {
    let text = TWO_JOINTS.replace("1 90 0 10 0 0 5 0 0", "1 90 0 10 0 0 5 0");
    assert_malformed("count.bvh", &text, "line 20");
}

#[test]
fn fewer_frame_lines_than_declared_are_rejected() {
    let text = TWO_JOINTS.replace("Frames: 3", "Frames: 4");
    assert_malformed("missing.bvh", &text, "Frames");
}

#[test]
fn a_huge_frame_count_is_rejected_without_allocating() {
    let text = TWO_JOINTS.replace("Frames: 3", "Frames: 99999999999999");
    assert_malformed("huge_frames.bvh", &text, "Frames");
}

#[test]
fn a_huge_channel_count_is_rejected_without_allocating() {
    let text = TWO_JOINTS.replace("CHANNELS 3", "CHANNELS 99999999999999");
    assert_malformed("huge_channels.bvh", &text, "channel");
}

#[test]
fn a_deeply_nested_hierarchy_is_rejected() {
    let mut text = String::from("HIERARCHY\nROOT J0\n{\nOFFSET 0 0 0\nCHANNELS 0\n");
    for depth in 1..1000 {
        text.push_str(&format!("JOINT J{depth}\n{{\nOFFSET 0 0 0\nCHANNELS 0\n"));
    }
    text.push_str(&"}\n".repeat(1000));
    text.push_str("MOTION\nFrames: 2\nFrame Time: 0.0333333\n\n\n");
    assert_malformed("deep.bvh", &text, "nested");
}

#[test]
fn unbalanced_braces_are_rejected() {
    let missing_close = TWO_JOINTS.replacen("}\nMOTION", "MOTION", 1);
    assert_malformed("open.bvh", &missing_close, "brace");
    let extra_close = TWO_JOINTS.replacen("MOTION", "}\nMOTION", 1);
    assert_malformed("close.bvh", &extra_close, "line");
}

#[test]
fn duplicate_sibling_names_are_rejected() {
    let text = TWO_JOINTS.replacen(
        "\t}\n}\nMOTION",
        "\t}\n\tJOINT Spine\n\t{\n\t\tOFFSET 0 1 0\n\t\tCHANNELS 0\n\t}\n}\nMOTION",
        1,
    );
    assert_malformed("duplicate.bvh", &text, "Spine");
}

#[test]
fn a_file_without_a_root_is_rejected() {
    assert_malformed(
        "no_root.bvh",
        "HIERARCHY\nMOTION\nFrames: 0\nFrame Time: 0.1\n",
        "ROOT",
    );
}

#[test]
fn more_than_one_root_is_rejected() {
    let text = TWO_JOINTS.replacen(
        "MOTION",
        "ROOT Other\n{\n\tOFFSET 0 0 0\n\tCHANNELS 0\n}\nMOTION",
        1,
    );
    assert_malformed("two_roots.bvh", &text, "ROOT");
}

#[test]
fn a_missing_hierarchy_header_is_rejected() {
    assert_malformed("header.bvh", "ROOT Hips\n", "HIERARCHY");
}

fn run(name: &str, text: &str, settings: &str) -> Result<ImportOutputs, ImportError> {
    let path = write_source(name, text);
    let table: toml::Table = settings.parse().unwrap();
    let mut ctx = ImportContext::new(PathBuf::from(name)).with_settings(table);
    BvhImporter.import(&path, &mut ctx)?;
    Ok(ctx.into_parts())
}

fn import_ok(name: &str, text: &str, settings: &str) -> (Skeleton, AnimationClip) {
    let outputs = run(name, text, settings).expect("the import should succeed");
    let decode = |sub: &str| {
        outputs
            .sub_assets
            .iter()
            .find(|asset| asset.name == sub)
            .unwrap_or_else(|| panic!("missing sub-asset {sub}"))
            .bytes
            .clone()
    };
    let stem = Path::new(name).file_stem().unwrap().to_str().unwrap();
    (
        bincode::deserialize(&decode("skeleton/0")).unwrap(),
        bincode::deserialize(&decode(&format!("animation/{stem}"))).unwrap(),
    )
}

fn assert_malformed_with(name: &str, text: &str, settings: &str, expected: &str) {
    match run(name, text, settings) {
        Err(ImportError::MalformedSource { message, .. }) => assert!(
            message.contains(expected),
            "expected {expected:?} in {message:?}"
        ),
        other => panic!("expected MalformedSource for {name}, got {:?}", other.err()),
    }
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-5
}

fn same_rotation(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() > 1.0 - 1e-5
}

fn path_id(path: &str) -> Uuid {
    let mut hasher = DefaultHasher::new();
    path.to_string().hash(&mut hasher);
    Uuid::from_u128(hasher.finish() as u128)
}

fn sample(clip: &AnimationClip, id: &Uuid, time: f32) -> JointPose {
    let mut pose = JointPose::IDENTITY;
    for channel in clip.get_channels(id).expect("channels for the bone") {
        channel.sample_transform(time, &mut pose);
    }
    pose
}

fn deg(value: f32) -> f32 {
    value.to_radians()
}

const FRAME_TIME: f32 = 0.0333333;

#[test]
fn the_skeleton_holds_the_hierarchy_rest_pose_and_inverse_bind_poses() {
    let (skeleton, _) = import_ok("skeleton.bvh", TWO_JOINTS, "");
    assert_eq!(&*skeleton.names, &["Hips".to_string(), "Spine".to_string()]);
    assert_eq!(&*skeleton.parents, &[None, Some(0)]);
    assert!(near(
        skeleton.rest_pose[0].translation,
        Vec3::new(0.0, 0.9, 0.0)
    ));
    assert!(near(
        skeleton.rest_pose[1].translation,
        Vec3::new(0.0, 0.1, 0.0)
    ));
    for pose in skeleton.rest_pose.iter() {
        assert_eq!(pose.rotation, Quat::IDENTITY);
        assert_eq!(pose.scale, Vec3::ONE);
    }
    let expected = [
        Mat4::from_translation(Vec3::new(0.0, -0.9, 0.0)),
        Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)),
    ];
    for (actual, expected) in skeleton.inverse_bindposes.iter().zip(expected) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "{actual:?} != {expected:?}"
        );
    }
}

#[test]
fn end_sites_produce_no_bone() {
    let (skeleton, _) = import_ok("end_site.bvh", TWO_JOINTS, "");
    assert_eq!(skeleton.bone_count(), 2);
}

#[test]
fn sampling_at_each_frame_time_gives_the_authored_pose() {
    let (skeleton, clip) = import_ok("sample.bvh", TWO_JOINTS, "");
    let hips = skeleton.bone_ids[0];
    let spine = skeleton.bone_ids[1];
    for frame in 0..3 {
        let time = frame as f32 * FRAME_TIME;
        let hips_pose = sample(&clip, &hips, time);
        assert!(near(
            hips_pose.translation,
            Vec3::new(frame as f32 * 0.01, 0.9, 0.0)
        ));
        assert!(same_rotation(
            hips_pose.rotation,
            Quat::from_rotation_z(deg(10.0 * frame as f32))
        ));
        let spine_pose = sample(&clip, &spine, time);
        assert!(same_rotation(
            spine_pose.rotation,
            Quat::from_rotation_x(deg(5.0 * frame as f32))
        ));
    }
    assert!((clip.duration() - 2.0 * FRAME_TIME).abs() < 1e-6);
}

#[test]
fn rotation_channels_compose_in_the_order_they_are_listed() {
    let text = "HIERARCHY
ROOT Hips
{
\tOFFSET 0 0 0
\tCHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
\tJOINT Spine
\t{
\t\tOFFSET 0 10 0
\t\tCHANNELS 3 Xrotation Yrotation Zrotation
\t\tEnd Site
\t\t{
\t\t\tOFFSET 0 5 0
\t\t}
\t}
}
MOTION
Frames: 2
Frame Time: 0.5
0 0 0 30 40 50 10 20 30
0 0 0 30 40 50 10 20 30
";
    let (skeleton, clip) = import_ok("order.bvh", text, "");
    let hips = sample(&clip, &skeleton.bone_ids[0], 0.0);
    let spine = sample(&clip, &skeleton.bone_ids[1], 0.0);
    let expected_hips = Quat::from_rotation_z(deg(30.0))
        * Quat::from_rotation_y(deg(40.0))
        * Quat::from_rotation_x(deg(50.0));
    let expected_spine = Quat::from_rotation_x(deg(10.0))
        * Quat::from_rotation_y(deg(20.0))
        * Quat::from_rotation_z(deg(30.0));
    assert!(same_rotation(hips.rotation, expected_hips));
    assert!(same_rotation(spine.rotation, expected_spine));
}

#[test]
fn a_joint_with_some_position_axes_keeps_the_rest_offset_on_the_others() {
    let text = "HIERARCHY
ROOT Hips
{
\tOFFSET 100 200 300
\tCHANNELS 2 Xposition Zposition
}
MOTION
Frames: 2
Frame Time: 1
10 30
20 40
";
    let (skeleton, clip) = import_ok("partial.bvh", text, "scale = 0.1");
    let first = sample(&clip, &skeleton.bone_ids[0], 0.0);
    let second = sample(&clip, &skeleton.bone_ids[0], 1.0);
    assert!(near(first.translation, Vec3::new(1.0, 20.0, 3.0)));
    assert!(near(second.translation, Vec3::new(2.0, 20.0, 4.0)));
    assert_eq!(first.rotation, Quat::IDENTITY);
}

#[test]
fn neighbouring_rotation_keys_stay_in_one_hemisphere() {
    let text = "HIERARCHY
ROOT Hips
{
\tOFFSET 0 0 0
\tCHANNELS 3 Zrotation Yrotation Xrotation
}
MOTION
Frames: 3
Frame Time: 1
0 0 0
359 0 0
358 0 0
";
    let (skeleton, clip) = import_ok("hemisphere.bvh", text, "");
    let id = skeleton.bone_ids[0];
    let keys: Vec<Quat> = (0..3)
        .map(|i| sample(&clip, &id, i as f32).rotation)
        .collect();
    assert!(keys[0].dot(keys[1]) >= 0.0);
    assert!(keys[1].dot(keys[2]) >= 0.0);
    assert!(same_rotation(keys[1], Quat::from_rotation_z(deg(359.0))));
}

#[test]
fn scale_applies_to_offsets_and_translation_keys() {
    let (skeleton, clip) = import_ok("scaled.bvh", TWO_JOINTS, "scale = 1.0");
    assert!(near(
        skeleton.rest_pose[0].translation,
        Vec3::new(0.0, 90.0, 0.0)
    ));
    assert!(near(
        skeleton.rest_pose[1].translation,
        Vec3::new(0.0, 10.0, 0.0)
    ));
    let pose = sample(&clip, &skeleton.bone_ids[0], 2.0 * FRAME_TIME);
    assert!(near(pose.translation, Vec3::new(2.0, 90.0, 0.0)));
    assert!(
        skeleton.inverse_bindposes[1]
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, -100.0, 0.0)), 1e-3)
    );
}

#[test]
fn a_frame_range_trims_the_clip_and_rebases_its_times() {
    let (skeleton, clip) = import_ok("trim.bvh", TWO_JOINTS, "frame_range = [1, 3]");
    let spine = skeleton.bone_ids[1];
    let first = sample(&clip, &spine, 0.0);
    let last = sample(&clip, &spine, FRAME_TIME);
    assert!(same_rotation(
        first.rotation,
        Quat::from_rotation_x(deg(5.0))
    ));
    assert!(same_rotation(
        last.rotation,
        Quat::from_rotation_x(deg(10.0))
    ));
    assert!((clip.duration() - FRAME_TIME).abs() < 1e-6);
}

#[test]
fn a_bone_id_is_the_hash_of_the_joint_name_path() {
    let (skeleton, clip) = import_ok("ids.bvh", TWO_JOINTS, "");
    assert_eq!(skeleton.bone_ids[0], path_id("Hips"));
    assert_eq!(skeleton.bone_ids[1], path_id("Hips/Spine"));
    for id in skeleton.bone_ids.iter() {
        assert!(clip.get_channels(id).is_some());
    }
    assert_eq!(clip.target_ids().count(), 2);
}

#[test]
fn invalid_settings_are_rejected() {
    for scale in ["0", "-1.0", "nan", "inf"] {
        assert_malformed_with(
            "scale.bvh",
            TWO_JOINTS,
            &format!("scale = {scale}"),
            "scale",
        );
    }
    assert_malformed_with("unknown.bvh", TWO_JOINTS, "scal = 1.0", "scal");
    for range in ["[2, 2]", "[2, 1]", "[0, 4]", "[3, 5]"] {
        assert_malformed_with(
            "range.bvh",
            TWO_JOINTS,
            &format!("frame_range = {range}"),
            "frame_range",
        );
    }
}

#[test]
fn fewer_than_two_frames_are_rejected() {
    let one = TWO_JOINTS
        .replace("Frames: 3", "Frames: 1")
        .replace("1 90 0 10 0 0 5 0 0\n2 90 0 20 0 0 10 0 0\n", "");
    assert_malformed_with("one.bvh", &one, "", "two frames");
    assert_malformed_with(
        "trimmed.bvh",
        TWO_JOINTS,
        "frame_range = [1, 2]",
        "two frames",
    );
}

fn import_scene(name: &str, text: &str) -> Scene {
    let outputs = run(name, text, "").expect("the import should succeed");
    let entry = outputs
        .sub_assets
        .iter()
        .find(|asset| asset.name == "scene")
        .expect("missing sub-asset scene");
    bincode::deserialize(&entry.bytes).unwrap()
}

fn decode<T: Reflect + Default>(node: &SceneNode) -> Option<T> {
    let component = node
        .components
        .iter()
        .find(|component| component.type_name == std::any::type_name::<T>())?;
    let mut value = T::default();
    let warnings = apply_value(&mut value, &component.value);
    assert!(warnings.is_empty(), "{warnings:?}");
    Some(value)
}

#[test]
fn the_scene_has_one_node_per_joint_with_parents_first() {
    let scene = import_scene("scene.bvh", TWO_JOINTS);
    let names: Vec<&str> = scene.nodes.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(names, ["Hips", "Spine"]);
    assert_eq!(scene.nodes[0].children, [1]);
    assert!(scene.nodes[1].children.is_empty());
}

#[test]
fn each_scene_node_carries_the_rest_pose_transform() {
    let scene = import_scene("scene_pose.bvh", TWO_JOINTS);
    let hips: Transform = decode(&scene.nodes[0]).unwrap();
    let spine: Transform = decode(&scene.nodes[1]).unwrap();
    assert!(near(hips.translation, Vec3::new(0.0, 0.9, 0.0)));
    assert!(near(spine.translation, Vec3::new(0.0, 0.1, 0.0)));
    assert_eq!(spine.rotation, Quat::IDENTITY);
    assert_eq!(spine.scale, Vec3::ONE);
}

#[test]
fn the_root_node_carries_the_rooted_skeleton_component() {
    let scene = import_scene("scene_skeleton.bvh", TWO_JOINTS);
    let skeleton: SkeletonComponent = decode(&scene.nodes[0]).unwrap();
    assert_eq!(
        skeleton.skeleton.id(),
        AssetId::from_path("scene_skeleton.bvh#skeleton/0")
    );
    assert_eq!(
        skeleton.bones,
        [SceneEntityRef::Index(0), SceneEntityRef::Index(1)]
    );
    assert_eq!(skeleton.root, Some(SceneEntityRef::Index(0)));
    assert!(decode::<SkeletonComponent>(&scene.nodes[1]).is_none());
    assert_eq!(
        scene.referenced_assets,
        [AssetId::from_path("scene_skeleton.bvh#skeleton/0")]
    );
}

#[test]
fn the_clip_is_named_after_the_source_file_stem() {
    let outputs = run("Walk Cycle.bvh", TWO_JOINTS, "").expect("the import should succeed");
    assert!(
        outputs
            .sub_assets
            .iter()
            .any(|asset| asset.name == "animation/Walk Cycle"),
        "sub-assets: {:?}",
        outputs
            .sub_assets
            .iter()
            .map(|asset| asset.name.as_str())
            .collect::<Vec<_>>()
    );
}
