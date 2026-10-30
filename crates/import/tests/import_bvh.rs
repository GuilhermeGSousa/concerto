use std::path::Path;

use concerto_import::config::ContentConfig;
use concerto_import::metadata::{sidecar_path, SourceMetadata};
use concerto_import::{import_source_into, supported_extension};

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

fn source(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("walk.bvh");
    std::fs::write(&path, WALK).unwrap();
    path
}

fn import(source: &Path, project: &Path) -> anyhow::Result<concerto_import::ImportedSource> {
    import_source_into(
        source,
        "assets/walk.bvh",
        project,
        &ContentConfig::default(),
    )
}

fn read(project: &Path, address: &str) -> Vec<u8> {
    std::fs::read(project.join(address)).unwrap_or_else(|_| panic!("{address} on disk"))
}

#[test]
fn the_bvh_extension_is_supported() {
    assert!(supported_extension("bvh"));
    assert!(supported_extension("BVH"));
}

#[test]
fn importing_a_bvh_writes_skeleton_clip_scene_and_a_sidecar() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let imported = import(&source(scratch.path()), project.path()).expect("import succeeds");

    for name in ["skeleton_0", "animation_walk", "scene"] {
        let address = format!("content/walk/{name}.gasset");
        assert!(
            imported.assets.iter().any(|asset| asset.address == address),
            "expected {address} among {:?}",
            imported.assets
        );
        assert!(project.path().join(&address).exists(), "{address} on disk");
    }
    assert_eq!(imported.assets.len(), 3);

    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let metadata = SourceMetadata::load(&sidecar).expect("sidecar written");
    assert_eq!(metadata.importer, "bvh");
    for key in ["skeleton/0", "animation/walk", "scene"] {
        assert!(metadata.outputs.contains_key(key), "{key} in sidecar");
    }
}

#[test]
fn re_importing_after_a_scale_edit_changes_the_outputs_and_keeps_their_ids() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let first = import(&source(scratch.path()), project.path()).expect("first import");
    let skeleton = "content/walk/skeleton_0.gasset";
    let clip = "content/walk/animation_walk.gasset";
    let before = (read(project.path(), skeleton), read(project.path(), clip));

    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let mut metadata = SourceMetadata::load(&sidecar).unwrap();
    metadata
        .settings
        .insert("scale".to_string(), toml::Value::Float(0.5));
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();

    let second = import(&source(scratch.path()), project.path()).expect("re-import");
    let after = (read(project.path(), skeleton), read(project.path(), clip));
    assert_ne!(before.0, after.0, "skeleton must reflect the new scale");
    assert_ne!(before.1, after.1, "clip must reflect the new scale");

    let mut ids_first: Vec<_> = first
        .assets
        .iter()
        .map(|a| (a.address.clone(), a.id))
        .collect();
    let mut ids_second: Vec<_> = second
        .assets
        .iter()
        .map(|a| (a.address.clone(), a.id))
        .collect();
    ids_first.sort();
    ids_second.sort();
    assert_eq!(
        ids_first, ids_second,
        "asset identities survive a re-import"
    );

    let reread = SourceMetadata::load(&sidecar).unwrap();
    assert_eq!(reread.settings["scale"].as_float(), Some(0.5));
}

#[test]
fn an_invalid_scale_in_the_sidecar_fails_the_import_and_writes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("assets")).unwrap();
    let sidecar = project.path().join("assets/walk.bvh.import.toml");
    std::fs::write(
        &sidecar,
        "version = 1\nsource_id = \"00000000-0000-0000-0000-000000000001\"\nimporter = \"bvh\"\n\n[settings]\nscale = -1.0\n",
    )
    .unwrap();
    let error = import(&source(scratch.path()), project.path()).expect_err("negative scale fails");
    assert!(format!("{error:?}").contains("scale"), "{error:?}");
    assert!(!project.path().join("content").exists());
}
