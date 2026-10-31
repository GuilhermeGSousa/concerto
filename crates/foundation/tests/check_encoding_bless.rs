use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::{Asset, AssetId};
use serde::{Deserialize, Serialize};

#[derive(Asset, Serialize, Deserialize)]
#[asset(version = 2)]
struct Gear {
    teeth: u32,
}

#[test]
fn blessing_writes_the_fixture() {
    let dir = std::env::temp_dir().join(format!(
        "check-encoding-bless-{}",
        AssetId::new().simple_hex()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Gear.v2.bin"), b"stale").unwrap();
    std::env::set_var("CONCERTO_BLESS_ENCODING", "1");

    check_encoding(&Gear { teeth: 12 }, &dir);

    assert_eq!(
        std::fs::read(dir.join("Gear.v2.bin")).unwrap(),
        bincode::serialize(&Gear { teeth: 12 }).unwrap()
    );
}
