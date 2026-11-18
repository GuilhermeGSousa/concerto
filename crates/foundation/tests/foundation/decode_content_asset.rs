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
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    write_content_asset(&header, payload).unwrap()
}

struct Decode {
    name: &'static str,
    bytes: Vec<u8>,
    decode: fn(&[u8]) -> Result<String, String>,
    expected: Option<String>,
}

fn decoded<A: Asset + std::fmt::Debug>(bytes: &[u8]) -> Result<String, String> {
    decode_content_asset::<A>(bytes)
        .map(|decoded| format!("{:?}", decoded.1))
        .map_err(|err| format!("{err:#}"))
}

#[test]
fn a_payload_decodes_at_its_version_migrates_from_an_older_one_and_is_refused_otherwise() {
    #[derive(Serialize)]
    struct Old {
        spokes: u32,
    }
    let cases = [
        Decode {
            name: "the version the derive's attribute names",
            bytes: content_asset(
                "Spring",
                3,
                &bincode::serialize(&Spring { coils: 5 }).unwrap(),
            ),
            decode: decoded::<Spring>,
            expected: Some(format!("{:?}", Spring { coils: 5 })),
        },
        Decode {
            name: "an older version goes through migrate",
            bytes: content_asset(
                "Wheel",
                1,
                &bincode::serialize(&Old { spokes: 36 }).unwrap(),
            ),
            decode: decoded::<Wheel>,
            expected: Some(format!("{:?}", Wheel { spokes: 36, rim: 0 })),
        },
        Decode {
            name: "an older version without a migration",
            bytes: content_asset(
                "Spring",
                2,
                &bincode::serialize(&Spring { coils: 5 }).unwrap(),
            ),
            decode: decoded::<Spring>,
            expected: None,
        },
        Decode {
            name: "a newer version",
            bytes: content_asset("Gear", 2, &bincode::serialize(&Gear { teeth: 1 }).unwrap()),
            decode: decoded::<Gear>,
            expected: None,
        },
        Decode {
            name: "another kind",
            bytes: content_asset(
                "Spring",
                3,
                &bincode::serialize(&Spring { coils: 1 }).unwrap(),
            ),
            decode: decoded::<Gear>,
            expected: None,
        },
    ];
    for case in cases {
        let outcome = (case.decode)(&case.bytes);
        assert_eq!(outcome.ok(), case.expected, "{}", case.name);
    }
}
