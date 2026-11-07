//! save_content_asset_into writes a content asset into a project tree, minting
//! its id on first save and reusing it on re-save.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, resave_content_asset, save_content_asset_into,
    write_content_asset, ContentAssetHeader, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Widget {
    spokes: u32,
}

impl Asset for Widget {
    fn name() -> &'static str {
        "Widget"
    }
    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        vec![AssetId::from_path("content/parts/spoke.gasset")]
    }
}

#[test]
fn writes_a_readable_content_asset_under_the_project_root() {
    let project_root = std::env::temp_dir().join(format!("save-content-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let widget = Widget { spokes: 32 };

    save_content_asset_into(&widget, &project_root, address, &mut AssetRegistry::new())
        .expect("save");

    let written = std::fs::read(project_root.join(address)).expect("file exists at the address");
    let (header, payload) = read_content_asset(&written).expect("readable");

    assert_eq!(header.kind, "Widget");
    assert_eq!(
        header.references,
        vec![AssetId::from_path("content/parts/spoke.gasset")],
        "the header carries the value's outbound references"
    );
    assert_eq!(
        bincode::deserialize::<Widget>(payload).unwrap(),
        widget,
        "payload round-trips"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn creates_missing_parent_directories() {
    let project_root =
        std::env::temp_dir().join(format!("save-content-deep-{}", std::process::id()));
    save_content_asset_into(
        &Widget { spokes: 8 },
        &project_root,
        "content/a/b/c/deep.gasset",
        &mut AssetRegistry::new(),
    )
    .expect("save creates a/b/c");
    assert!(project_root.join("content/a/b/c/deep.gasset").exists());
    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn saving_into_a_registry_returns_the_id_and_records_it() {
    let project_root =
        std::env::temp_dir().join(format!("save-content-into-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let mut registry = AssetRegistry::new();
    let existing = AssetId::new();
    registry.insert(AssetEntry {
        id: existing,
        address: "content/other.gasset".into(),
        kind: "Widget".into(),
        kind_version: None,
        provenance: None,
    });

    let id = save_content_asset_into(&Widget { spokes: 3 }, &project_root, address, &mut registry)
        .expect("save");

    assert_eq!(
        read_content_asset_header(&project_root.join(address))
            .unwrap()
            .asset_id,
        id
    );
    assert_eq!(
        registry.get(id).map(|entry| entry.address.as_str()),
        Some(address)
    );
    assert_eq!(
        registry.get(existing).map(|entry| entry.address.as_str()),
        Some("content/other.gasset")
    );
    assert_eq!(
        save_content_asset_into(&Widget { spokes: 4 }, &project_root, address, &mut registry)
            .unwrap(),
        id,
        "a re-save keeps the identity"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Axle {
    length: u32,
}

impl Asset for Axle {
    const FORMAT_VERSION: u32 = 4;

    fn name() -> &'static str {
        "Axle"
    }
}

#[test]
fn a_save_writes_the_type_version() {
    let project_root =
        std::env::temp_dir().join(format!("save-content-version-{}", std::process::id()));
    let address = "content/axles/front.gasset";
    let mut registry = AssetRegistry::new();

    let id = save_content_asset_into(&Axle { length: 2 }, &project_root, address, &mut registry)
        .expect("save");

    assert_eq!(
        read_content_asset_header(&project_root.join(address))
            .unwrap()
            .kind_version,
        4
    );
    assert_eq!(
        registry.get(id).and_then(|entry| entry.kind_version),
        Some(4)
    );

    std::fs::remove_dir_all(&project_root).ok();
}

fn write_imported_widget(path: &std::path::Path, spokes: u32) -> ContentAssetHeader {
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        references: Vec::new(),
        kind: "Widget".to_string(),
        kind_version: Widget::FORMAT_VERSION,
        provenance: Some(ImportProvenance {
            source: "assets/wheel.glb".to_string(),
            sub_asset: "widget/0".to_string(),
        }),
        derived: None,
    };
    let payload = bincode::serialize(&Widget { spokes }).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, &payload).unwrap()).unwrap();
    header
}

#[test]
fn a_resave_keeps_the_id_and_the_import_provenance() {
    let project_root = std::env::temp_dir().join(format!("resave-content-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let original = write_imported_widget(&project_root.join(address), 32);
    let mut registry = AssetRegistry::new();

    let id = resave_content_asset(&Widget { spokes: 6 }, &project_root, address, &mut registry)
        .expect("resave");

    assert_eq!(id, original.asset_id);
    let written = std::fs::read(project_root.join(address)).unwrap();
    let (header, payload) = read_content_asset(&written).unwrap();
    assert_eq!(header.asset_id, original.asset_id);
    assert_eq!(header.provenance, original.provenance);
    assert_eq!(
        header.references,
        vec![AssetId::from_path("content/parts/spoke.gasset")]
    );
    assert_eq!(
        bincode::deserialize::<Widget>(payload).unwrap(),
        Widget { spokes: 6 }
    );
    let entry = registry
        .get(id)
        .expect("the registry holds the resaved asset");
    assert_eq!(entry.address, address);
    assert_eq!(entry.kind, "Widget");
    assert_eq!(entry.provenance, original.provenance);

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn a_resave_fails_without_a_file() {
    let project_root =
        std::env::temp_dir().join(format!("resave-content-missing-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let mut registry = AssetRegistry::new();

    resave_content_asset(&Widget { spokes: 6 }, &project_root, address, &mut registry)
        .expect_err("there is nothing to resave");

    assert!(!project_root.join(address).exists());
    assert!(registry.id_for_address(address).is_none());
}

#[test]
fn a_resave_fails_over_another_kind_and_leaves_the_file() {
    let project_root =
        std::env::temp_dir().join(format!("resave-content-kind-{}", std::process::id()));
    let address = "content/axles/front.gasset";
    let mut registry = AssetRegistry::new();
    save_content_asset_into(&Axle { length: 2 }, &project_root, address, &mut registry)
        .expect("save");
    let before = std::fs::read(project_root.join(address)).unwrap();

    resave_content_asset(&Widget { spokes: 6 }, &project_root, address, &mut registry)
        .expect_err("the file holds an Axle");

    assert_eq!(std::fs::read(project_root.join(address)).unwrap(), before);
    assert_eq!(
        registry
            .id_for_address(address)
            .and_then(|id| registry.get(id))
            .map(|entry| entry.kind.as_str()),
        Some("Axle")
    );

    std::fs::remove_dir_all(&project_root).ok();
}
