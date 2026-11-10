use std::path::PathBuf;

use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

#[derive(Asset, Serialize, Deserialize)]
#[asset(version = 2)]
struct Gear {
    teeth: u32,
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
    panic: &'static [&'static str],
}

#[test]
fn check_encoding_passes_only_on_identical_bytes_and_says_how_to_fix_a_mismatch() {
    let cases = [
        Check {
            name: "matching bytes",
            fixture: Some(Gear { teeth: 12 }),
            panic: &[],
        },
        Check {
            name: "no fixture",
            fixture: None,
            panic: &[
                "no encoding fixture for Gear version 2 at ",
                "Gear.v2.bin; run this test with CONCERTO_BLESS_ENCODING=1 to write it",
            ],
        },
        Check {
            name: "differing bytes",
            fixture: Some(Gear { teeth: 13 }),
            panic: &[
                "the encoding of Gear changed but its FORMAT_VERSION is still 2: raise it to 3, add a migration from 2 or accept that version-2 files are rejected (docs/asset-importing.md, \"Changing an asset type's encoding\"), then run with CONCERTO_BLESS_ENCODING=1",
            ],
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
        let result = std::panic::catch_unwind(|| check_encoding(&Gear { teeth: 12 }, &dir));
        std::fs::remove_dir_all(&dir).ok();
        match result {
            Ok(()) => assert!(case.panic.is_empty(), "{} passed", case.name),
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap();
                assert!(!case.panic.is_empty(), "{}: {message}", case.name);
                for fragment in case.panic {
                    assert!(message.contains(fragment), "{}: {message}", case.name);
                }
            }
        }
    }
}
