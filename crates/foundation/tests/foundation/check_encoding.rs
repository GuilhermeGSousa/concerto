use std::path::PathBuf;

use concerto_foundation::assets::content::{check_encoding, check_uncooked_encoding};
use concerto_foundation::assets::{Asset, AssetId, UncookedData};
use serde::{Deserialize, Serialize};

#[derive(Asset, Serialize, Deserialize)]
#[asset(version = 2)]
struct Gear {
    teeth: u32,
}

#[derive(Serialize, Deserialize)]
struct GearDrawing {
    strokes: u32,
}

impl UncookedData for GearDrawing {
    type Asset = Gear;

    const VERSION: u32 = 5;
}

fn fixture_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "check-encoding-{test}-{}",
        AssetId::new().simple_hex()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Check {
    name: &'static str,
    fixture: Option<Gear>,
    panics: bool,
}

#[test]
fn check_encoding_passes_only_on_identical_bytes() {
    let cases = [
        Check {
            name: "matching bytes",
            fixture: Some(Gear { teeth: 12 }),
            panics: false,
        },
        Check {
            name: "no fixture",
            fixture: None,
            panics: true,
        },
        Check {
            name: "differing bytes",
            fixture: Some(Gear { teeth: 13 }),
            panics: true,
        },
    ];
    for case in cases {
        let dir = fixture_dir("check");
        if let Some(fixture) = &case.fixture {
            std::fs::write(
                dir.join("Gear.v2.bin"),
                bincode::serialize(fixture).unwrap(),
            )
            .unwrap();
        }
        if let Some(fixture) = &case.fixture {
            let drawing = GearDrawing {
                strokes: fixture.teeth,
            };
            std::fs::write(
                dir.join("Gear.uncooked.v5.bin"),
                bincode::serialize(&drawing).unwrap(),
            )
            .unwrap();
        }
        let result = std::panic::catch_unwind(|| check_encoding(&Gear { teeth: 12 }, &dir));
        let uncooked = std::panic::catch_unwind(|| {
            check_uncooked_encoding(&GearDrawing { strokes: 12 }, &dir)
        });
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result.is_err(), case.panics, "{}", case.name);
        assert_eq!(uncooked.is_err(), case.panics, "{}, uncooked", case.name);
    }
}
