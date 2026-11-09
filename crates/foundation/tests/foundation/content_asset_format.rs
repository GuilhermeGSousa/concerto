//! The on-disk framing for game-ready content assets: magic, a
//! length-prefixed bincode header, the payload verbatim, then an optional derived blob.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, read_content_asset_parts, write_content_asset,
    write_cooked_content_asset, ContentAssetHeader, CookedSection, DerivedSection,
    ImportProvenance, CONTENT_ASSET_MAGIC,
};
use concerto_foundation::assets::AssetId;
use serde::Serialize;
use std::path::Path;

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
        derived: None,
    }
}

fn section(blob: &[u8]) -> CookedSection<'_> {
    CookedSection {
        derived_version: 2,
        key: 0x0123_4567_89ab_cdef,
        blob,
    }
}

#[derive(Serialize)]
struct HeaderV5 {
    format_version: u32,
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    kind_version: u32,
    provenance: Option<ImportProvenance>,
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
fn writes_header_version_6_after_the_length() {
    let bytes = write_content_asset(&header(), b"payload").expect("write");
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(version, 6);
}

#[test]
fn a_cooked_file_round_trips_header_payload_and_blob() {
    let payload: Vec<u8> = (0u8..=255).collect();
    let blob = b"derived blob bytes";
    let bytes = write_cooked_content_asset(&header(), &payload, section(blob)).expect("write");

    let parts = read_content_asset_parts(&bytes).expect("read");
    let expected = ContentAssetHeader {
        derived: Some(DerivedSection {
            derived_version: 2,
            key: 0x0123_4567_89ab_cdef,
            len: blob.len() as u64,
        }),
        ..header()
    };
    assert_eq!(parts.header, expected);
    assert_eq!(parts.payload, &payload[..]);
    assert_eq!(parts.derived_blob, Some(&blob[..]));
}

#[test]
fn read_content_asset_leaves_the_derived_blob_out_of_the_payload() {
    let bytes = write_cooked_content_asset(&header(), b"payload", section(b"blob")).expect("write");

    let (decoded, payload) = read_content_asset(&bytes).expect("read");
    assert_eq!(payload, b"payload");
    assert!(decoded.derived.is_some());
}

#[test]
fn a_file_without_a_section_has_no_derived_blob() {
    let bytes = write_content_asset(&header(), b"payload").expect("write");

    let parts = read_content_asset_parts(&bytes).expect("read");
    assert_eq!(parts.header, header());
    assert_eq!(parts.payload, b"payload");
    assert_eq!(parts.derived_blob, None);
}

#[test]
fn a_version_5_header_reads_with_no_derived_section() {
    let old = HeaderV5 {
        format_version: 5,
        asset_id: AssetId::from_path("content/hero/body.gasset"),
        references: vec![AssetId::from_path("content/hero/skin.gasset")],
        kind: "Mesh".to_string(),
        kind_version: 3,
        provenance: None,
    };
    let bytes = framed(&old, b"payload");
    let expected = ContentAssetHeader {
        asset_id: old.asset_id,
        references: old.references.clone(),
        kind: old.kind.clone(),
        kind_version: 3,
        provenance: None,
        derived: None,
    };

    let parts = read_content_asset_parts(&bytes).expect("read");
    assert_eq!(parts.header, expected);
    assert_eq!(parts.payload, b"payload");
    assert_eq!(parts.derived_blob, None);

    let path = written_to_disk("v5", &bytes);
    let from_file = read_content_asset_header(&path);
    std::fs::remove_file(&path).ok();
    assert_eq!(from_file.expect("read header"), expected);
}

#[test]
fn rejects_a_section_longer_than_the_file() {
    let oversized = ContentAssetHeader {
        derived: Some(DerivedSection {
            derived_version: 1,
            key: 7,
            len: 1000,
        }),
        ..header()
    };
    let bytes = framed(&(6u32, &oversized), b"payload");

    for message in [
        format!(
            "{:#}",
            read_content_asset_parts(&bytes).expect_err("must reject")
        ),
        format!("{:#}", read_content_asset(&bytes).expect_err("must reject")),
    ] {
        assert!(
            message.contains("content asset truncated: derived section claims 1000 bytes"),
            "error should name the claimed section length, got: {message}"
        );
    }
}

#[test]
fn write_content_asset_refuses_a_header_with_a_derived_section() {
    let cooked = ContentAssetHeader {
        derived: Some(DerivedSection {
            derived_version: 1,
            key: 7,
            len: 4,
        }),
        ..header()
    };

    assert!(write_content_asset(&cooked, b"payload").is_err());
}

#[test]
fn read_content_asset_header_reads_a_cooked_header() {
    let bytes = write_cooked_content_asset(&header(), b"payload", section(b"blob")).expect("write");

    let path = written_to_disk("v6", &bytes);
    let from_file = read_content_asset_header(&path);
    std::fs::remove_file(&path).ok();
    assert_eq!(
        from_file.expect("read header"),
        ContentAssetHeader {
            derived: Some(DerivedSection {
                derived_version: 2,
                key: 0x0123_4567_89ab_cdef,
                len: 4,
            }),
            ..header()
        }
    );
}

#[test]
fn the_cooked_framing_matches_its_fixture() {
    let header = ContentAssetHeader {
        asset_id: AssetId::from_bytes([
            0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed,
            0xfe, 0x0f,
        ]),
        references: vec![AssetId::from_bytes([0x5a; 16])],
        kind: "MotionDatabase".to_string(),
        kind_version: 4,
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".to_string(),
            sub_asset: "anim/idle".to_string(),
        }),
        derived: None,
    };
    let bytes = write_cooked_content_asset(
        &header,
        b"payload",
        CookedSection {
            derived_version: 3,
            key: 0xfedc_ba98_7654_3210,
            blob: b"derived",
        },
    )
    .expect("write");

    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"));
    let path = dir.join("content_asset.v6.bin");
    if std::env::var("CONCERTO_BLESS_ENCODING").as_deref() == Ok("1") {
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|error| panic!("failed to create '{}': {error}", dir.display()));
        std::fs::write(&path, &bytes)
            .unwrap_or_else(|error| panic!("failed to write '{}': {error}", path.display()));
        return;
    }
    let Ok(fixture) = std::fs::read(&path) else {
        panic!(
            "no encoding fixture for the content asset framing version 6 at {}; run this test with CONCERTO_BLESS_ENCODING=1 to write it",
            path.display()
        );
    };
    assert!(
        fixture == bytes,
        "the content asset framing changed but CONTENT_FORMAT_VERSION is still 6: raise it to 7 and read version 6 files as before, then run with CONCERTO_BLESS_ENCODING=1"
    );
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
        derived: None,
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
fn rejects_versions_3_and_7_through_both_readers() {
    for version in [3u32, 7] {
        let mut old = version_4_header();
        old.format_version = version;
        let bytes = framed(&old, b"payload");

        let err = read_content_asset(&bytes).expect_err("must reject");
        let message = format!("{err:#}");
        assert!(
            message.contains(&format!("version {version}"))
                && message.contains("reads versions 4 to 6"),
            "error should name version {version} and the readable versions, got: {message}"
        );

        let path = written_to_disk(&format!("v{version}"), &bytes);
        let from_file = read_content_asset_header(&path);
        std::fs::remove_file(&path).ok();
        let message = format!("{:#}", from_file.expect_err("must reject"));
        assert!(
            message.contains(&format!("version {version}"))
                && message.contains("reads versions 4 to 6"),
            "error should name version {version} and the readable versions, got: {message}"
        );
    }
}
