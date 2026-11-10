use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use concerto_animation::clip::{Additive, AdditiveBase, AnimationClip, ClipRange};
use concerto_animation::event::AnimationEvent;
use concerto_asset_import::{ImportContext, ImportError, ImportOutputs, Importer};
use concerto_bvh::bvh_importer::BvhImporter;
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
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

fn run(name: &str, text: &str, settings: &str) -> Result<ImportOutputs, ImportError> {
    let path = write_source(name, text);
    let table: toml::Table = settings.parse().unwrap();
    let mut ctx = ImportContext::new(PathBuf::from(name)).with_settings(table);
    BvhImporter.import(&path, &mut ctx)?;
    Ok(ctx.into_parts())
}

struct Imported {
    skeleton: Skeleton,
    clip: AnimationClip,
}

fn import_ok(name: &str, text: &str, settings: &str) -> Imported {
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
    Imported {
        skeleton: bincode::deserialize(&decode("skeleton/0")).unwrap(),
        clip: bincode::deserialize(&decode(&format!("animation/{stem}"))).unwrap(),
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

#[derive(Debug, PartialEq)]
struct NamedKey {
    time: f32,
    name: String,
}

fn named_key(time: f32, name: &str) -> NamedKey {
    NamedKey {
        time,
        name: name.to_string(),
    }
}

fn event_keys(clip: &AnimationClip) -> Vec<NamedKey> {
    clip.events()
        .iter()
        .map(|key| {
            assert_eq!(
                key.event.type_name(),
                std::any::type_name::<AnimationEvent>()
            );
            let mut event = AnimationEvent::default();
            let warnings = apply_value(&mut event, &key.event.value().unwrap());
            assert!(warnings.is_empty(), "{warnings:?}");
            named_key(key.time, event.name.as_str())
        })
        .collect()
}

const BASE_ID: &str = "d6e76e64-a888-49cd-bc16-61f0566f36ab";

struct Case {
    case: &'static str,
    source: String,
    settings: String,
    refusal: Option<&'static str>,
}

fn case(case: &'static str, source: String, settings: &str, refusal: Option<&'static str>) -> Case {
    Case {
        case,
        source,
        settings: settings.to_string(),
        refusal,
    }
}

fn source(case: &'static str, source: String, refusal: &'static str) -> Case {
    self::case(case, source, "", Some(refusal))
}

fn setting(case: &'static str, settings: String, refusal: &'static str) -> Case {
    self::case(case, TWO_JOINTS.to_string(), &settings, Some(refusal))
}

fn deeply_nested() -> String {
    let mut text = String::from("HIERARCHY\nROOT J0\n{\nOFFSET 0 0 0\nCHANNELS 0\n");
    for depth in 1..1000 {
        text.push_str(&format!("JOINT J{depth}\n{{\nOFFSET 0 0 0\nCHANNELS 0\n"));
    }
    text.push_str(&"}\n".repeat(1000));
    text.push_str("MOTION\nFrames: 2\nFrame Time: 0.0333333\n\n\n");
    text
}

fn additive(range: &str) -> String {
    format!("[additive.events]\nbase = \"{BASE_ID}\"\n{range}")
}

#[test]
fn a_source_and_its_settings_import_or_are_refused_naming_the_problem() {
    let mut cases = vec![
        case(
            "CRLF line endings",
            TWO_JOINTS.replace('\n', "\r\n"),
            "",
            None,
        ),
        case(
            "a brace on the joint's line",
            TWO_JOINTS.replace("ROOT Hips\n{", "ROOT Hips {"),
            "",
            None,
        ),
        source(
            "a slash in a joint name",
            TWO_JOINTS.replace("JOINT Spine", "JOINT Spine/Upper"),
            "'/'",
        ),
        source(
            "an unknown channel",
            TWO_JOINTS.replace(
                "Zrotation Yrotation Xrotation",
                "Zrotation Yrotation Wrotation",
            ),
            "Wrotation",
        ),
        source(
            "a frame line with too few values",
            TWO_JOINTS.replace("1 90 0 10 0 0 5 0 0", "1 90 0 10 0 0 5 0"),
            "line 20",
        ),
        source(
            "fewer frame lines than declared",
            TWO_JOINTS.replace("Frames: 3", "Frames: 4"),
            "Frames",
        ),
        source(
            "a huge frame count",
            TWO_JOINTS.replace("Frames: 3", "Frames: 99999999999999"),
            "Frames",
        ),
        source(
            "a huge channel count",
            TWO_JOINTS.replace("CHANNELS 3", "CHANNELS 99999999999999"),
            "channel",
        ),
        source("a deeply nested hierarchy", deeply_nested(), "nested"),
        source(
            "a missing closing brace",
            TWO_JOINTS.replacen("}\nMOTION", "MOTION", 1),
            "brace",
        ),
        source(
            "an extra closing brace",
            TWO_JOINTS.replacen("MOTION", "}\nMOTION", 1),
            "line",
        ),
        source(
            "duplicate sibling names",
            TWO_JOINTS.replacen(
                "\t}\n}\nMOTION",
                "\t}\n\tJOINT Spine\n\t{\n\t\tOFFSET 0 1 0\n\t\tCHANNELS 0\n\t}\n}\nMOTION",
                1,
            ),
            "Spine",
        ),
        source(
            "no root",
            "HIERARCHY\nMOTION\nFrames: 0\nFrame Time: 0.1\n".to_string(),
            "ROOT",
        ),
        source(
            "two roots",
            TWO_JOINTS.replacen(
                "MOTION",
                "ROOT Other\n{\n\tOFFSET 0 0 0\n\tCHANNELS 0\n}\nMOTION",
                1,
            ),
            "ROOT",
        ),
        source(
            "no hierarchy header",
            "ROOT Hips\n".to_string(),
            "HIERARCHY",
        ),
        source(
            "one frame",
            TWO_JOINTS
                .replace("Frames: 3", "Frames: 1")
                .replace("1 90 0 10 0 0 5 0 0\n2 90 0 20 0 0 10 0 0\n", ""),
            "two frames",
        ),
        setting(
            "a frame range of one frame",
            "frame_range = [1, 2]".to_string(),
            "two frames",
        ),
        setting("an unknown key", "scal = 1.0".to_string(), "scal"),
        setting(
            "events for another clip",
            "[events.run]\nstep = [0.0]\n".to_string(),
            "run",
        ),
        setting(
            "an event past the trimmed clip",
            "frame_range = [1, 3]\n[events.events]\nstep = [0.05]\n".to_string(),
            "step",
        ),
        setting(
            "an additive table for another clip",
            format!("[additive.run]\nbase = \"{BASE_ID}\"\n"),
            "run",
        ),
        setting(
            "an additive base that is not an id",
            "[additive.events]\nbase = \"not-an-id\"\n".to_string(),
            "UUID",
        ),
        setting("an additive weight", additive("weight = 1.0\n"), "weight"),
        setting(
            "an additive range with a step",
            additive("range = { start = 0.0, end = 0.05, step = 0.01 }\n"),
            "step",
        ),
    ];
    for value in ["0", "-0.03", "NaN", "inf", "abc"] {
        cases.push(source(
            "a frame time that is not positive",
            TWO_JOINTS.replace("0.0333333", value),
            "Frame Time",
        ));
    }
    for scale in ["0", "-1.0", "nan", "inf"] {
        cases.push(setting(
            "a scale that is not positive",
            format!("scale = {scale}"),
            "scale",
        ));
    }
    for range in ["[2, 2]", "[2, 1]", "[0, 4]", "[3, 5]"] {
        cases.push(setting(
            "a frame range outside the frames",
            format!("frame_range = {range}"),
            "frame_range",
        ));
    }
    for time in ["-0.01", "nan", "inf", "0.07"] {
        cases.push(setting(
            "an event outside the clip",
            format!("[events.events]\nstep = [{time}]\n"),
            "step",
        ));
    }
    for range in [
        "[0.0, 0.07]",
        "[0.02, 0.02]",
        "[0.05, 0.01]",
        "[-0.01, 0.05]",
        "[nan, 0.05]",
        "[0.0, inf]",
    ] {
        cases.push(setting(
            "an additive range outside the clip",
            additive(&format!("range = {range}\n")),
            "range",
        ));
    }

    for case in &cases {
        let result = run("events.bvh", &case.source, &case.settings);
        match (result, case.refusal) {
            (Ok(_), None) => {}
            (Err(ImportError::MalformedSource { message, .. }), Some(expected)) => assert!(
                message.contains(expected),
                "{}: expected {expected:?} in {message:?}",
                case.case
            ),
            (result, _) => panic!("{}: got {:?}", case.case, result.err()),
        }
    }
}

#[test]
fn the_skeleton_holds_the_hierarchy_rest_pose_inverse_bind_poses_and_path_ids() {
    let Imported { skeleton, clip } = import_ok("skeleton.bvh", TWO_JOINTS, "");
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
    assert_eq!(skeleton.bone_ids[0], path_id("Hips"));
    assert_eq!(skeleton.bone_ids[1], path_id("Hips/Spine"));
    for id in skeleton.bone_ids.iter() {
        assert!(clip.get_channels(id).is_some());
    }
    assert_eq!(clip.target_ids().count(), 2);

    let Imported { skeleton, clip } = import_ok("scaled.bvh", TWO_JOINTS, "scale = 1.0");
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
fn the_clip_gives_the_authored_pose_at_each_frame() {
    let Imported { skeleton, clip } = import_ok("sample.bvh", TWO_JOINTS, "");
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
    assert_eq!(clip.frame_rate(), Some(30.0));
    assert_eq!(clip.additive(), &Additive::None);
}

const ORDERED: &str = "HIERARCHY
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

const PARTIAL: &str = "HIERARCHY
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

struct Channels {
    case: &'static str,
    source: &'static str,
    settings: &'static str,
    bone: usize,
    time: f32,
    translation: Option<Vec3>,
    rotation: Quat,
}

#[test]
fn channels_compose_in_the_order_they_are_listed_over_the_rest_offset() {
    let cases = [
        Channels {
            case: "ZYX on the root",
            source: ORDERED,
            settings: "",
            bone: 0,
            time: 0.0,
            translation: None,
            rotation: Quat::from_rotation_z(deg(30.0))
                * Quat::from_rotation_y(deg(40.0))
                * Quat::from_rotation_x(deg(50.0)),
        },
        Channels {
            case: "XYZ on the child",
            source: ORDERED,
            settings: "",
            bone: 1,
            time: 0.0,
            translation: None,
            rotation: Quat::from_rotation_x(deg(10.0))
                * Quat::from_rotation_y(deg(20.0))
                * Quat::from_rotation_z(deg(30.0)),
        },
        Channels {
            case: "X and Z positions keep the rest Y",
            source: PARTIAL,
            settings: "scale = 0.1",
            bone: 0,
            time: 0.0,
            translation: Some(Vec3::new(1.0, 20.0, 3.0)),
            rotation: Quat::IDENTITY,
        },
        Channels {
            case: "X and Z positions on the second frame",
            source: PARTIAL,
            settings: "scale = 0.1",
            bone: 0,
            time: 1.0,
            translation: Some(Vec3::new(2.0, 20.0, 4.0)),
            rotation: Quat::IDENTITY,
        },
    ];
    for case in &cases {
        let Imported { skeleton, clip } = import_ok("channels.bvh", case.source, case.settings);
        let pose = sample(&clip, &skeleton.bone_ids[case.bone], case.time);
        assert!(same_rotation(pose.rotation, case.rotation), "{}", case.case);
        if let Some(translation) = case.translation {
            assert!(near(pose.translation, translation), "{}", case.case);
        }
    }
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
    let Imported { skeleton, clip } = import_ok("hemisphere.bvh", text, "");
    let id = skeleton.bone_ids[0];
    let keys: Vec<Quat> = (0..3)
        .map(|i| sample(&clip, &id, i as f32).rotation)
        .collect();
    assert!(keys[0].dot(keys[1]) >= 0.0);
    assert!(keys[1].dot(keys[2]) >= 0.0);
    assert!(same_rotation(keys[1], Quat::from_rotation_z(deg(359.0))));
}

#[test]
fn a_frame_range_trims_the_clip_and_rebases_its_times_and_events() {
    let settings = "frame_range = [1, 3]\n[events.trim]\nland = [0.0, 0.03]\n";
    let Imported { skeleton, clip } = import_ok("trim.bvh", TWO_JOINTS, settings);
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
    assert_eq!(clip.frame_rate(), Some(30.0));
    assert_eq!(
        event_keys(&clip),
        [named_key(0.0, "land"), named_key(0.03, "land")]
    );
}

#[test]
fn the_clip_is_named_after_the_sanitised_file_stem_and_takes_the_events_under_that_name() {
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

    let settings =
        "[events.\"Walk_Cycle\"]\nfootstep_right = [0.05, 0.01, 0.05]\nfootstep_left = [0.02]\n";
    let outputs = run("Walk#Cycle.bvh", TWO_JOINTS, settings).expect("the import should succeed");
    let clip: AnimationClip = bincode::deserialize(
        &outputs
            .sub_assets
            .iter()
            .find(|asset| asset.name == "animation/Walk_Cycle")
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert_eq!(
        event_keys(&clip),
        [
            named_key(0.01, "footstep_right"),
            named_key(0.02, "footstep_left"),
            named_key(0.05, "footstep_right"),
        ]
    );
}

#[test]
fn an_additive_table_makes_the_clip_additive_against_its_base() {
    let settings = format!(
        "[additive.cold_walk]\nbase = \"{BASE_ID}\"\nbase_range = [0.5, 1.5]\nrange = {{ start = 0.01, end = 0.05 }}\n"
    );
    let Imported { skeleton, clip } = import_ok("cold_walk.bvh", TWO_JOINTS, &settings);
    assert_eq!(
        clip.additive(),
        &Additive::LocalSpace(
            AdditiveBase::new(AssetHandle::weak(
                AssetId::from_simple_hex(BASE_ID).unwrap()
            ))
            .with_base_range(ClipRange::new(0.5, 1.5))
            .with_range(ClipRange::new(0.01, 0.05))
            .with_root(skeleton.bone_ids[0])
        )
    );
}

#[test]
fn the_scene_has_a_node_per_joint_with_its_rest_pose_and_the_root_holds_the_skeleton() {
    let scene = import_scene("scene.bvh", TWO_JOINTS);
    let names: Vec<&str> = scene.nodes.iter().map(|node| node.name.as_str()).collect();
    assert_eq!(names, ["Hips", "Spine"]);
    assert_eq!(scene.nodes[0].children, [1]);
    assert!(scene.nodes[1].children.is_empty());

    let hips: Transform = decode(&scene.nodes[0]).unwrap();
    let spine: Transform = decode(&scene.nodes[1]).unwrap();
    assert!(near(hips.translation, Vec3::new(0.0, 0.9, 0.0)));
    assert!(near(spine.translation, Vec3::new(0.0, 0.1, 0.0)));
    assert_eq!(spine.rotation, Quat::IDENTITY);
    assert_eq!(spine.scale, Vec3::ONE);

    let skeleton: SkeletonComponent = decode(&scene.nodes[0]).unwrap();
    assert_eq!(
        skeleton.skeleton.id(),
        AssetId::from_path("scene.bvh#skeleton/0")
    );
    assert_eq!(
        skeleton.bones,
        [SceneEntityRef::Index(0), SceneEntityRef::Index(1)]
    );
    assert_eq!(skeleton.root, Some(SceneEntityRef::Index(0)));
    assert!(decode::<SkeletonComponent>(&scene.nodes[1]).is_none());
    assert_eq!(
        scene.referenced_assets,
        [AssetId::from_path("scene.bvh#skeleton/0")]
    );
}
