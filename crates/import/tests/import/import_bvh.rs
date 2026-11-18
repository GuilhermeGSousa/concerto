use std::collections::BTreeMap;
use std::path::Path;

use concerto_animation::clip::{Additive, AdditiveBase, AnimationClip, ClipRange};
use concerto_animation::event::AnimationEvent;
use concerto_bvh::bvh_importer::{BvhSettings, FrameRange};
use concerto_foundation::assets::content::{
    decode_content_asset, read_asset_record, read_content_asset_header, resave_content_asset,
    ImportRecord,
};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::assets::registry::AssetRegistry;
use concerto_foundation::assets::AssetId;
use concerto_foundation::hash::Fnv1a;
use concerto_import::config::ContentConfig;
use concerto_import::{import_source_into, importer_for, ImportSettings};
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
    import_with(source, project, None)
}

fn import_with(
    source: &Path,
    project: &Path,
    settings: Option<BvhSettings>,
) -> anyhow::Result<concerto_import::ImportedSource> {
    import_source_into(
        source,
        "assets/walk.bvh",
        project,
        &ContentConfig::default(),
        ImportSettings {
            settings: settings.map(|settings| Box::new(settings) as _),
            asset_settings: None,
        },
    )
}

fn record(project: &Path) -> ImportRecord {
    read_content_asset_header(&project.join("content/walk/scene.gasset"))
        .unwrap()
        .import
        .expect("the scene is the anchor and holds the import record")
}

fn recorded_settings(record: &ImportRecord) -> BvhSettings {
    let importer = importer_for("bvh").unwrap();
    assert_eq!(record.importer, importer.name());
    importer
        .settings_from_bytes(record.settings_version, &record.settings)
        .unwrap()
        .expect("the record is at the importer's settings version")
        .downcast_ref::<BvhSettings>()
        .unwrap()
        .clone()
}

fn read(project: &Path, address: &str) -> Vec<u8> {
    std::fs::read(project.join(address)).unwrap_or_else(|_| panic!("{address} on disk"))
}

#[test]
fn a_bvh_imports_its_outputs_and_a_record_whose_settings_a_re_import_keeps_under_the_same_ids() {
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
    let recorded = record(project.path());
    assert_eq!(recorded.source.path, "assets/walk.bvh");
    let mut hash = Fnv1a::new();
    hash.raw(WALK.as_bytes());
    assert_eq!(recorded.source.hash, hash.finish());
    assert_eq!(recorded_settings(&recorded), BvhSettings::default());

    let skeleton = first
        .assets
        .iter()
        .find(|asset| asset.embedded_in.is_some())
        .unwrap();
    let clip = "content/walk/animation_walk.gasset";
    let skeleton_before = read_asset_record(project.path(), skeleton).unwrap();
    let clip_before = read(project.path(), clip);
    let half = BvhSettings {
        scale: 0.5,
        ..BvhSettings::default()
    };

    let second = import_with(&source(scratch.path()), project.path(), Some(half.clone()))
        .expect("re-import");
    let skeleton_after = read_asset_record(project.path(), skeleton).unwrap();
    assert_ne!(skeleton_after, skeleton_before);
    assert_ne!(read(project.path(), clip), clip_before);
    assert_eq!(recorded_settings(&record(project.path())), half);

    let third = import(&source(scratch.path()), project.path()).expect("re-import");
    assert_eq!(
        read_asset_record(project.path(), skeleton).unwrap(),
        skeleton_after,
        "a re-import with no settings keeps the recorded ones"
    );
    assert_eq!(recorded_settings(&record(project.path())), half);
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
    assert_eq!(ids(&third), ids(&first));
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

    import_with(
        &source(scratch.path()),
        project.path(),
        Some(BvhSettings {
            frame_range: Some(FrameRange { start: 0, end: 2 }),
            ..BvhSettings::default()
        }),
    )
    .expect("re-import with a shorter range");

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
