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

fn panic_message(result: std::thread::Result<()>) -> String {
    let payload = result.expect_err("check_encoding should panic");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap()
}

#[test]
fn matching_bytes_pass() {
    let dir = fixture_dir("match");
    std::fs::write(
        dir.join("Gear.v2.bin"),
        bincode::serialize(&Gear { teeth: 12 }).unwrap(),
    )
    .unwrap();

    check_encoding(&Gear { teeth: 12 }, &dir);
}

#[test]
fn a_missing_fixture_panics_with_the_bless_hint() {
    let dir = fixture_dir("missing");

    let message = panic_message(std::panic::catch_unwind(|| {
        check_encoding(&Gear { teeth: 12 }, &dir)
    }));

    assert!(
        message.starts_with("no encoding fixture for Gear version 2 at "),
        "{message}"
    );
    assert!(message.contains("Gear.v2.bin"), "{message}");
    assert!(
        message.ends_with("; run this test with CONCERTO_BLESS_ENCODING=1 to write it"),
        "{message}"
    );
}

#[test]
fn differing_bytes_panic_asking_for_a_version_bump() {
    let dir = fixture_dir("differ");
    std::fs::write(
        dir.join("Gear.v2.bin"),
        bincode::serialize(&Gear { teeth: 13 }).unwrap(),
    )
    .unwrap();

    let message = panic_message(std::panic::catch_unwind(|| {
        check_encoding(&Gear { teeth: 12 }, &dir)
    }));

    assert_eq!(
        message,
        "the encoding of Gear changed but its FORMAT_VERSION is still 2: raise it to 3, add a migration from 2 or accept that version-2 files are rejected (docs/asset-importing.md, \"Changing an asset type's encoding\"), then run with CONCERTO_BLESS_ENCODING=1"
    );
}
