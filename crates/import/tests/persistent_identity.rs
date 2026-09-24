//! End-to-end identity and ownership guarantees for the offline import workflow.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use concerto_core::assets::content::{
    read_content_asset, read_content_asset_header, write_content_asset, AssetRegistry,
    ContentAssetHeader, ImportProvenance, CONTENT_FORMAT_VERSION,
};
use concerto_core::assets::AssetId;
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
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../gltf-loader/tests/fixtures/triangle.gltf"),
            &source,
        )
        .unwrap();
        source
    }

    fn import(&self, source: &Path) -> Vec<concerto_import::ImportedAsset> {
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

fn output<'a>(assets: &'a [concerto_import::ImportedAsset], sub_asset: &str) -> &'a str {
    &assets
        .iter()
        .find(|a| a.sub_asset_name == sub_asset)
        .unwrap()
        .address
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
        AssetRegistry::load(&project.0).unwrap().get(id),
        Some(moved_address)
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

    assert_eq!(
        first, second,
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
            .find(|a| a.sub_asset_name == asset.sub_asset_name)
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
        AssetRegistry::load(&project.0).unwrap().iter().count(),
        first.len() + second.len()
    );
}

#[test]
fn an_output_owned_by_another_source_rejects_import_before_any_writes() {
    let project = Project::new();
    let source = project.source("assets/triangle.gltf");
    let occupied = project.path("content/triangle/scene.gasset");
    std::fs::create_dir_all(occupied.parent().unwrap()).unwrap();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::new(),
        references: vec![],
        kind: "Scene".into(),
        provenance: Some(ImportProvenance {
            source: "assets/another_source.gltf".into(),
            sub_asset: "scene".into(),
        }),
    };
    std::fs::write(
        occupied,
        write_content_asset(&header, b"untouched payload").unwrap(),
    )
    .unwrap();
    let before = project.snapshot();

    assert!(concerto_import::import_source(&source, &project.0, &Default::default()).is_err());
    assert_eq!(
        project.snapshot(),
        before,
        "a collision must not leave partial outputs or metadata"
    );
}

#[test]
fn a_corrupt_existing_header_fails_without_modifying_content_or_metadata() {
    let project = Project::new();
    let source = project.source("assets/triangle.gltf");
    let first = project.import(&source);
    std::fs::write(project.path(output(&first, "scene")), b"corrupt header").unwrap();
    rename_node(&source, "Should not be imported");
    let before = project.snapshot();

    assert!(concerto_import::import_source(&source, &project.0, &Default::default()).is_err());
    assert_eq!(
        project.snapshot(),
        before,
        "corruption must not reset identity or modify another output"
    );
}

#[test]
fn legacy_content_bootstraps_metadata_and_retains_its_uuid_and_address() {
    let project = Project::new();
    let source = project.source("assets/characters/hero.gltf");
    let legacy_address = "content/hero/scene.gasset";
    let legacy_id = AssetId::from_path(legacy_address);
    let legacy_path = project.path(legacy_address);
    std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: legacy_id,
        references: vec![],
        kind: "Scene".into(),
        provenance: Some(ImportProvenance {
            source: "assets/characters/hero.gltf".into(),
            sub_asset: "scene".into(),
        }),
    };
    std::fs::write(
        &legacy_path,
        write_content_asset(&header, b"legacy payload").unwrap(),
    )
    .unwrap();

    let imported = project.import(&source);

    assert_eq!(output(&imported, "scene"), legacy_address);
    assert_eq!(
        read_content_asset_header(&legacy_path).unwrap().asset_id,
        legacy_id
    );
    let metadata: toml::Value =
        toml::from_str(&std::fs::read_to_string(sidecar(&source)).unwrap()).unwrap();
    assert_eq!(metadata["version"].as_integer(), Some(1));
    assert_eq!(metadata["importer"].as_str(), Some("gltf"));
    assert_eq!(
        AssetId::from_simple_hex(metadata["outputs"]["scene"]["asset_id"].as_str().unwrap())
            .unwrap(),
        legacy_id
    );
    assert_eq!(
        AssetRegistry::load(&project.0).unwrap().get(legacy_id),
        Some(legacy_address)
    );
}

#[test]
fn copying_a_sidecar_cannot_claim_another_live_sources_outputs() {
    let project = Project::new();
    let original = project.source("assets/original.gltf");
    project.import(&original);
    let copied = project.source("assets/copied.gltf");
    std::fs::copy(sidecar(&original), sidecar(&copied)).unwrap();
    let before = project.snapshot();

    assert!(concerto_import::import_source(&copied, &project.0, &Default::default()).is_err());
    assert_eq!(project.snapshot(), before);
}

#[test]
fn external_legacy_source_bootstraps_without_changing_its_identity() {
    let project = Project::new();
    let external = Project::new();
    let source = external.source("triangle.gltf");
    let address = "content/triangle/scene.gasset";
    let id = AssetId::new();
    let path = project.path(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: vec![],
        kind: "Scene".into(),
        provenance: Some(ImportProvenance {
            source: source.to_str().unwrap().into(),
            sub_asset: "scene".into(),
        }),
    };
    std::fs::write(
        &path,
        write_content_asset(&header, b"legacy payload").unwrap(),
    )
    .unwrap();

    let imported = project.import(&source);

    assert_eq!(output(&imported, "scene"), address);
    assert_eq!(read_content_asset_header(&path).unwrap().asset_id, id);
    assert!(sidecar(&source).is_file());
    assert_eq!(
        AssetRegistry::load(&project.0).unwrap().get(id),
        Some(address)
    );
}
