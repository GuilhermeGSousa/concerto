//! End-to-end identity and ownership guarantees for the offline import workflow.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, save_content_asset_into, write_content_asset,
    ContentAssetHeader, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::AssetId;
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

    fn import(&self, source: &Path) -> Vec<AssetEntry> {
        concerto_import::import_source(source, &self.0, &Default::default())
            .expect("import succeeds")
    }

    /// Compare the complete project so failures cannot hide a modified sidecar,
    /// registry, partially rewritten output, or abandoned staging file.
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

fn sorted(registry: &AssetRegistry) -> Vec<AssetEntry> {
    let mut entries: Vec<_> = registry.iter().cloned().collect();
    entries.sort_by(|a, b| a.address.cmp(&b.address));
    entries
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
    let scene_address = output(&first, "scene").to_owned();
    let scene_path = project.path(&scene_address);
    let raw = std::fs::read(&scene_path).unwrap();
    let (imported, payload) = read_content_asset(&raw).unwrap();
    let mut scene: Scene = bincode::deserialize(payload).unwrap();
    scene.nodes[0].name = "Edited in the editor".into();
    save_content_asset_into(
        &scene,
        &project.0,
        &scene_address,
        &mut AssetRegistry::new(),
    )
    .unwrap();
    let saved = std::fs::read(&scene_path).unwrap();
    let other = first
        .iter()
        .find(|asset| sub_asset_of(asset) != "scene")
        .unwrap();
    let other_path = project.path(&other.address);
    let fresh = std::fs::read(&other_path).unwrap();
    let (header, _) = read_content_asset(&fresh).unwrap();
    std::fs::write(&other_path, write_content_asset(&header, b"stale").unwrap()).unwrap();
    rename_node(&source, "Changed in the source");
    let mut registry = AssetRegistry::from_content_tree(&project.0, "content", "gasset").unwrap();

    let second = project.import(&source);

    assert_eq!(std::fs::read(&scene_path).unwrap(), saved);
    assert_eq!(std::fs::read(&other_path).unwrap(), fresh);
    assert_eq!(second.len(), first.len() - 1);
    assert!(second.iter().all(|asset| asset.id != imported.asset_id));
    assert!(second.iter().all(|asset| sub_asset_of(asset) != "scene"));
    for entry in &second {
        registry.insert(entry.clone());
    }
    let rescanned = AssetRegistry::from_content_tree(&project.0, "content", "gasset").unwrap();
    assert_eq!(sorted(&registry), sorted(&rescanned));
    assert_eq!(
        rescanned
            .get(imported.asset_id)
            .map(|entry| (entry.address.as_str(), entry.provenance.is_none())),
        Some((scene_address.as_str(), true))
    );
    let metadata: toml::Value =
        toml::from_str(&std::fs::read_to_string(sidecar(&source)).unwrap()).unwrap();
    assert_eq!(
        AssetId::from_simple_hex(metadata["outputs"]["scene"]["asset_id"].as_str().unwrap())
            .unwrap(),
        imported.asset_id
    );
}

#[test]
fn moving_a_source_with_its_sidecar_preserves_existing_outputs() {
    let project = Project::new();
    let source = project.source("assets/characters/hero.gltf");
    let first = project.import(&source);
    let ids: Vec<_> = first
        .iter()
        .map(|a| {
            (
                a.address.clone(),
                read_content_asset_header(&project.path(&a.address))
                    .unwrap()
                    .asset_id,
            )
        })
        .collect();
    let metadata = std::fs::read(sidecar(&source)).unwrap();
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
    assert_eq!(std::fs::read(sidecar(&moved)).unwrap(), metadata);
    for (address, id) in ids {
        assert_eq!(
            read_content_asset_header(&project.path(&address))
                .unwrap()
                .asset_id,
            id
        );
    }
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
        assert_ne!(
            read_content_asset_header(&project.path(&asset.address))
                .unwrap()
                .asset_id,
            read_content_asset_header(&project.path(&counterpart.address))
                .unwrap()
                .asset_id,
        );
    }
    assert_eq!(
        AssetRegistry::from_content_tree(&project.0, "content", "gasset")
            .unwrap()
            .iter()
            .count(),
        first.len() + second.len()
    );
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
        derived: None,
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

fn corrupt_header(project: &Project) -> PathBuf {
    let source = project.source("assets/triangle.gltf");
    let first = project.import(&source);
    std::fs::write(project.path(output(&first, "scene")), b"corrupt header").unwrap();
    rename_node(&source, "Should not be imported");
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
    message: &'static str,
}

#[test]
fn an_import_that_would_lose_an_identity_or_break_a_setting_writes_nothing() {
    let cases = [
        Refusal {
            case: "an output owned by another source",
            setup: owned_by_another_source,
            message: "",
        },
        Refusal {
            case: "a corrupt existing header",
            setup: corrupt_header,
            message: "",
        },
        Refusal {
            case: "a sidecar copied from a live source",
            setup: copied_sidecar,
            message: "",
        },
        Refusal {
            case: "an invalid setting in the sidecar",
            setup: invalid_setting,
            message: "scale",
        },
    ];
    for case in &cases {
        let project = Project::new();
        let source = (case.setup)(&project);
        let before = project.snapshot();

        let error = concerto_import::import_source(&source, &project.0, &Default::default())
            .expect_err(case.case);

        assert!(
            format!("{error:?}").contains(case.message),
            "{}: {error:?}",
            case.case
        );
        assert_eq!(project.snapshot(), before, "{}", case.case);
    }
}

struct Legacy {
    external: bool,
    source: &'static str,
    address: &'static str,
}

#[test]
fn legacy_content_bootstraps_a_sidecar_and_keeps_its_uuid_and_address() {
    let cases = [
        Legacy {
            external: false,
            source: "assets/characters/hero.gltf",
            address: "content/hero/scene.gasset",
        },
        Legacy {
            external: true,
            source: "triangle.gltf",
            address: "content/triangle/scene.gasset",
        },
    ];
    for case in &cases {
        let project = Project::new();
        let elsewhere = Project::new();
        let source = if case.external {
            elsewhere.source(case.source)
        } else {
            project.source(case.source)
        };
        let recorded = if case.external {
            source.to_str().unwrap()
        } else {
            case.source
        };
        let id = AssetId::from_path(case.address);
        write_scene_header(&project.path(case.address), id, recorded);

        let imported = project.import(&source);

        assert_eq!(output(&imported, "scene"), case.address);
        assert_eq!(
            read_content_asset_header(&project.path(case.address))
                .unwrap()
                .asset_id,
            id
        );
        let metadata: toml::Value =
            toml::from_str(&std::fs::read_to_string(sidecar(&source)).unwrap()).unwrap();
        assert_eq!(metadata["version"].as_integer(), Some(1));
        assert_eq!(metadata["importer"].as_str(), Some("gltf"));
        assert_eq!(
            AssetId::from_simple_hex(metadata["outputs"]["scene"]["asset_id"].as_str().unwrap())
                .unwrap(),
            id
        );
        assert_eq!(
            AssetRegistry::from_content_tree(&project.0, "content", "gasset")
                .unwrap()
                .get(id)
                .map(|entry| entry.address.as_str()),
            Some(case.address)
        );
    }
}
