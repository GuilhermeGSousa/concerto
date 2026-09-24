//! The on-disk framing for game-ready content assets: magic, a
//! length-prefixed bincode header, then the payload verbatim.
use concerto_foundation::assets::content::{
    read_content_asset, write_content_asset, ContentAssetHeader, CONTENT_ASSET_MAGIC,
    CONTENT_FORMAT_VERSION,
};
use concerto_foundation::assets::AssetId;

fn header() -> ContentAssetHeader {
    ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::from_path("content/hero/body.gasset"),
        references: vec![
            AssetId::from_path("content/hero/body.mat.gasset"),
            AssetId::from_path("content/hero/skin.gasset"),
        ],
        kind: "Mesh".to_string(),
        provenance: None,
    }
}

#[test]
fn round_trips_header_and_payload_verbatim() {
    let payload: Vec<u8> = (0u8..=255).collect();
    let bytes = write_content_asset(&header(), &payload).expect("write");

    assert_eq!(&bytes[..4], &CONTENT_ASSET_MAGIC, "magic leads the file");

    let (decoded, decoded_payload) = read_content_asset(&bytes).expect("read");
    assert_eq!(decoded, header(), "every header field survives");
    assert_eq!(decoded_payload, &payload[..], "payload is byte-identical");
}

#[test]
fn empty_payload_is_valid() {
    let bytes = write_content_asset(&header(), &[]).expect("write");
    let (_, payload) = read_content_asset(&bytes).expect("read");
    assert!(payload.is_empty());
}

#[test]
fn rejects_a_buffer_without_the_magic() {
    let bytes = write_content_asset(&header(), b"payload").expect("write");
    let mut corrupted = bytes.clone();
    corrupted[0] = b'X';

    let err = read_content_asset(&corrupted).expect_err("must reject");
    assert!(
        err.to_string().contains("GRDY"),
        "error should name the missing magic, got: {err}"
    );

    // A headerless raw blob must also be rejected, not misread.
    assert!(read_content_asset(b"\x01\x02\x03").is_err());
    assert!(read_content_asset(&[]).is_err());
}

#[test]
fn rejects_a_truncated_header() {
    let bytes = write_content_asset(&header(), b"payload").expect("write");
    let truncated = &bytes[..bytes.len() - 10];

    let err = read_content_asset(truncated).expect_err("must reject");
    assert!(
        err.to_string().contains("truncated"),
        "error should say the file is truncated, got: {err}"
    );
}

#[test]
fn rejects_an_unknown_format_version() {
    let mut future = header();
    future.format_version = CONTENT_FORMAT_VERSION + 1;
    let bytes = write_content_asset(&future, b"payload").expect("write");

    let err = read_content_asset(&bytes).expect_err("a newer format version must be rejected");
    let message = format!("{err:#}");
    assert!(
        message.contains("version") && message.contains(&(CONTENT_FORMAT_VERSION + 1).to_string()),
        "error should name the unsupported version, got: {message}"
    );
}
