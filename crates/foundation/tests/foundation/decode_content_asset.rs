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
        derived: None,
    };
    write_content_asset(&header, payload).unwrap()
}

struct Decode {
    name: &'static str,
    bytes: Vec<u8>,
    decode: fn(&[u8]) -> Result<String, String>,
    expected: Result<String, Vec<&'static str>>,
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
            name: "the derive's default version",
            bytes: content_asset("Gear", 1, &bincode::serialize(&Gear { teeth: 12 }).unwrap()),
            decode: decoded::<Gear>,
            expected: Ok(format!("{:?}", Gear { teeth: 12 })),
        },
        Decode {
            name: "the version the derive's attribute names",
            bytes: content_asset(
                "Spring",
                3,
                &bincode::serialize(&Spring { coils: 5 }).unwrap(),
            ),
            decode: decoded::<Spring>,
            expected: Ok(format!("{:?}", Spring { coils: 5 })),
        },
        Decode {
            name: "an older version goes through migrate",
            bytes: content_asset(
                "Wheel",
                1,
                &bincode::serialize(&Old { spokes: 36 }).unwrap(),
            ),
            decode: decoded::<Wheel>,
            expected: Ok(format!("{:?}", Wheel { spokes: 36, rim: 0 })),
        },
        Decode {
            name: "an older version without a migration",
            bytes: content_asset(
                "Spring",
                2,
                &bincode::serialize(&Spring { coils: 5 }).unwrap(),
            ),
            decode: decoded::<Spring>,
            expected: Err(vec![
                "Spring payload is version 2; this build's Spring is version 3",
                "Spring version 2 has no migration to version 3",
            ]),
        },
        Decode {
            name: "a newer version",
            bytes: content_asset("Gear", 2, &bincode::serialize(&Gear { teeth: 1 }).unwrap()),
            decode: decoded::<Gear>,
            expected: Err(vec![
                "Gear payload is version 2, newer than this build's version 1",
            ]),
        },
        Decode {
            name: "another kind",
            bytes: content_asset(
                "Spring",
                3,
                &bincode::serialize(&Spring { coils: 1 }).unwrap(),
            ),
            decode: decoded::<Gear>,
            expected: Err(vec![
                "content asset holds a Spring but a Gear was requested",
            ]),
        },
    ];
    for case in cases {
        let outcome = (case.decode)(&case.bytes);
        match case.expected {
            Ok(expected) => assert_eq!(outcome, Ok(expected), "{}", case.name),
            Err(fragments) => {
                let message = outcome.expect_err(case.name);
                for fragment in fragments {
                    assert!(message.contains(fragment), "{}: {message}", case.name);
                }
            }
        }
    }
}
