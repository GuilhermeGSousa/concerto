//! The on-disk framing for game-ready content assets: magic, a
//! length-prefixed bincode header, the payload verbatim, then an optional derived blob.
use concerto_foundation::assets::content::{
    read_content_asset, read_content_asset_header, read_content_asset_parts, write_content_asset,
    write_cooked_content_asset, ContentAssetHeader, CookedSection, DerivedSection,
    ImportProvenance, CONTENT_ASSET_MAGIC,
};
use concerto_foundation::assets::AssetId;
use proptest::prelude::*;
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
struct HeaderV6<'a> {
    format_version: u32,
    header: &'a ContentAssetHeader,
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

prop_compose! {
    fn any_provenance()(source in "[a-z/.]{0,16}", sub_asset in "[a-z/]{0,16}") -> ImportProvenance {
        ImportProvenance { source, sub_asset }
    }
}

prop_compose! {
    fn any_header()(
        id in any::<[u8; 16]>(),
        references in proptest::collection::vec(any::<[u8; 16]>(), 0..4),
        kind in "[A-Za-z]{1,12}",
        kind_version in any::<u32>(),
        provenance in proptest::option::of(any_provenance()),
    ) -> ContentAssetHeader {
        ContentAssetHeader {
            asset_id: AssetId::from_bytes(id),
            references: references.into_iter().map(AssetId::from_bytes).collect(),
            kind,
            kind_version,
            provenance,
            derived: None,
        }
    }
}

proptest! {
    #[test]
    fn a_written_asset_reads_back_its_header_payload_and_derived_blob(
        header in any_header(),
        payload in proptest::collection::vec(any::<u8>(), 0..64),
        blob in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
        derived_version in any::<u32>(),
        key in any::<u64>(),
    ) {
        let bytes = match &blob {
            Some(blob) => write_cooked_content_asset(
                &header,
                &payload,
                CookedSection { derived_version, key, blob },
            ),
            None => write_content_asset(&header, &payload),
        }
        .unwrap();
        let expected = ContentAssetHeader {
            derived: blob.as_ref().map(|blob| DerivedSection {
                derived_version,
                key,
                len: blob.len() as u64,
            }),
            ..header
        };

        prop_assert_eq!(&bytes[..4], &CONTENT_ASSET_MAGIC);
        let parts = read_content_asset_parts(&bytes).unwrap();
        prop_assert_eq!(&parts.header, &expected);
        prop_assert_eq!(parts.payload, &payload[..]);
        prop_assert_eq!(parts.derived_blob, blob.as_deref());
        let read = read_content_asset(&bytes).unwrap();
        prop_assert_eq!(read.0, expected);
        prop_assert_eq!(read.1, &payload[..]);
    }
}

struct Read {
    name: &'static str,
    bytes: Vec<u8>,
    expected: Result<ContentAssetHeader, Vec<String>>,
    header_from_file: bool,
}

fn refused(fragments: &[&str]) -> Result<ContentAssetHeader, Vec<String>> {
    Err(fragments
        .iter()
        .map(|fragment| fragment.to_string())
        .collect())
}

#[test]
fn each_reader_reads_versions_4_to_6_and_refuses_anything_else_saying_why() {
    let v4 = version_4_header();
    let v5 = HeaderV5 {
        format_version: 5,
        asset_id: AssetId::from_path("content/hero/body.gasset"),
        references: vec![AssetId::from_path("content/hero/skin.gasset")],
        kind: "Mesh".to_string(),
        kind_version: 3,
        provenance: None,
    };
    let written = write_content_asset(&header(), b"payload").unwrap();
    let mut no_magic = written.clone();
    no_magic[0] = b'X';
    let oversized = ContentAssetHeader {
        derived: Some(DerivedSection {
            derived_version: 1,
            key: 7,
            len: 1000,
        }),
        ..header()
    };
    let version = |format_version: u32| {
        let mut old = version_4_header();
        old.format_version = format_version;
        framed(&old, b"payload")
    };
    let cases = [
        Read {
            name: "a version 6 cooked file",
            bytes: write_cooked_content_asset(&header(), b"payload", section(b"blob")).unwrap(),
            expected: Ok(ContentAssetHeader {
                derived: Some(DerivedSection {
                    derived_version: 2,
                    key: 0x0123_4567_89ab_cdef,
                    len: 4,
                }),
                ..header()
            }),
            header_from_file: true,
        },
        Read {
            name: "a version 5 header has no derived section",
            bytes: framed(&v5, b"payload"),
            expected: Ok(ContentAssetHeader {
                asset_id: v5.asset_id,
                references: v5.references.clone(),
                kind: v5.kind.clone(),
                kind_version: 3,
                provenance: None,
                derived: None,
            }),
            header_from_file: true,
        },
        Read {
            name: "a version 4 header reads as kind version 1",
            bytes: framed(&v4, b"payload"),
            expected: Ok(ContentAssetHeader {
                asset_id: v4.asset_id,
                references: v4.references.clone(),
                kind: v4.kind.clone(),
                kind_version: 1,
                provenance: v4.provenance.clone(),
                derived: None,
            }),
            header_from_file: true,
        },
        Read {
            name: "version 3",
            bytes: version(3),
            expected: refused(&["version 3", "reads versions 4 to 6"]),
            header_from_file: true,
        },
        Read {
            name: "version 7",
            bytes: version(7),
            expected: refused(&["version 7", "reads versions 4 to 6"]),
            header_from_file: true,
        },
        Read {
            name: "no magic",
            bytes: no_magic,
            expected: refused(&["GRDY"]),
            header_from_file: false,
        },
        Read {
            name: "a raw blob",
            bytes: b"\x01\x02\x03".to_vec(),
            expected: refused(&[]),
            header_from_file: false,
        },
        Read {
            name: "nothing",
            bytes: Vec::new(),
            expected: refused(&[]),
            header_from_file: false,
        },
        Read {
            name: "a truncated header",
            bytes: written[..written.len() - 10].to_vec(),
            expected: refused(&["truncated"]),
            header_from_file: false,
        },
        Read {
            name: "a section longer than the file",
            bytes: framed(
                &HeaderV6 {
                    format_version: 6,
                    header: &oversized,
                },
                b"payload",
            ),
            expected: refused(&["content asset truncated: derived section claims 1000 bytes"]),
            header_from_file: false,
        },
    ];
    for case in cases {
        let mut outcomes = vec![
            read_content_asset(&case.bytes).map(|read| {
                assert_eq!(read.1, b"payload", "{}", case.name);
                read.0
            }),
            read_content_asset_parts(&case.bytes).map(|parts| {
                assert_eq!(parts.payload, b"payload", "{}", case.name);
                parts.header
            }),
        ];
        if case.header_from_file {
            let path = written_to_disk("read", &case.bytes);
            outcomes.push(read_content_asset_header(&path));
            std::fs::remove_file(&path).ok();
        }
        for outcome in outcomes {
            match &case.expected {
                Ok(expected) => assert_eq!(&outcome.unwrap(), expected, "{}", case.name),
                Err(fragments) => {
                    let message = format!("{:#}", outcome.expect_err(case.name));
                    for fragment in fragments {
                        assert!(
                            message.contains(fragment.as_str()),
                            "{}: {message}",
                            case.name
                        );
                    }
                }
            }
        }
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
