use std::path::PathBuf;

use concerto_asset_import::{ImportContext, ImportError, Importer};
use concerto_bvh::bvh_importer::BvhImporter;

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
