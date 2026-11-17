//! End-to-end identity and ownership guarantees for the offline import workflow.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use concerto_foundation::assets::content::{
    asset_record, read_asset_record, read_content_asset, read_content_asset_header,
    write_content_asset, ContentAssetHeader, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::AssetId;
use concerto_import::config::content_address;
use concerto_import::metadata::SourceMetadata;
use concerto_scene::scene::Scene;

pub(crate) struct Project(pub(crate) PathBuf);

impl Project {
    pub(crate) fn new() -> Self {
        let root = std::env::temp_dir().join(format!("import-identity-{:?}", AssetId::new()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    pub(crate) fn path(&self, relative: &str) -> PathBuf {
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

    pub(crate) fn import(&self, source: &Path) -> Vec<AssetEntry> {
        concerto_import::import_source(source, &self.0, &Default::default())
            .expect("import succeeds")
    }

    /// Compare the complete project so failures cannot hide a modified sidecar,
    /// registry, partially rewritten output, or abandoned staging file.
    pub(crate) fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
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

fn sidecar(source: &Path) -> PathBuf {
    source.with_file_name(format!(
        "{}.import.toml",
        source.file_name().unwrap().to_str().unwrap()
    ))
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

pub(crate) fn sorted(registry: &AssetRegistry) -> Vec<AssetEntry> {
    let mut entries: Vec<_> = registry.iter().cloned().collect();
    entries.sort_by_key(|entry| entry.location());
    entries
}

pub(crate) fn loose_address(source: &str, entry: &AssetEntry) -> String {
    content_address(&Default::default(), Path::new(source), sub_asset_of(entry))
}

pub(crate) fn unpack_into_the_old_layout(project: &Project, source: &str, assets: &[AssetEntry]) {
    for entry in assets.iter().filter(|a| a.embedded_in.is_some()) {
        std::fs::write(
            project.path(&loose_address(source, entry)),
            read_asset_record(&project.0, entry).unwrap(),
        )
        .unwrap();
    }
    let host = project.path(output(assets, "scene"));
    let file = std::fs::read(&host).unwrap();
    std::fs::write(&host, asset_record(&file, None).unwrap()).unwrap();
}

fn save_from_the_editor(path: &Path) {
    let file = std::fs::read(path).unwrap();
    let (mut header, payload) = read_content_asset(&file).unwrap();
    header.provenance = None;
    header.embedded.clear();
    std::fs::write(path, write_content_asset(&header, payload).unwrap()).unwrap();
}

#[test]
fn a_reimport_from_the_old_layout_keeps_every_id() {
    let project = Project::new();
    let source = project.source("assets/triangle.gltf");
    let first = project.import(&source);
    unpack_into_the_old_layout(&project, "assets/triangle.gltf", &first);
    let mesh = first.iter().find(|a| sub_asset_of(a) == "mesh/0").unwrap();
    let loose = project.path(&loose_address("assets/triangle.gltf", mesh));

    let second = project.import(&source);

    assert_eq!(second, first);
    assert!(!loose.exists(), "the loose mesh is now in the scene file");
    let rescanned = AssetRegistry::from_content_tree(&project.0, "content", "gasset").unwrap();
    assert_eq!(sorted(&rescanned), {
        let mut entries = first.clone();
        entries.sort_by_key(|entry| entry.location());
        entries
    });
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
fn reimport_leaves_an_output_saved_from_the_editor_and_updates_the_rest() {
    let project = Project::new();
    let source = project.source("assets/triangle.gltf");
    let first = project.import(&source);
    unpack_into_the_old_layout(&project, "assets/triangle.gltf", &first);
    let mesh = first.iter().find(|a| sub_asset_of(a) == "mesh/0").unwrap();
    let mesh_address = loose_address("assets/triangle.gltf", mesh);
    let mesh_path = project.path(&mesh_address);
    save_from_the_editor(&mesh_path);
    let saved = std::fs::read(&mesh_path).unwrap();
    let scene_path = project.path(output(&first, "scene"));
    rename_node(&source, "Changed in the source");

    let second = project.import(&source);

    assert_eq!(std::fs::read(&mesh_path).unwrap(), saved);
    assert!(second.iter().all(|asset| asset.id != mesh.id));
    let raw = std::fs::read(&scene_path).unwrap();
    let (_, payload) = read_content_asset(&raw).unwrap();
    assert_eq!(
        bincode::deserialize::<Scene>(payload).unwrap().nodes[0].name,
        "Changed in the source"
    );
    let rescanned = AssetRegistry::from_content_tree(&project.0, "content", "gasset").unwrap();
    assert_eq!(rescanned.get(mesh.id).unwrap().location(), mesh_address);
}

#[test]
fn moving_a_source_with_its_sidecar_preserves_existing_outputs() {
    let project = Project::new();
    let source = project.source("assets/characters/hero.gltf");
    let first = project.import(&source);
    let moved = project.path("assets/enemies/renamed.gltf");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&source, &moved).unwrap();
    std::fs::rename(sidecar(&source), sidecar(&moved)).unwrap();
    rename_node(&moved, "Moved source");

    let second = project.import(&moved);

    let outputs = |assets: &[AssetEntry]| {
        assets
            .iter()
            .map(|a| (a.id, a.address.clone(), sub_asset_of(a).to_owned()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        outputs(&first),
        outputs(&second),
        "output addresses and importer keys survive source moves"
    );
    let raw = std::fs::read(project.path(output(&second, "scene"))).unwrap();
    let (header, payload) = read_content_asset(&raw).unwrap();
    assert_eq!(
        header.provenance.unwrap().source,
        "assets/enemies/renamed.gltf"
    );
    assert_eq!(
        bincode::deserialize::<Scene>(payload).unwrap().nodes[0].name,
        "Moved source"
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

fn write_scene_header(path: &Path, id: AssetId, source: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let header = ContentAssetHeader {
        asset_id: id,
        references: vec![],
        kind: "Scene".into(),
        kind_version: 1,
        provenance: Some(ImportProvenance {
            source: source.into(),
            sub_asset: "scene".into(),
        }),
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::write(
        path,
        write_content_asset(&header, b"untouched payload").unwrap(),
    )
    .unwrap();
}

fn owned_by_another_source(project: &Project) -> PathBuf {
    let source = project.source("assets/triangle.gltf");
    write_scene_header(
        &project.path("content/triangle/scene.gasset"),
        AssetId::new(),
        "assets/another_source.gltf",
    );
    source
}

fn copied_sidecar(project: &Project) -> PathBuf {
    let original = project.source("assets/original.gltf");
    project.import(&original);
    let copied = project.source("assets/copied.gltf");
    std::fs::copy(sidecar(&original), sidecar(&copied)).unwrap();
    copied
}

const WALK: &str = "HIERARCHY\nROOT Hips\n{\n\tOFFSET 0 0 0\n\tCHANNELS 1 Xposition\n}\nMOTION\nFrames: 2\nFrame Time: 0.1\n0\n1\n";

fn invalid_setting(project: &Project) -> PathBuf {
    let source = project.path("assets/walk.bvh");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, WALK).unwrap();
    std::fs::write(
        sidecar(&source),
        "version = 1\nsource_id = \"00000000-0000-0000-0000-000000000001\"\nimporter = \"bvh\"\n\n[settings]\nscale = -1.0\n",
    )
    .unwrap();
    source
}

struct Refusal {
    case: &'static str,
    setup: fn(&Project) -> PathBuf,
}

#[test]
fn an_import_that_would_lose_an_identity_or_break_a_setting_writes_nothing() {
    let cases = [
        Refusal {
            case: "an output owned by another source",
            setup: owned_by_another_source,
        },
        Refusal {
            case: "a sidecar copied from a live source",
            setup: copied_sidecar,
        },
        Refusal {
            case: "an invalid setting in the sidecar",
            setup: invalid_setting,
        },
    ];
    for case in &cases {
        let project = Project::new();
        let source = (case.setup)(&project);
        let before = project.snapshot();

        concerto_import::import_source(&source, &project.0, &Default::default())
            .expect_err(case.case);

        assert_eq!(project.snapshot(), before, "{}", case.case);
    }
}

#[test]
fn legacy_content_bootstraps_a_sidecar_and_keeps_its_uuid_and_address() {
    let project = Project::new();
    let source = project.source("assets/characters/hero.gltf");
    let address = "content/hero/scene.gasset";
    let id = AssetId::from_path(address);
    write_scene_header(&project.path(address), id, "assets/characters/hero.gltf");

    let imported = project.import(&source);

    assert_eq!(output(&imported, "scene"), address);
    assert_eq!(
        read_content_asset_header(&project.path(address))
            .unwrap()
            .asset_id,
        id
    );
    assert_eq!(
        SourceMetadata::load(&sidecar(&source)).unwrap().outputs["scene"].asset_id,
        id
    );
}
