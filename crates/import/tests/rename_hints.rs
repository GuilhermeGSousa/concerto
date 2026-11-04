use std::collections::BTreeMap;
use std::path::Path;

use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, write_content_asset,
};
use concerto_foundation::assets::AssetId;
use concerto_import::config::ContentConfig;
use concerto_import::metadata::{sidecar_path, SourceMetadata};
use concerto_import::{import_source_into, ImportedSource};

const WALK: &str = "HIERARCHY
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

fn import(source: &Path, destination: &str, project: &Path) -> anyhow::Result<ImportedSource> {
    import_source_into(source, destination, project, &ContentConfig::default())
}

fn ids_by_name(imported: &ImportedSource) -> BTreeMap<String, AssetId> {
    imported
        .assets
        .iter()
        .map(|asset| {
            (
                asset.provenance.as_ref().unwrap().sub_asset.clone(),
                asset.id,
            )
        })
        .collect()
}

fn rename_outputs(project: &Path, destination: &str, renames: &[(&str, &str)]) {
    let sidecar = sidecar_path(&project.join(destination));
    let mut metadata = SourceMetadata::load(&sidecar).unwrap();
    for (from, to) in renames {
        let output = metadata.outputs.remove(*from).expect("output exists");
        let path = walk(project)
            .into_iter()
            .find(|path| read_content_asset_header(path).unwrap().asset_id == output.asset_id)
            .expect("file of the output");
        let file = std::fs::read(&path).unwrap();
        let (mut header, payload) = read_content_asset(&file).unwrap();
        header.provenance.as_mut().unwrap().sub_asset = (*to).to_string();
        std::fs::write(&path, write_content_asset(&header, payload).unwrap()).unwrap();
        metadata.outputs.insert((*to).to_string(), output);
    }
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();
}

fn walk(project: &Path) -> Vec<std::path::PathBuf> {
    fn visit(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, files);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "gasset")
            {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    visit(&project.join("content"), &mut files);
    files
}

fn gltf_with_clip_names(names: &[Option<&str>]) -> String {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../gltf/tests/fixtures/animations_named.gltf");
    let mut document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    let animations = document["animations"].as_array_mut().unwrap();
    assert_eq!(animations.len(), names.len());
    for (animation, name) in animations.iter_mut().zip(names) {
        let object = animation.as_object_mut().unwrap();
        match name {
            Some(name) => object.insert("name".into(), (*name).into()),
            None => object.remove("name"),
        };
    }
    serde_json::to_string(&document).unwrap()
}

#[test]
fn a_bvh_reimport_over_an_index_named_clip_keeps_its_id_and_file() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let source = scratch.path().join("walk.bvh");
    std::fs::write(&source, WALK).unwrap();
    let first = import(&source, "assets/walk.bvh", project.path()).expect("first import");
    let clip = first
        .assets
        .iter()
        .find(|asset| asset.provenance.as_ref().unwrap().sub_asset == "animation/walk")
        .unwrap()
        .clone();
    rename_outputs(
        project.path(),
        "assets/walk.bvh",
        &[("animation/walk", "animation/0")],
    );

    let second = import(&source, "assets/walk.bvh", project.path()).expect("second import");

    let renamed = second
        .assets
        .iter()
        .find(|asset| asset.provenance.as_ref().unwrap().sub_asset == "animation/walk")
        .expect("clip re-emitted under its new name");
    assert_eq!(renamed.id, clip.id);
    assert_eq!(renamed.address, clip.address);
    assert!(
        second.removed.is_empty(),
        "nothing is pruned: {:?}",
        second.removed
    );
    let metadata =
        SourceMetadata::load(&sidecar_path(&project.path().join("assets/walk.bvh"))).unwrap();
    assert!(!metadata.outputs.contains_key("animation/0"));
    assert_eq!(metadata.outputs["animation/walk"].asset_id, clip.id);
}

#[test]
fn a_gltf_reimport_over_index_named_clips_keeps_their_ids() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let source = scratch.path().join("clips.gltf");
    std::fs::write(
        &source,
        gltf_with_clip_names(&[Some("Wiggle"), None, Some("Wiggle")]),
    )
    .unwrap();
    let first = import(&source, "assets/clips.gltf", project.path()).expect("first import");
    let before = ids_by_name(&first);
    rename_outputs(
        project.path(),
        "assets/clips.gltf",
        &[
            ("animation/Wiggle", "animation/0"),
            ("animation/Wiggle_2", "animation/2"),
        ],
    );

    let second = import(&source, "assets/clips.gltf", project.path()).expect("second import");

    let after = ids_by_name(&second);
    for name in ["animation/Wiggle", "animation/1", "animation/Wiggle_2"] {
        assert_eq!(after[name], before[name], "{name} keeps its id");
    }
    assert!(
        second.removed.is_empty(),
        "nothing is pruned: {:?}",
        second.removed
    );
}

#[test]
fn a_gltf_clip_named_like_another_clips_index_is_not_hinted() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let source = scratch.path().join("clips.gltf");
    std::fs::write(
        &source,
        gltf_with_clip_names(&[Some("Wiggle"), None, Some("Wiggle")]),
    )
    .unwrap();
    let first = import(&source, "assets/clips.gltf", project.path()).expect("first import");
    let before = ids_by_name(&first);
    rename_outputs(
        project.path(),
        "assets/clips.gltf",
        &[
            ("animation/Wiggle", "animation/0"),
            ("animation/Wiggle_2", "animation/2"),
        ],
    );

    std::fs::write(&source, gltf_with_clip_names(&[Some("1"), None, Some("0")])).unwrap();
    let second = import(&source, "assets/clips.gltf", project.path()).expect("second import");

    let after = ids_by_name(&second);
    let unique: std::collections::BTreeSet<_> = after.values().collect();
    assert_eq!(
        unique.len(),
        after.len(),
        "every output has its own id: {after:?}"
    );
    assert_eq!(after["animation/1"], before["animation/1"]);
    assert_eq!(after["animation/0"], before["animation/Wiggle"]);
    assert_eq!(second.removed, vec![before["animation/Wiggle_2"]]);
}
