//! load_content_asset_bytes reads a content asset at <root>/<address>.
use concerto_core::assets::content::{
    write_content_asset, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};
use concerto_core::assets::utils::load_content_asset_bytes;
use concerto_core::assets::{AssetId, ContentAssetRoot};
use serde::{Deserialize, Serialize};

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("content-first-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reads_a_content_asset_at_its_literal_address() {
    let dir = temp_root("reads");
    let address = "content/x/mesh_0.gasset";
    let id = AssetId::from_path(address);

    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/x")).unwrap();
    std::fs::write(
        dir.join(address),
        write_content_asset(&header, b"payload").unwrap(),
    )
    .unwrap();

    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
        "Mesh",
    ))
    .expect("load");
    assert_eq!(bytes, b"payload", "the payload comes back header-stripped");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_kind_mismatch_is_an_error() {
    let dir = temp_root("kind");
    let address = "content/x/thing.gasset";
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::from_path(address),
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/x")).unwrap();
    std::fs::write(
        dir.join(address),
        write_content_asset(&header, b"payload").unwrap(),
    )
    .unwrap();

    let err = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
        "Scene",
    ))
    .expect_err("kind mismatch must be an error");
    let message = format!("{err:#}");
    assert!(
        message.contains("Mesh") && message.contains("Scene"),
        "got: {message}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_content_asset_is_an_error() {
    let dir = temp_root("missing");

    let err = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(dir.clone()),
        "content/x/absent.gasset",
        "Mesh",
    ))
    .expect_err("a missing content asset must be an error, not empty bytes");
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

    let err = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
        "Mesh",
    ))
    .expect_err("a directory at the joined path is not a readable content asset");
    assert!(
        format!("{err:#}").contains(address),
        "the error must name the address"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct StandInMesh {
    vertices: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

#[test]
fn a_cooked_payload_deserializes_as_its_asset_type() {
    // AssetServer performs this same validated read followed by bincode
    // deserialization. A stand-in avoids making `concerto-core` depend on `concerto-mesh`.
    let dir = temp_root("cooked-payload");
    let address = "content/x/m.gasset";
    let id = AssetId::from_path(address);
    let mesh = StandInMesh {
        vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
    };

    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/x")).unwrap();
    let payload = bincode::serialize(&mesh).unwrap();
    std::fs::write(
        dir.join(address),
        write_content_asset(&header, &payload).unwrap(),
    )
    .unwrap();

    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(dir.clone()),
        address,
        "Mesh",
    ))
    .expect("the content asset payload is readable");
    let decoded: StandInMesh = bincode::deserialize(&bytes).expect("payload is the mesh type");
    assert_eq!(
        decoded, mesh,
        "the cooked payload decodes as its declared asset type"
    );

    std::fs::remove_dir_all(&dir).ok();
}
