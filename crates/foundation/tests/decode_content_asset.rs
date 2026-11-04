use concerto_foundation::assets::content::{
    decode_content_asset, write_content_asset, ContentAssetHeader,
};
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

#[derive(Asset, Serialize, Deserialize, PartialEq, Debug)]
struct Gear {
    teeth: u32,
}

#[derive(Asset, Serialize, Deserialize, PartialEq, Debug)]
#[asset(version = 3)]
struct Spring {
    coils: u32,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Wheel {
    spokes: u32,
    rim: u32,
}

#[derive(Deserialize)]
struct WheelV1 {
    spokes: u32,
}

impl Asset for Wheel {
    const FORMAT_VERSION: u32 = 2;

    fn name() -> &'static str {
        "Wheel"
    }

    fn migrate(version: u32, payload: &[u8]) -> anyhow::Result<Self> {
        match version {
            1 => {
                let old: WheelV1 = bincode::deserialize(payload)?;
                Ok(Wheel {
                    spokes: old.spokes,
                    rim: 0,
                })
            }
            _ => anyhow::bail!("no migration from {version}"),
        }
    }
}

fn content_asset(kind: &str, kind_version: u32, payload: &[u8]) -> Vec<u8> {
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path("content/parts/thing.gasset"),
        references: Vec::new(),
        kind: kind.to_string(),
        kind_version,
        provenance: None,
    };
    write_content_asset(&header, payload).unwrap()
}

#[test]
fn the_derive_defaults_to_version_one_and_reads_the_attribute() {
    assert_eq!(Gear::FORMAT_VERSION, 1);
    assert_eq!(Spring::FORMAT_VERSION, 3);
}

#[test]
fn an_equal_version_decodes_the_payload() {
    let bytes = content_asset("Gear", 1, &bincode::serialize(&Gear { teeth: 12 }).unwrap());
    let (header, gear) = decode_content_asset::<Gear>(&bytes).expect("decodes");
    assert_eq!(gear, Gear { teeth: 12 });
    assert_eq!(header.kind_version, 1);
}

#[test]
fn an_older_version_goes_through_migrate() {
    #[derive(Serialize)]
    struct Old {
        spokes: u32,
    }
    let bytes = content_asset(
        "Wheel",
        1,
        &bincode::serialize(&Old { spokes: 36 }).unwrap(),
    );
    let (_, wheel) = decode_content_asset::<Wheel>(&bytes).expect("migrates");
    assert_eq!(wheel, Wheel { spokes: 36, rim: 0 });
}

#[test]
fn an_older_version_without_a_migration_is_rejected() {
    let bytes = content_asset(
        "Spring",
        2,
        &bincode::serialize(&Spring { coils: 5 }).unwrap(),
    );
    let err = decode_content_asset::<Spring>(&bytes).expect_err("no migration");
    let message = format!("{err:#}");
    assert!(
        message.contains("Spring payload is version 2; this build's Spring is version 3"),
        "got: {message}"
    );
    assert!(
        message.contains("Spring version 2 has no migration to version 3"),
        "got: {message}"
    );
}

#[test]
fn a_newer_version_is_rejected() {
    let bytes = content_asset("Gear", 2, &bincode::serialize(&Gear { teeth: 1 }).unwrap());
    let err = decode_content_asset::<Gear>(&bytes).expect_err("newer");
    assert_eq!(
        format!("{err:#}"),
        "Gear payload is version 2, newer than this build's version 1"
    );
}

#[test]
fn a_wrong_kind_is_rejected() {
    let bytes = content_asset(
        "Spring",
        3,
        &bincode::serialize(&Spring { coils: 1 }).unwrap(),
    );
    let err = decode_content_asset::<Gear>(&bytes).expect_err("wrong kind");
    assert_eq!(
        format!("{err:#}"),
        "content asset holds a Spring but a Gear was requested"
    );
}
