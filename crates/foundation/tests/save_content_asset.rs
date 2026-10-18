//! save_content_asset writes a content asset into a project tree, minting
//! its id on first save and reusing it on re-save.
use concerto_foundation::assets::content::{read_content_asset, save_content_asset};
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

    save_content_asset(&widget, &project_root, address).expect("save");

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
    save_content_asset(
        &Widget { spokes: 8 },
        &project_root,
        "content/a/b/c/deep.gasset",
    )
    .expect("save creates a/b/c");
    assert!(project_root.join("content/a/b/c/deep.gasset").exists());
    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn saving_into_a_registry_returns_the_id_and_leaves_the_registry_file_alone() {
    use concerto_foundation::assets::content::{
        read_content_asset_header, save_content_asset_into, AssetRegistry, REGISTRY_FILE_NAME,
    };
    let project_root =
        std::env::temp_dir().join(format!("save-content-into-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let mut registry = AssetRegistry::new();
    let existing = AssetId::new();
    registry.insert(existing, "content/other.gasset");

    let id = save_content_asset_into(&Widget { spokes: 3 }, &project_root, address, &mut registry)
        .expect("save");

    assert_eq!(
        read_content_asset_header(&project_root.join(address))
            .unwrap()
            .asset_id,
        id
    );
    assert_eq!(registry.get(id), Some(address));
    assert_eq!(registry.get(existing), Some("content/other.gasset"));
    assert!(
        !project_root.join(REGISTRY_FILE_NAME).exists(),
        "the caller decides when its registry is persisted"
    );
    assert_eq!(
        save_content_asset_into(&Widget { spokes: 4 }, &project_root, address, &mut registry)
            .unwrap(),
        id,
        "a re-save keeps the identity"
    );

    std::fs::remove_dir_all(&project_root).ok();
}
