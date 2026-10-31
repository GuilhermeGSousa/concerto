//! load_content_asset reads a content asset at <root>/<address>.
use concerto_foundation::assets::content::{write_content_asset, ContentAssetHeader};
use concerto_foundation::assets::utils::load_content_asset;
use concerto_foundation::assets::{Asset, AssetId, ContentAssetRoot};

#[derive(Asset, serde::Serialize, serde::Deserialize, PartialEq, Debug)]
struct Pebble {
    weight: u32,
}

#[derive(Asset, serde::Serialize, serde::Deserialize, Debug)]
struct Boulder {
    weight: u32,
}

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("content-first-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_pebble(dir: &std::path::Path, address: &str) {
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path(address),
        references: Vec::new(),
        kind: Pebble::name().to_string(),
        kind_version: Pebble::FORMAT_VERSION,
        provenance: None,
    };
    let payload = bincode::serialize(&Pebble { weight: 7 }).unwrap();
    let path = dir.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, &payload).unwrap()).unwrap();
}

#[test]
fn reads_a_content_asset_at_its_literal_address() {
    let dir = temp_root("reads");
    let address = "content/x/pebble_0.gasset";
    write_pebble(&dir, address);

    let pebble = pollster::block_on(load_content_asset::<Pebble>(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
    ))
    .expect("load");
    assert_eq!(pebble, Pebble { weight: 7 });

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_kind_mismatch_is_an_error() {
    let dir = temp_root("kind");
    let address = "content/x/thing.gasset";
    write_pebble(&dir, address);

    let err = pollster::block_on(load_content_asset::<Boulder>(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
    ))
    .expect_err("kind mismatch must be an error");
    let message = format!("{err:#}");
    assert!(
        message.contains("Pebble") && message.contains("Boulder") && message.contains(address),
        "got: {message}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_content_asset_is_an_error() {
    let dir = temp_root("missing");

    let err = pollster::block_on(load_content_asset::<Pebble>(
        &ContentAssetRoot::Directory(dir.clone()),
        "content/x/absent.gasset",
    ))
    .expect_err("a missing content asset must be an error");
    assert!(
        format!("{err:#}").contains("content/x/absent.gasset"),
        "the error must name the address so a missing asset is traceable"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_directory_at_the_address_is_an_error() {
    let dir = temp_root("addr-is-dir");
    let address = "content/x";
    std::fs::create_dir_all(dir.join(address)).unwrap();

    let err = pollster::block_on(load_content_asset::<Pebble>(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
    ))
    .expect_err("a directory at the joined path is not a readable content asset");
    assert!(
        format!("{err:#}").contains(address),
        "the error must name the address"
    );

    std::fs::remove_dir_all(&dir).ok();
}
