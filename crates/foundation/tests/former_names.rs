use concerto_foundation::assets::content::{
    decode_content_asset, read_content_asset_header, save_content_asset_into, write_content_asset,
    ContentAssetHeader,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::{kind_matches, Asset, AssetId};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Lantern {
    wick: u32,
    glow: u32,
}

#[derive(Serialize, Deserialize)]
struct LampV1 {
    wick: u32,
}

impl Asset for Lantern {
    const FORMAT_VERSION: u32 = 2;
    const FORMER_NAMES: &'static [&'static str] = &["Lamp"];

    fn name() -> &'static str {
        "Lantern"
    }

    fn migrate_former(name: &str, version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        match (name, version) {
            ("Lamp", 1) => {
                let old: LampV1 = bincode::deserialize(payload)?;
                Ok(Lantern {
                    wick: old.wick,
                    glow: 0,
                })
            }
            _ => anyhow::bail!("no migration from {name} {version}"),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Torch {
    flame: u32,
}

impl Asset for Torch {
    const FORMER_NAMES: &'static [&'static str] = &["Candle"];

    fn name() -> &'static str {
        "Torch"
    }
}

fn entry(address: &str, kind: &str) -> AssetEntry {
    AssetEntry {
        id: AssetId::new(),
        address: address.into(),
        kind: kind.into(),
        kind_version: None,
        provenance: None,
    }
}

fn content_asset(id: AssetId, kind: &str, kind_version: u32, payload: &[u8]) -> Vec<u8> {
    let header = ContentAssetHeader {
        asset_id: id,
        references: Vec::new(),
        kind: kind.to_string(),
        kind_version,
        provenance: None,
        derived: None,
    };
    write_content_asset(&header, payload).unwrap()
}

fn lamp(id: AssetId) -> Vec<u8> {
    content_asset(
        id,
        "Lamp",
        1,
        &bincode::serialize(&LampV1 { wick: 4 }).unwrap(),
    )
}

#[test]
fn an_entry_under_a_former_name_holds_the_renamed_type() {
    let mut registry = AssetRegistry::new();
    let lamp = entry("content/b/lamp.gasset", "Lamp");
    let lantern = entry("content/a/lantern.gasset", "Lantern");
    let torch = entry("content/a/torch.gasset", "Torch");
    for entry in [&lamp, &lantern, &torch] {
        registry.insert(entry.clone());
    }

    assert!(kind_matches::<Lantern>("Lamp"));
    assert!(kind_matches::<Lantern>("Lantern"));
    assert!(!kind_matches::<Lantern>("Torch"));
    assert!(lamp.is::<Lantern>());
    assert!(!lamp.is::<Torch>());
    let of_type: Vec<_> = registry
        .of_type::<Lantern>()
        .map(|entry| entry.id)
        .collect();
    assert_eq!(of_type, [lantern.id, lamp.id]);
    let of_kind: Vec<_> = registry
        .of_kind(Lantern::name(), Lantern::FORMER_NAMES)
        .map(|entry| entry.id)
        .collect();
    assert_eq!(of_kind, [lantern.id, lamp.id]);
    let current_only: Vec<_> = registry
        .of_kind(Lantern::name(), &[])
        .map(|entry| entry.id)
        .collect();
    assert_eq!(current_only, [lantern.id]);
}

#[test]
fn a_file_under_a_former_name_decodes_through_migrate_former() {
    let (header, lantern) = decode_content_asset::<Lantern>(&lamp(AssetId::new())).unwrap();

    assert_eq!(header.kind, "Lamp");
    assert_eq!(lantern, Lantern { wick: 4, glow: 0 });
}

#[test]
fn a_file_under_a_former_name_is_saved_back_under_the_current_name_and_version() {
    let root = std::env::temp_dir().join(format!("former-names-save-{}", std::process::id()));
    let address = "content/lights/lamp.gasset";
    let id = AssetId::new();
    std::fs::create_dir_all(root.join("content/lights")).unwrap();
    std::fs::write(root.join(address), lamp(id)).unwrap();
    let (_, lantern) =
        decode_content_asset::<Lantern>(&std::fs::read(root.join(address)).unwrap()).unwrap();
    let mut registry = AssetRegistry::new();

    let saved = save_content_asset_into(&lantern, &root, address, &mut registry).unwrap();

    let header = read_content_asset_header(&root.join(address)).unwrap();
    assert_eq!(saved, id);
    assert_eq!(header.asset_id, id);
    assert_eq!(header.kind, "Lantern");
    assert_eq!(header.kind_version, 2);
    let entry = registry.get(id).unwrap();
    assert_eq!(entry.kind, "Lantern");
    assert_eq!(entry.kind_version, Some(2));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_former_name_without_a_migration_is_rejected() {
    let bytes = content_asset(
        AssetId::new(),
        "Candle",
        1,
        &bincode::serialize(&Torch { flame: 2 }).unwrap(),
    );

    let err = decode_content_asset::<Torch>(&bytes).expect_err("no former migration");

    let message = format!("{err:#}");
    assert!(
        message.contains("Candle is the former name of Torch; this build's Torch is version 1"),
        "got: {message}"
    );
    assert!(
        message.contains("Candle version 1 has no migration to Torch version 1"),
        "got: {message}"
    );
}
