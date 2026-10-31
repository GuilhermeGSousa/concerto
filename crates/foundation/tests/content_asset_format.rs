//! The on-disk framing for game-ready content assets: magic, a
//! length-prefixed bincode header, then the payload verbatim.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, write_content_asset, ContentAssetHeader,
    ImportProvenance, CONTENT_ASSET_MAGIC,
};
use concerto_foundation::assets::AssetId;
use serde::Serialize;

fn header() -> ContentAssetHeader {
    ContentAssetHeader {
        asset_id: AssetId::from_path("content/hero/body.gasset"),
        references: vec![
            AssetId::from_path("content/hero/body.mat.gasset"),
            AssetId::from_path("content/hero/skin.gasset"),
        ],
        kind: "Mesh".to_string(),
        kind_version: 3,
        provenance: None,
    }
}

#[derive(Serialize)]
struct HeaderV4 {
    format_version: u32,
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    provenance: Option<ImportProvenance>,
}

fn version_4_header() -> HeaderV4 {
    HeaderV4 {
        format_version: 4,
        asset_id: AssetId::from_path("content/hero/idle.gasset"),
        references: vec![AssetId::from_path("content/hero/skeleton.gasset")],
        kind: "AnimationClip".to_string(),
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".to_string(),
            sub_asset: "anim/idle".to_string(),
        }),
    }
}

fn framed(header: &impl Serialize, payload: &[u8]) -> Vec<u8> {
    let header_bytes = bincode::serialize(header).expect("serialize header");
    let mut out = Vec::new();
    out.extend_from_slice(&CONTENT_ASSET_MAGIC);
    out.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(payload);
    out
}

fn written_to_disk(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "content-asset-format-{tag}-{}.gasset",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write file");
    path
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
fn writes_header_version_5_after_the_length() {
    let bytes = write_content_asset(&header(), b"payload").expect("write");
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(version, 5);
}

#[test]
fn a_version_4_header_reads_as_kind_version_1_through_both_readers() {
    let old = version_4_header();
    let bytes = framed(&old, b"payload");
    let expected = ContentAssetHeader {
        asset_id: old.asset_id,
        references: old.references.clone(),
        kind: old.kind.clone(),
        kind_version: 1,
        provenance: old.provenance.clone(),
    };

    let (decoded, payload) = read_content_asset(&bytes).expect("read");
    assert_eq!(decoded, expected);
    assert_eq!(payload, b"payload");

    let path = written_to_disk("v4", &bytes);
    let from_file = read_content_asset_header(&path);
    std::fs::remove_file(&path).ok();
    assert_eq!(from_file.expect("read header"), expected);
}

#[test]
fn rejects_versions_3_and_6_through_both_readers() {
    for version in [3u32, 6] {
        let mut old = version_4_header();
        old.format_version = version;
        let bytes = framed(&old, b"payload");

        let err = read_content_asset(&bytes).expect_err("must reject");
        let message = format!("{err:#}");
        assert!(
            message.contains(&format!("version {version}"))
                && message.contains("reads versions 4 and 5"),
            "error should name version {version} and the readable versions, got: {message}"
        );

        let path = written_to_disk(&format!("v{version}"), &bytes);
        let from_file = read_content_asset_header(&path);
        std::fs::remove_file(&path).ok();
        let message = format!("{:#}", from_file.expect_err("must reject"));
        assert!(
            message.contains(&format!("version {version}"))
                && message.contains("reads versions 4 and 5"),
            "error should name version {version} and the readable versions, got: {message}"
        );
    }
}
