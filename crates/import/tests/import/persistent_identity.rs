//! End-to-end identity and ownership guarantees for the offline import workflow.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use concerto_bvh::bvh_importer::{BvhSettings, FrameRange};
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, write_content_asset, ContentAssetHeader,
    ImportProvenance, ImportRecord, SourceFile,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::AssetId;
use concerto_foundation::hash::Fnv1a;
use concerto_import::config::ContentConfig;
use concerto_import::{importer_for, reimport, ImportSettings};
use concerto_scene::scene::Scene;

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("import-identity-{:?}", AssetId::new()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn source(&self, relative: &str) -> PathBuf {
        let source = self.path(relative);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../gltf/tests/fixtures/triangle.gltf"),
            &source,
        )
        .unwrap();
        source
    }

    fn walk(&self, relative: &str) -> PathBuf {
        let source = self.path(relative);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, WALK).unwrap();
        source
    }

    fn try_import(&self, source: &Path) -> anyhow::Result<Vec<AssetEntry>> {
        let destination = source.strip_prefix(&self.0).unwrap().to_str().unwrap();
        concerto_import::import_source_into(
            source,
            destination,
            &self.0,
            &Default::default(),
            Default::default(),
        )
        .map(|imported| imported.assets)
    }

    fn import(&self, source: &Path) -> Vec<AssetEntry> {
        self.try_import(source).expect("import succeeds")
    }

    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).unwrap().into(),
                        std::fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(&self.0, &self.0, &mut files);
        files
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn output<'a>(assets: &'a [AssetEntry], sub_asset: &str) -> &'a str {
    &assets
        .iter()
        .find(|a| sub_asset_of(a) == sub_asset)
        .unwrap()
        .address
}

fn sub_asset_of(entry: &AssetEntry) -> &str {
    &entry
        .provenance
        .as_ref()
        .expect("an imported entry carries provenance")
        .sub_asset
}

fn rename_node(source: &Path, name: &str) {
    let mut gltf: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    gltf["nodes"][0]["name"] = name.into();
    std::fs::write(source, serde_json::to_vec(&gltf).unwrap()).unwrap();
}

#[test]
fn reimport_updates_a_moved_output_in_place_and_preserves_its_uuid() {
    let project = Project::new();
    let source = project.source("assets/triangle.gltf");
    let first = project.import(&source);
    let original = project.path(output(&first, "scene"));
    let id = read_content_asset_header(&original).unwrap().asset_id;
    let moved_address = "content/levels/renamed_scene.gasset";
    let moved = project.path(moved_address);
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&original, &moved).unwrap();
    rename_node(&source, "Updated triangle");

    let second = project.import(&source);

    assert_eq!(output(&second, "scene"), moved_address);
    assert!(
        !original.exists(),
        "reimport must not recreate the old location"
    );
    let raw = std::fs::read(&moved).unwrap();
    let (header, payload) = read_content_asset(&raw).unwrap();
    assert_eq!(header.asset_id, id);
    let scene: Scene = bincode::deserialize(payload).unwrap();
    assert_eq!(scene.nodes[0].name, "Updated triangle");
    assert_eq!(
        AssetRegistry::from_content_tree(&project.0, "content", "gasset")
            .unwrap()
            .get(id)
            .map(|entry| entry.address.as_str()),
        Some(moved_address)
    );
}

#[test]
fn same_stem_sources_in_different_directories_have_independent_outputs() {
    let project = Project::new();
    let hero = project.source("assets/characters/hero.gltf");
    let enemy = project.source("assets/enemies/hero.gltf");
    let first = project.import(&hero);
    let first_bytes: Vec<_> = first
        .iter()
        .map(|a| std::fs::read(project.path(&a.address)).unwrap())
        .collect();
    let second = project.import(&enemy);

    for (asset, before) in first.iter().zip(first_bytes) {
        assert!(asset.address.starts_with("content/characters/hero/"));
        assert_eq!(std::fs::read(project.path(&asset.address)).unwrap(), before);
        let counterpart = second
            .iter()
            .find(|a| sub_asset_of(a) == sub_asset_of(asset))
            .unwrap();
        assert!(counterpart.address.starts_with("content/enemies/hero/"));
        assert_ne!(asset.id, counterpart.id);
    }
}

struct Header {
    address: &'static str,
    source: &'static str,
    provenance: bool,
    import: Option<ImportRecord>,
}

fn write_header(project: &Project, header: Header) {
    let path = project.path(header.address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        references: vec![],
        kind: "Scene".into(),
        kind_version: 1,
        provenance: header.provenance.then(|| ImportProvenance {
            source: header.source.into(),
            sub_asset: "scene".into(),
        }),
        import: header.import,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::write(
        path,
        write_content_asset(&header, b"untouched payload").unwrap(),
    )
    .unwrap();
}

fn scene_of(source: &'static str) -> Header {
    Header {
        address: "content/triangle/scene.gasset",
        source,
        provenance: true,
        import: None,
    }
}

fn record(importer: &str, settings: Vec<u8>) -> Option<ImportRecord> {
    Some(ImportRecord {
        importer: importer.into(),
        settings_version: 1,
        settings,
        source: SourceFile {
            path: "assets/triangle.gltf".into(),
            hash: 0,
        },
        dependencies: Vec::new(),
    })
}

fn owned_by_another_source(project: &Project) -> PathBuf {
    write_header(project, scene_of("assets/another_source.gltf"));
    project.source("assets/triangle.gltf")
}

fn claimed_twice(project: &Project) -> PathBuf {
    write_header(project, scene_of("assets/triangle.gltf"));
    write_header(
        project,
        Header {
            address: "content/elsewhere/scene.gasset",
            ..scene_of("assets/triangle.gltf")
        },
    );
    project.source("assets/triangle.gltf")
}

fn saved_from_the_editor(project: &Project) -> PathBuf {
    write_header(
        project,
        Header {
            provenance: false,
            ..scene_of("assets/triangle.gltf")
        },
    );
    project.source("assets/triangle.gltf")
}

fn recorded_by_another_importer(project: &Project) -> PathBuf {
    write_header(
        project,
        Header {
            import: record("obj", Vec::new()),
            ..scene_of("assets/triangle.gltf")
        },
    );
    project.source("assets/triangle.gltf")
}

const WALK: &str = "HIERARCHY\nROOT Hips\n{\n\tOFFSET 0 0 0\n\tCHANNELS 1 Xposition\n}\nMOTION\nFrames: 3\nFrame Time: 0.1\n0\n1\n2\n";

fn an_undecodable_record(project: &Project) -> PathBuf {
    write_header(
        project,
        Header {
            address: "content/walk/scene.gasset",
            import: record("bvh", vec![1]),
            ..scene_of("assets/walk.bvh")
        },
    );
    project.walk("assets/walk.bvh")
}

struct Refusal {
    case: &'static str,
    setup: fn(&Project) -> PathBuf,
    reason: &'static str,
}

#[test]
fn an_import_that_would_lose_an_identity_or_break_a_setting_writes_nothing() {
    let cases = [
        Refusal {
            case: "an output owned by another source",
            setup: owned_by_another_source,
            reason: "owned by another asset",
        },
        Refusal {
            case: "two records claim one output",
            setup: claimed_twice,
            reason: "are both 'scene' of 'assets/triangle.gltf'",
        },
        Refusal {
            case: "a file saved from the editor at an output's address",
            setup: saved_from_the_editor,
            reason: "'content/triangle/scene.gasset' was saved from the editor and is not this import's 'scene'",
        },
        Refusal {
            case: "a record by another importer",
            setup: recorded_by_another_importer,
            reason: "'content/triangle/scene.gasset' was imported by 'obj', not 'gltf'",
        },
        Refusal {
            case: "an undecodable record",
            setup: an_undecodable_record,
            reason: "the import record of 'content/walk/scene.gasset' does not decode",
        },
    ];
    for case in &cases {
        let project = Project::new();
        let source = (case.setup)(&project);
        let before = project.snapshot();

        let error = project.try_import(&source).expect_err(case.case);

        let message = format!("{error:#}");
        assert!(message.contains(case.reason), "{}: {message}", case.case);
        assert_eq!(project.snapshot(), before, "{}", case.case);
    }
}

fn locations(assets: &[AssetEntry]) -> BTreeMap<AssetId, String> {
    assets
        .iter()
        .map(|asset| (asset.id, asset.location()))
        .collect()
}

fn walk_record(project: &Project) -> ImportRecord {
    read_content_asset_header(&project.path("content/walk/scene.gasset"))
        .unwrap()
        .import
        .expect("the scene holds the import record")
}

fn recorded_settings(record: &ImportRecord) -> BvhSettings {
    importer_for("bvh")
        .unwrap()
        .settings_from_bytes(record.settings_version, &record.settings)
        .unwrap()
        .expect("the record is at the importer's settings version")
        .downcast_ref::<BvhSettings>()
        .unwrap()
        .clone()
}

#[test]
fn a_reimport_from_the_content_keeps_its_settings_and_follows_a_moved_source() {
    let project = Project::new();
    let config = ContentConfig::default();
    let source = project.walk("assets/walk.bvh");
    let settings = BvhSettings {
        scale: 0.5,
        frame_range: Some(FrameRange { start: 1, end: 3 }),
    };
    let first = concerto_import::import_source_into(
        &source,
        "assets/walk.bvh",
        &project.0,
        &config,
        ImportSettings {
            settings: Some(Box::new(settings.clone())),
            asset_settings: None,
        },
    )
    .expect("import succeeds");
    let id_of = |sub_asset: &str| {
        first
            .assets
            .iter()
            .find(|asset| sub_asset_of(asset).starts_with(sub_asset))
            .unwrap()
            .id
    };

    let again = reimport(
        id_of("scene"),
        None,
        &project.0,
        &config,
        ImportSettings::default(),
    )
    .expect("re-import from the recorded source");
    assert_eq!(locations(&again.assets), locations(&first.assets));
    let record = walk_record(&project);
    assert_eq!(recorded_settings(&record), settings);
    assert_eq!(record.source.path, "assets/walk.bvh");
    let mut hash = Fnv1a::new();
    hash.raw(WALK.as_bytes());
    assert_eq!(record.source.hash, hash.finish());

    let moved = project.path("assets/moved/walk.bvh");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&source, &moved).unwrap();
    let followed = reimport(
        id_of("animation/"),
        Some(&moved),
        &project.0,
        &config,
        ImportSettings::default(),
    )
    .expect("re-import from the moved source");
    assert_eq!(locations(&followed.assets), locations(&first.assets));
    let sources: BTreeSet<String> =
        AssetRegistry::from_content_tree(&project.0, "content", "gasset")
            .unwrap()
            .iter()
            .filter_map(|entry| Some(entry.provenance.as_ref()?.source.clone()))
            .collect();
    assert_eq!(
        sources,
        BTreeSet::from(["assets/moved/walk.bvh".to_string()])
    );
    let record = walk_record(&project);
    assert_eq!(record.source.path, "assets/moved/walk.bvh");
    assert_eq!(recorded_settings(&record), settings);

    let renamed = project.path("assets/walk.txt");
    std::fs::write(&renamed, WALK).unwrap();
    let run = project.walk("assets/run.bvh");
    project.import(&run);
    let before = project.snapshot();
    let refused = |source: &Path| {
        let error = reimport(
            id_of("scene"),
            Some(source),
            &project.0,
            &config,
            ImportSettings::default(),
        )
        .expect_err("the source is refused");
        format!("{error:#}")
    };
    let message = refused(&renamed);
    assert!(
        message.contains("must have the extension '.bvh' of 'assets/moved/walk.bvh'"),
        "{message}"
    );
    let message = refused(&run);
    assert!(
        message.contains("'assets/run.bvh' is already the source of 'content/run/scene.gasset'"),
        "{message}"
    );
    assert_eq!(project.snapshot(), before);

    std::fs::remove_file(&moved).unwrap();
    let error = reimport(
        id_of("scene"),
        None,
        &project.0,
        &config,
        ImportSettings::default(),
    )
    .expect_err("the source is gone");
    let message = format!("{error:#}");
    assert!(
        message.contains(
            "'assets/moved/walk.bvh', the source of 'content/walk/scene.gasset', is missing"
        ),
        "{message}"
    );

    let elsewhere = Project::new();
    let outside = elsewhere.walk("stride.bvh");
    let copied = reimport(
        id_of("scene"),
        Some(&outside),
        &project.0,
        &config,
        ImportSettings::default(),
    )
    .expect("re-import from a file outside the project");
    assert_eq!(
        locations(&copied.assets)
            .get(&id_of("scene"))
            .map(String::as_str),
        Some("content/walk/scene.gasset")
    );
    assert_eq!(
        std::fs::read(project.path("assets/moved/stride.bvh")).unwrap(),
        WALK.as_bytes()
    );
    let ids: BTreeSet<AssetId> = copied.assets.iter().map(|asset| asset.id).collect();
    let sources: BTreeSet<String> =
        AssetRegistry::from_content_tree(&project.0, "content", "gasset")
            .unwrap()
            .iter()
            .filter(|entry| ids.contains(&entry.id))
            .filter_map(|entry| Some(entry.provenance.as_ref()?.source.clone()))
            .collect();
    assert_eq!(
        sources,
        BTreeSet::from(["assets/moved/stride.bvh".to_string()])
    );
    assert_eq!(walk_record(&project).source.path, "assets/moved/stride.bvh");
}
