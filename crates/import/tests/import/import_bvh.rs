use std::collections::BTreeMap;
use std::path::Path;

use concerto_animation::clip::AnimationClip;
use concerto_foundation::assets::content::{decode_content_asset, resave_content_asset};
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_import::config::ContentConfig;
use concerto_import::import_source_into;
use concerto_import::metadata::{sidecar_path, SourceMetadata};
use concerto_reflect::ReflectedValue;

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
fn a_bvh_imports_its_outputs_and_a_sidecar_whose_settings_a_re_import_applies_under_the_same_ids() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let first = import(&source(scratch.path()), project.path()).expect("import succeeds");

    for name in ["skeleton_0", "animation_walk", "scene"] {
        let address = format!("content/walk/{name}.gasset");
        assert!(
            first.assets.iter().any(|asset| asset.address == address),
            "expected {address} among {:?}",
            first.assets
        );
        assert!(project.path().join(&address).exists(), "{address} on disk");
    }
    assert_eq!(first.assets.len(), 3);
    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let mut metadata = SourceMetadata::load(&sidecar).expect("sidecar written");
    assert_eq!(metadata.importer, "bvh");
    for key in ["skeleton/0", "animation/walk", "scene"] {
        assert!(metadata.outputs.contains_key(key), "{key} in sidecar");
    }

    let skeleton = "content/walk/skeleton_0.gasset";
    let clip = "content/walk/animation_walk.gasset";
    let skeleton_before = read(project.path(), skeleton);
    let clip_before = read(project.path(), clip);
    metadata
        .settings
        .insert("scale".to_string(), toml::Value::Float(0.5));
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();

    let second = import(&source(scratch.path()), project.path()).expect("re-import");
    assert_ne!(read(project.path(), skeleton), skeleton_before);
    assert_ne!(read(project.path(), clip), clip_before);
    let ids = |imported: &concerto_import::ImportedSource| {
        imported
            .assets
            .iter()
            .map(|asset| (asset.address.clone(), asset.id))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        ids(&second),
        ids(&first),
        "asset identities survive a re-import"
    );
    let reread = SourceMetadata::load(&sidecar).unwrap();
    assert_eq!(reread.settings["scale"].as_float(), Some(0.5));
}

#[derive(Debug, PartialEq)]
struct StoredEvent {
    time: f32,
    type_name: String,
    value: ReflectedValue,
}

fn stored_events(clip: &AnimationClip) -> Vec<StoredEvent> {
    clip.events()
        .iter()
        .map(|key| StoredEvent {
            time: key.time,
            type_name: key.event.type_name().to_string(),
            value: key.event.value().unwrap().into_owned(),
        })
        .collect()
}

#[test]
fn a_re_import_overwrites_a_resaved_clip_and_rebuilds_its_events_from_the_sidecar() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    import(&source(scratch.path()), project.path()).expect("first import");
    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let mut metadata = SourceMetadata::load(&sidecar).unwrap();
    metadata.settings.insert(
        "events".to_string(),
        toml::Value::Table(toml::from_str("walk = { step = [0.03] }").unwrap()),
    );
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();
    import(&source(scratch.path()), project.path()).expect("import with events");
    let address = "content/walk/animation_walk.gasset";
    let (_, imported) = decode_content_asset::<AnimationClip>(&read(project.path(), address))
        .expect("the clip decodes");
    let imported_events = stored_events(&imported);
    assert_eq!(imported_events.len(), 1);

    let mut edited = imported;
    edited
        .add_animation_events(&BTreeMap::from([("edited".to_string(), vec![0.05])]))
        .unwrap();
    resave_content_asset(&edited, project.path(), address, &mut AssetRegistry::new())
        .expect("resave");
    let (_, resaved) = decode_content_asset::<AnimationClip>(&read(project.path(), address))
        .expect("the resaved clip decodes");
    assert_eq!(resaved.events().len(), 2);

    import(&source(scratch.path()), project.path()).expect("re-import");

    let (header, reimported) =
        decode_content_asset::<AnimationClip>(&read(project.path(), address))
            .expect("the re-imported clip decodes");
    assert!(header.provenance.is_some());
    assert_eq!(stored_events(&reimported), imported_events);
}
