//! save_content_asset_into writes a content asset into a project tree, minting
//! its id on first save and reusing it on re-save.
use concerto_foundation::assets::content::{
    read_asset_record, read_asset_record_without_editor_only, read_content_asset,
    read_content_asset_header, read_content_asset_parts, read_editor_only, resave_content_asset,
    save_content_asset_into, write_content_asset_with_editor_only,
    write_content_asset_with_embedded, ContentAssetHeader, EmbeddedPayload, ImportProvenance,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{Asset, AssetId};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Widget {
    spokes: u32,
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Blueprint {
    drawing: Vec<u8>,
}

impl Asset for Widget {
    type EditorOnly = Blueprint;

    fn name() -> &'static str {
        "Widget"
    }
    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        vec![AssetId::from_path("content/parts/spoke.gasset")]
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Axle {
    length: u32,
}

impl Asset for Axle {
    type EditorOnly = ();

    const FORMAT_VERSION: u32 = 4;

    fn name() -> &'static str {
        "Axle"
    }
}

#[test]
fn a_save_writes_a_readable_asset_under_a_minted_id_and_a_re_save_keeps_it() {
    let project_root = std::env::temp_dir().join(format!("save-content-{}", std::process::id()));
    let address = "content/a/b/wheel.gasset";
    let widget = Widget { spokes: 32 };
    let mut registry = AssetRegistry::new();
    let existing = AssetId::new();
    registry.insert(AssetEntry {
        embedded_in: None,
        id: existing,
        address: "content/other.gasset".into(),
        kind: "Widget".into(),
        kind_version: None,
        provenance: None,
    });

    let blueprint = Blueprint {
        drawing: vec![7, 8, 9],
    };

    let id = save_content_asset_into(
        &widget,
        Some(&blueprint),
        &project_root,
        address,
        &mut registry,
    )
    .expect("save");

    let written = std::fs::read(project_root.join(address)).expect("file exists at the address");
    let (header, payload) = read_content_asset(&written).expect("readable");
    assert_eq!(header.asset_id, id);
    assert_ne!(
        id,
        AssetId::from_path(address),
        "identity is minted, not derived from where the file sits"
    );
    assert_eq!(header.kind, "Widget");
    assert_eq!(
        header.references,
        vec![AssetId::from_path("content/parts/spoke.gasset")],
        "the header carries the value's outbound references"
    );
    assert_eq!(bincode::deserialize::<Widget>(payload).unwrap(), widget);
    assert_eq!(
        registry.get(id).map(|entry| entry.address.as_str()),
        Some(address)
    );
    assert_eq!(
        registry.get(existing).map(|entry| entry.address.as_str()),
        Some("content/other.gasset")
    );
    let entry = registry.get(id).unwrap().clone();
    assert_eq!(
        read_editor_only::<Widget>(&project_root, &entry).unwrap(),
        Some(Blueprint {
            drawing: vec![7, 8, 9]
        }),
        "a save writes the editor-only section it is given"
    );
    let whole = read_asset_record(&project_root, &entry).unwrap();
    assert!(read_content_asset_parts(&whole)
        .unwrap()
        .editor_only
        .is_some());
    let stripped = read_asset_record_without_editor_only(&project_root, &entry).unwrap();
    let parts = read_content_asset_parts(&stripped).unwrap();
    assert_eq!(parts.editor_only, None);
    assert_eq!(parts.header.editor_only, None);
    assert_eq!(
        bincode::deserialize::<Widget>(parts.payload).unwrap(),
        widget
    );
    assert_eq!(
        save_content_asset_into(
            &Widget { spokes: 4 },
            None,
            &project_root,
            address,
            &mut registry
        )
        .unwrap(),
        id,
        "a re-save keeps the identity"
    );
    assert_eq!(
        read_content_asset_header(&project_root.join(address))
            .unwrap()
            .asset_id,
        id
    );
    assert_eq!(
        read_editor_only::<Widget>(&project_root, &entry).unwrap(),
        Some(Blueprint {
            drawing: vec![7, 8, 9]
        }),
        "a save given no section keeps the file's"
    );

    let axle = "content/axles/front.gasset";
    let id = save_content_asset_into(
        &Axle { length: 2 },
        Some(&()),
        &project_root,
        axle,
        &mut registry,
    )
    .expect("save");
    assert_eq!(
        read_content_asset_header(&project_root.join(axle))
            .unwrap()
            .kind_version,
        4,
        "a save writes the type's version"
    );
    assert_eq!(
        registry.get(id).and_then(|entry| entry.kind_version),
        Some(4)
    );
    assert_eq!(
        read_content_asset_header(&project_root.join(axle))
            .unwrap()
            .editor_only,
        None,
        "a value of zero bytes writes no section"
    );
    save_content_asset_into(
        &Axle { length: 3 },
        None,
        &project_root,
        address,
        &mut registry,
    )
    .expect("save over another kind");
    assert_eq!(
        read_content_asset_header(&project_root.join(address))
            .unwrap()
            .editor_only,
        None,
        "a section of another kind is dropped"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

fn imported_widget_header(sub_asset: &str) -> ContentAssetHeader {
    ContentAssetHeader {
        asset_id: AssetId::new(),
        references: Vec::new(),
        kind: "Widget".to_string(),
        kind_version: Widget::FORMAT_VERSION,
        provenance: Some(ImportProvenance {
            source: "assets/wheel.glb".to_string(),
            sub_asset: sub_asset.to_string(),
        }),
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    }
}

fn write_imported_widget(path: &std::path::Path, spokes: u32) -> ContentAssetHeader {
    let header = imported_widget_header("widget/0");
    let payload = bincode::serialize(&Widget { spokes }).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let blueprint = bincode::serialize(&Blueprint {
        drawing: vec![1, 2],
    })
    .unwrap();
    let file = write_content_asset_with_editor_only(&header, &payload, &blueprint).unwrap();
    std::fs::write(path, file).unwrap();
    header
}

#[test]
fn a_resave_keeps_the_id_and_the_import_provenance() {
    let project_root = std::env::temp_dir().join(format!("resave-content-{}", std::process::id()));
    let address = "content/widgets/wheel.gasset";
    let original = write_imported_widget(&project_root.join(address), 32);
    let mut registry = AssetRegistry::new();

    let id = resave_content_asset(
        &Widget { spokes: 6 },
        None,
        &project_root,
        address,
        &mut registry,
    )
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
    let entry = entry.clone();
    assert_eq!(
        read_editor_only::<Widget>(&project_root, &entry).unwrap(),
        Some(Blueprint {
            drawing: vec![1, 2]
        }),
        "a resave given no section keeps the file's"
    );
    let redrawn = Blueprint {
        drawing: vec![3, 4, 5],
    };
    resave_content_asset(
        &Widget { spokes: 6 },
        Some(&redrawn),
        &project_root,
        address,
        &mut registry,
    )
    .expect("resave");
    assert_eq!(
        read_editor_only::<Widget>(&project_root, &entry).unwrap(),
        Some(redrawn),
        "a resave given a section replaces the file's"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn a_file_with_embedded_assets_is_not_overwritten() {
    let project_root =
        std::env::temp_dir().join(format!("save-content-host-{}", std::process::id()));
    let address = "content/wheel/scene.gasset";
    let payload = bincode::serialize(&Widget { spokes: 32 }).unwrap();
    let host = write_content_asset_with_embedded(
        &imported_widget_header("scene"),
        &payload,
        &[EmbeddedPayload {
            header: imported_widget_header("widget/0"),
            payload: &payload,
            editor_only: None,
            derived_blob: None,
        }],
    )
    .unwrap();
    let path = project_root.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &host).unwrap();
    let mut registry = AssetRegistry::new();
    let widget = Widget { spokes: 6 };

    let refusals = [
        save_content_asset_into(&widget, None, &project_root, address, &mut registry),
        resave_content_asset(&widget, None, &project_root, address, &mut registry),
    ];

    for refusal in refusals {
        assert!(refusal.is_err());
    }
    assert_eq!(std::fs::read(&path).unwrap(), host);
    assert!(registry.id_for_address(address).is_none());

    std::fs::remove_dir_all(&project_root).ok();
}
