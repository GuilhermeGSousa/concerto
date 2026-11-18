use std::collections::BTreeMap;
use std::path::Path;

use concerto_animation::clip::{Additive, AdditiveBase, AnimationClip, ClipRange};
use concerto_animation::event::AnimationEvent;
use concerto_foundation::assets::content::{
    decode_content_asset, read_asset_record, resave_content_asset,
};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_foundation::assets::AssetId;
use concerto_import::config::ContentConfig;
use concerto_import::import_source_into;
use concerto_import::metadata::{sidecar_path, SourceMetadata};
use concerto_mesh::Skeleton;
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
        Default::default(),
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

    let mut locations: Vec<String> = first.assets.iter().map(|asset| asset.location()).collect();
    locations.sort();
    assert_eq!(
        locations,
        [
            "content/walk/animation_walk.gasset",
            "content/walk/scene.gasset",
            "content/walk/scene.gasset#skeleton/0",
        ]
    );
    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let mut metadata = SourceMetadata::load(&sidecar).expect("sidecar written");

    let skeleton = first
        .assets
        .iter()
        .find(|asset| asset.embedded_in.is_some())
        .unwrap();
    let clip = "content/walk/animation_walk.gasset";
    let skeleton_before = read_asset_record(project.path(), skeleton).unwrap();
    let clip_before = read(project.path(), clip);
    metadata
        .settings
        .insert("scale".to_string(), toml::Value::Float(0.5));
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();

    let second = import(&source(scratch.path()), project.path()).expect("re-import");
    assert_ne!(
        read_asset_record(project.path(), skeleton).unwrap(),
        skeleton_before
    );
    assert_ne!(read(project.path(), clip), clip_before);
    let ids = |imported: &concerto_import::ImportedSource| {
        imported
            .assets
            .iter()
            .map(|asset| (asset.location(), asset.id))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        ids(&second),
        ids(&first),
        "asset identities survive a re-import"
    );
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

fn clip_at(project: &Path, address: &str) -> AnimationClip {
    decode_content_asset::<AnimationClip>(&read(project, address))
        .expect("the clip decodes")
        .1
}

#[test]
fn a_resaved_clip_keeps_its_events_and_additive_base_across_re_imports_fitted_to_a_shorter_clip() {
    let scratch = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let first = import(&source(scratch.path()), project.path()).expect("first import");
    let skeleton = first
        .assets
        .iter()
        .find(|asset| asset.embedded_in.is_some())
        .unwrap();
    let (_, skeleton) =
        decode_content_asset::<Skeleton>(&read_asset_record(project.path(), skeleton).unwrap())
            .expect("the skeleton decodes");
    let address = "content/walk/animation_walk.gasset";
    let mut edited = clip_at(project.path(), address);
    edited.add_event(0.01, AnimationEvent::default());
    edited.add_event(0.06, AnimationEvent::default());
    let base = AdditiveBase::new(AssetHandle::weak(AssetId::new()))
        .with_base_range(ClipRange::new(0.0, 1.0))
        .with_range(ClipRange::new(0.02, 0.06));
    edited.set_additive(Additive::LocalSpace(base.clone()));
    resave_content_asset(&edited, project.path(), address, &mut AssetRegistry::new())
        .expect("resave");
    let rooted = |range: ClipRange| {
        let mut base = base.clone().with_root(skeleton.bone_ids[0]);
        base.range = Some(range);
        Additive::LocalSpace(base)
    };

    import(&source(scratch.path()), project.path()).expect("re-import");

    let kept = clip_at(project.path(), address);
    assert_eq!(stored_events(&kept), stored_events(&edited));
    assert_eq!(kept.additive(), &rooted(ClipRange::new(0.02, 0.06)));

    let sidecar = sidecar_path(&project.path().join("assets/walk.bvh"));
    let mut metadata = SourceMetadata::load(&sidecar).unwrap();
    metadata.settings.insert(
        "frame_range".to_string(),
        toml::Value::Array(vec![toml::Value::Integer(0), toml::Value::Integer(2)]),
    );
    std::fs::write(&sidecar, toml::to_string_pretty(&metadata).unwrap()).unwrap();

    import(&source(scratch.path()), project.path()).expect("re-import with a shorter range");

    let fitted = clip_at(project.path(), address);
    let end = fitted.duration();
    assert!(end < 0.06, "{end}");
    let times: Vec<f32> = stored_events(&fitted)
        .iter()
        .map(|event| event.time)
        .collect();
    assert_eq!(times, [0.01, end]);
    assert_eq!(fitted.additive(), &rooted(ClipRange::new(0.02, end)));
}
