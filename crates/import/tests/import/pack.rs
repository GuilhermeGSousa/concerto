//! `pack_imports` moves a project still in the one-file-per-output layout into scene files.
use std::collections::BTreeMap;

use concerto_foundation::assets::content::{
    asset_record, read_asset_record, read_content_asset, read_editor_only_section,
    write_content_asset, write_content_asset_with_editor_only, ContentAssetHeader,
    ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::AssetId;
use concerto_import::metadata::{OutputMetadata, SourceMetadata};
use concerto_import::{pack_imports, PackedSource};

use crate::import_gltf::{content_files, write_hero, Runtime};
use crate::persistent_identity::{loose_address, sorted, unpack_into_the_old_layout, Project};

const HERO: &str = "assets/hero.gltf";
const WALK_CLIP: &str = "content/walk/animation_walk.gasset";

fn clip_only_source(project: &Project, clip: &AssetEntry) -> AssetEntry {
    let file = std::fs::read(project.path(&clip.address)).unwrap();
    let (header, payload) = read_content_asset(&file).unwrap();
    let provenance = ImportProvenance {
        source: "assets/walk.bvh".into(),
        sub_asset: "animation/walk".into(),
    };
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        provenance: Some(provenance.clone()),
        ..header
    };
    let path = project.path(WALK_CLIP);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, write_content_asset(&header, payload).unwrap()).unwrap();
    let metadata = SourceMetadata {
        version: 1,
        source_id: AssetId::new(),
        importer: "bvh".into(),
        outputs: BTreeMap::from([(
            provenance.sub_asset.clone(),
            OutputMetadata {
                asset_id: header.asset_id,
                file: false,
            },
        )]),
        settings: toml::Table::new(),
    };
    std::fs::write(
        project.path("assets/walk.bvh.import.toml"),
        toml::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();
    AssetEntry {
        id: header.asset_id,
        address: WALK_CLIP.into(),
        kind: header.kind,
        kind_version: Some(header.kind_version),
        provenance: Some(provenance),
        embedded_in: None,
    }
}

#[test]
fn packing_keeps_ids_and_bytes_and_is_idempotent() {
    let project = Project::new();
    let source = project.path(HERO);
    write_hero(&source, 2);
    let imported = project.import(&source);
    let packed_files = content_files(&project.0);
    unpack_into_the_old_layout(&project, HERO, &imported);
    let clip = imported.iter().find(|a| a.kind == "AnimationClip").unwrap();
    let walk = clip_only_source(&project, clip);
    let embedded: Vec<&AssetEntry> = imported
        .iter()
        .filter(|a| a.embedded_in.is_some())
        .collect();
    let drawn = embedded[0];
    let drawing = [7u8, 8, 9];
    let loose = project.path(&loose_address(HERO, drawn));
    let file = std::fs::read(&loose).unwrap();
    let (header, payload) = read_content_asset(&file).unwrap();
    std::fs::write(
        &loose,
        write_content_asset_with_editor_only(&header, payload, &drawing).unwrap(),
    )
    .unwrap();
    let old = project.snapshot();
    let old_record = |entry: &AssetEntry| &old[std::path::Path::new(&loose_address(HERO, entry))];

    let report = pack_imports(&project.0).expect("the tree packs");

    assert_eq!(
        report.sources,
        [PackedSource {
            source: HERO.into(),
            embedded: embedded.len(),
        }]
    );
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    let mut expected_files = packed_files.clone();
    expected_files.push(WALK_CLIP.into());
    expected_files.sort();
    assert_eq!(content_files(&project.0), expected_files);
    let mut expected = imported.clone();
    expected.push(walk);
    expected.sort_by_key(|entry| entry.location());
    let registry = AssetRegistry::from_content_tree(&project.0, "content", "gasset").unwrap();
    assert_eq!(sorted(&registry), expected);
    for entry in &embedded {
        assert_eq!(
            &read_asset_record(&project.0, entry).unwrap(),
            old_record(entry),
            "{}",
            entry.location()
        );
    }
    let section = read_editor_only_section(
        &project.path("content/hero/scene.gasset"),
        Some(&drawn.embedded_in.as_ref().unwrap().name),
    )
    .unwrap()
    .expect("the packed asset keeps its editor-only section");
    assert_eq!(section.bytes, drawing);
    let host = std::fs::read(project.path("content/hero/scene.gasset")).unwrap();
    assert_eq!(
        asset_record(&host, None).unwrap(),
        old[std::path::Path::new("content/hero/scene.gasset")]
    );
    let packed = project.snapshot();
    for untouched in [
        WALK_CLIP,
        "content/hero/texture_0.gasset",
        "content/hero/animation_Wiggle.gasset",
        "assets/hero.gltf.import.toml",
        "assets/walk.bvh.import.toml",
    ] {
        let path = std::path::Path::new(untouched);
        assert_eq!(packed[path], old[path], "{untouched}");
    }
    let mut runtime = Runtime::new(&project.0);
    for entry in &expected {
        assert!(runtime.loads(entry), "{} loads", entry.location());
    }

    let again = pack_imports(&project.0).expect("a packed tree packs again");

    assert_eq!(
        again.sources,
        [PackedSource {
            source: HERO.into(),
            embedded: 0,
        }]
    );
    assert_eq!(project.snapshot(), packed);
}
