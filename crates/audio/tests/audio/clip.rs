use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
};
use concerto_asset_import::{EmittedSubAsset, ImportContext, ImportError, Importer, NoSettings};
use concerto_audio::{AudioBackend, AudioClip, AudioImporter, AudioPlugin};
use concerto_foundation::assets::{
    Asset, AssetId, ContentAssetRoot,
    asset_server::AssetServer,
    asset_store::AssetStore,
    content::{ContentAssetHeader, check_encoding, write_content_asset},
    registry::AssetRegistry,
};

pub(crate) fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("failed to read fixture {name}: {error}"))
}

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

pub(crate) fn import(
    project: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<EmittedSubAsset, ImportError> {
    let relative = PathBuf::from("assets").join(name);
    let source = project.join(&relative);
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, bytes).unwrap();
    let mut ctx = ImportContext::new(relative);
    AudioImporter.import(&source, &NoSettings::default(), None, &mut ctx)?;
    let outputs = ctx.into_parts();
    let [clip] = outputs.sub_assets.as_slice() else {
        panic!(
            "{name} imports to one clip, got {}",
            outputs.sub_assets.len()
        );
    };
    assert_eq!(clip.name, "main");
    assert_eq!(clip.type_name, AudioClip::name());
    Ok(clip.clone())
}

pub(crate) fn write_content(project: &Path, name: &str, clip: &EmittedSubAsset) {
    let path = project.join(format!("content/{name}/main.gasset"));
    let header = ContentAssetHeader {
        asset_id: clip.asset_id,
        references: clip.references.clone(),
        kind: clip.type_name.to_string(),
        kind_version: clip.kind_version,
        provenance: None,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, &clip.bytes).unwrap()).unwrap();
}

#[derive(Debug, PartialEq)]
struct Loaded {
    milliseconds: u128,
    sample_rate: u32,
}

fn load(project: &Path, ids: &[AssetId]) -> Vec<Option<Loaded>> {
    let mut app = App::new();
    app.insert_resource(AudioBackend::Mock);
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AudioPlugin);
    app.finish_plugin_build();
    app.update();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.publish_content_source(
        ContentAssetRoot::Directory(project.to_path_buf()),
        AssetRegistry::from_content_tree(project, "content", "gasset").unwrap(),
    );
    let handles: Vec<_> = ids.iter().map(|id| server.load::<AudioClip>(*id)).collect();
    let started = Instant::now();
    while server.is_loading() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "load timed out"
        );
        thread::sleep(Duration::from_millis(1));
        app.update();
    }
    app.update();
    let store = app.get_resource::<AssetStore<AudioClip>>().unwrap();
    handles
        .iter()
        .map(|handle| {
            store.get(handle).map(|clip| Loaded {
                milliseconds: clip.duration().as_millis(),
                sample_rate: clip.sample_rate(),
            })
        })
        .collect()
}

#[test]
fn audio_clip_encoding_matches_its_fixture() {
    let project = tempfile::tempdir().unwrap();
    let imported = import(project.path(), "tone.wav", &fixture("tone.wav")).unwrap();
    let clip: AudioClip = bincode::deserialize(&imported.bytes).unwrap();

    check_encoding(&clip, encoding_dir());
}

enum Outcome {
    Loads(Loaded),
    FailsImport,
}

struct Case {
    source: &'static str,
    bytes: Vec<u8>,
    outcome: Outcome,
}

#[test]
fn an_ogg_or_wav_source_imports_to_a_clip_that_loads_with_its_duration_and_sample_rate() {
    let cases = [
        Case {
            source: "tone.wav",
            bytes: fixture("tone.wav"),
            outcome: Outcome::Loads(Loaded {
                milliseconds: 100,
                sample_rate: 8000,
            }),
        },
        Case {
            source: "tone.ogg",
            bytes: fixture("tone.ogg"),
            outcome: Outcome::Loads(Loaded {
                milliseconds: 100,
                sample_rate: 22050,
            }),
        },
        Case {
            source: "noise.ogg",
            bytes: b"not an ogg stream, only text".to_vec(),
            outcome: Outcome::FailsImport,
        },
    ];
    let project = tempfile::tempdir().unwrap();
    let mut expected = Vec::new();
    let mut ids = Vec::new();
    for case in cases {
        let imported = import(project.path(), case.source, &case.bytes);
        match case.outcome {
            Outcome::Loads(loaded) => {
                let clip = imported
                    .unwrap_or_else(|error| panic!("{} fails import: {error:?}", case.source));
                write_content(project.path(), case.source, &clip);
                ids.push(clip.asset_id);
                expected.push(Some(loaded));
            }
            Outcome::FailsImport => {
                let error = format!("{:?}", imported.expect_err(case.source));
                assert!(
                    error.contains(case.source),
                    "the error names {}: {error}",
                    case.source
                );
            }
        }
    }

    assert_eq!(load(project.path(), &ids), expected);
}
