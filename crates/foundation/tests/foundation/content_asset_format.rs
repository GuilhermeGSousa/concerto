//! The on-disk framing for game-ready content assets: magic, a
//! length-prefixed bincode header, the payload verbatim, an optional derived blob, the embedded assets, then the uncooked sections.
use concerto_foundation::assets::content::{
    asset_record, read_content_asset, read_content_asset_header, read_content_asset_parts,
    read_content_without_uncooked, read_uncooked_section, without_uncooked, write_content_asset,
    write_content_asset_with_embedded, write_content_asset_with_uncooked,
    write_cooked_content_asset, ContentAssetHeader, CookedSection, DerivedSection, EmbeddedAsset,
    EmbeddedPayload, ImportProvenance, ImportRecord, SourceFile, UncookedBytes, UncookedSection,
    CONTENT_ASSET_MAGIC, CONTENT_FORMAT_VERSION,
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
        import: None,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    }
}

fn section(blob: &[u8]) -> CookedSection<'_> {
    CookedSection {
        derived_version: 2,
        key: 0x0123_4567_89ab_cdef,
        blob,
    }
}

const SECTION_VERSION: u32 = 7;

fn described(uncooked: &Option<Vec<u8>>) -> Option<UncookedSection> {
    uncooked.as_ref().map(|bytes| UncookedSection {
        len: bytes.len() as u64,
        version: SECTION_VERSION,
    })
}

fn written_loose(
    header: &ContentAssetHeader,
    payload: &[u8],
    uncooked: &Option<Vec<u8>>,
) -> Vec<u8> {
    match uncooked {
        Some(bytes) => write_content_asset_with_uncooked(header, payload, SECTION_VERSION, bytes),
        None => write_content_asset(header, payload),
    }
    .unwrap()
}

#[derive(Serialize)]
struct AtVersion<'a> {
    format_version: u32,
    header: &'a ContentAssetHeader,
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
    fn any_source_file()(path in "[a-z/.]{0,16}", hash in any::<u64>()) -> SourceFile {
        SourceFile { path, hash }
    }
}

prop_compose! {
    fn any_import()(
        importer in "[a-z]{0,8}",
        settings_version in any::<u32>(),
        settings in proptest::collection::vec(any::<u8>(), 0..16),
        source in any_source_file(),
        dependencies in proptest::collection::vec(any_source_file(), 0..3),
    ) -> ImportRecord {
        ImportRecord { importer, settings_version, settings, source, dependencies }
    }
}

prop_compose! {
    fn any_header()(
        id in any::<[u8; 16]>(),
        references in proptest::collection::vec(any::<[u8; 16]>(), 0..4),
        kind in "[A-Za-z]{1,12}",
        kind_version in any::<u32>(),
        provenance in proptest::option::of(any_provenance()),
        import in proptest::option::of(any_import()),
    ) -> ContentAssetHeader {
        ContentAssetHeader {
            asset_id: AssetId::from_bytes(id),
            references: references.into_iter().map(AssetId::from_bytes).collect(),
            kind,
            kind_version,
            provenance,
            import,
            uncooked: None,
            derived: None,
            embedded: Vec::new(),
        }
    }
}

proptest! {
    #[test]
    fn a_written_asset_reads_back_its_header_payload_and_derived_blob(
        header in any_header(),
        payload in proptest::collection::vec(any::<u8>(), 0..64),
        uncooked in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
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
            None => Ok(written_loose(&header, &payload, &uncooked)),
        }
        .unwrap();
        let uncooked = uncooked.filter(|_| blob.is_none());
        let expected = ContentAssetHeader {
            uncooked: described(&uncooked),
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
        prop_assert_eq!(parts.uncooked, uncooked.as_deref());
        prop_assert_eq!(parts.derived_blob, blob.as_deref());
        let read = read_content_asset(&bytes).unwrap();
        prop_assert_eq!(read.0, expected);
        prop_assert_eq!(read.1, &payload[..]);
        prop_assert_eq!(asset_record(&bytes, None).unwrap(), bytes);
    }
}

#[derive(Debug)]
struct StoredAsset {
    payload: Vec<u8>,
    uncooked: Option<Vec<u8>>,
    blob: Option<Vec<u8>>,
    derived_version: u32,
    key: u64,
    kind_version: u32,
}

impl StoredAsset {
    fn header(&self, index: usize) -> ContentAssetHeader {
        ContentAssetHeader {
            asset_id: AssetId::from_bytes([index as u8 + 1; 16]),
            references: vec![AssetId::from_bytes([0x5a; 16])],
            kind: "Mesh".to_string(),
            kind_version: self.kind_version,
            provenance: Some(ImportProvenance {
                source: "assets/hero.glb".to_string(),
                sub_asset: format!("mesh/{index}"),
            }),
            import: None,
            uncooked: described(&self.uncooked),
            derived: self.blob.as_ref().map(|blob| DerivedSection {
                derived_version: self.derived_version,
                key: self.key,
                len: blob.len() as u64,
            }),
            embedded: Vec::new(),
        }
    }

    fn embedded(&self, index: usize) -> EmbeddedPayload<'_> {
        EmbeddedPayload {
            header: self.header(index),
            payload: &self.payload,
            uncooked: self.uncooked.as_deref(),
            derived_blob: self.blob.as_deref(),
        }
    }

    fn without_uncooked(&self, index: usize) -> EmbeddedPayload<'_> {
        EmbeddedPayload {
            header: ContentAssetHeader {
                uncooked: None,
                ..self.header(index)
            },
            uncooked: None,
            ..self.embedded(index)
        }
    }
}

fn embedded(stored: &[StoredAsset]) -> Vec<EmbeddedPayload<'_>> {
    stored
        .iter()
        .enumerate()
        .map(|(index, asset)| asset.embedded(index))
        .collect()
}

prop_compose! {
    fn any_stored_asset()(
        payload in proptest::collection::vec(any::<u8>(), 0..64),
        uncooked in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
        blob in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
        derived_version in any::<u32>(),
        key in any::<u64>(),
        kind_version in any::<u32>(),
    ) -> StoredAsset {
        StoredAsset { payload, uncooked, blob, derived_version, key, kind_version }
    }
}

proptest! {
    #[test]
    fn a_file_gives_back_every_asset_it_holds(
        host in any_header(),
        host_payload in proptest::collection::vec(any::<u8>(), 0..64),
        stored in proptest::collection::vec(any_stored_asset(), 0..=5),
    ) {
        let host = ContentAssetHeader {
            asset_id: AssetId::from_bytes([0xee; 16]),
            ..host
        };
        let file =
            write_content_asset_with_embedded(&host, &host_payload, &embedded(&stored)).unwrap();

        let whole = read_content_asset_parts(&file).unwrap();
        prop_assert_eq!(whole.payload, &host_payload[..]);
        prop_assert_eq!(whole.derived_blob, None);
        prop_assert_eq!(whole.header.embedded.len(), stored.len());
        prop_assert_eq!(whole.header.record_header(None), Some(host.clone()));

        let record = asset_record(&file, None).unwrap();
        let parts = read_content_asset_parts(&record).unwrap();
        prop_assert_eq!(&parts.header, &host);
        prop_assert_eq!(parts.payload, &host_payload[..]);
        prop_assert_eq!(parts.derived_blob, None);

        for (index, asset) in stored.iter().enumerate() {
            let name = format!("mesh/{index}");
            let record = asset_record(&file, Some(&name)).unwrap();
            let parts = read_content_asset_parts(&record).unwrap();
            prop_assert_eq!(&parts.header, &asset.header(index));
            prop_assert_eq!(whole.header.record_header(Some(&name)), Some(asset.header(index)));
            prop_assert_eq!(parts.payload, &asset.payload[..]);
            prop_assert_eq!(parts.uncooked, asset.uncooked.as_deref());
            prop_assert_eq!(parts.derived_blob, asset.blob.as_deref());
        }
    }

    #[test]
    fn a_file_without_its_uncooked_sections_is_the_file_written_without_them(
        host in any_header(),
        host_payload in proptest::collection::vec(any::<u8>(), 0..64),
        host_uncooked in proptest::option::of(proptest::collection::vec(any::<u8>(), 0..32)),
        stored in proptest::collection::vec(any_stored_asset(), 0..=5),
    ) {
        let host = ContentAssetHeader {
            asset_id: AssetId::from_bytes([0xee; 16]),
            ..host
        };
        let plain: Vec<EmbeddedPayload<'_>> = stored
            .iter()
            .enumerate()
            .map(|(index, asset)| asset.without_uncooked(index))
            .collect();
        let packed =
            write_content_asset_with_embedded(&host, &host_payload, &embedded(&stored)).unwrap();
        let packed_plain = write_content_asset_with_embedded(&host, &host_payload, &plain).unwrap();
        let loose =
            written_loose(&host, &host_payload, &host_uncooked);
        let loose_plain = write_content_asset(&host, &host_payload).unwrap();

        prop_assert_eq!(&without_uncooked(&packed).unwrap(), &packed_plain);
        prop_assert_eq!(&without_uncooked(&packed_plain).unwrap(), &packed_plain);
        prop_assert_eq!(&without_uncooked(&loose).unwrap(), &loose_plain);

        let path = written_to_disk("packed", &packed);
        prop_assert_eq!(&read_content_without_uncooked(&path).unwrap(), &packed_plain);
        prop_assert_eq!(read_uncooked_section(&path, None).unwrap(), None);
        for (index, asset) in stored.iter().enumerate() {
            let name = format!("mesh/{index}");
            let expected = asset.uncooked.as_ref().map(|bytes| UncookedBytes {
                kind: "Mesh".to_string(),
                version: SECTION_VERSION,
                bytes: bytes.to_vec(),
            });
            prop_assert_eq!(read_uncooked_section(&path, Some(&name)).unwrap(), expected);
        }
        prop_assert!(read_uncooked_section(&path, Some("mesh/9")).is_err());

        std::fs::write(&path, &loose).unwrap();
        prop_assert_eq!(&read_content_without_uncooked(&path).unwrap(), &loose_plain);
        let expected = host_uncooked.as_ref().map(|bytes| UncookedBytes {
            kind: host.kind.clone(),
            version: SECTION_VERSION,
            bytes: bytes.to_vec(),
        });
        prop_assert_eq!(read_uncooked_section(&path, None).unwrap(), expected);
        std::fs::remove_file(&path).ok();
    }
}

fn listed(name: &str, id: u8, offset: u64, len: u64) -> EmbeddedAsset {
    EmbeddedAsset {
        asset_id: AssetId::from_bytes([id; 16]),
        references: Vec::new(),
        kind: "Mesh".to_string(),
        kind_version: 1,
        provenance: ImportProvenance {
            source: "assets/hero.glb".to_string(),
            sub_asset: name.to_string(),
        },
        uncooked: None,
        derived: None,
        offset,
        len,
    }
}

fn host_file(embedded: Vec<EmbeddedAsset>) -> Vec<u8> {
    host_file_with_sections(None, embedded, b"hostAAAABBBB")
}

fn host_file_with_sections(
    uncooked: Option<UncookedSection>,
    embedded: Vec<EmbeddedAsset>,
    body: &[u8],
) -> Vec<u8> {
    let header = ContentAssetHeader {
        uncooked,
        embedded,
        ..header()
    };
    framed(
        &AtVersion {
            format_version: CONTENT_FORMAT_VERSION,
            header: &header,
        },
        body,
    )
}

fn sectioned(len: u64) -> Option<UncookedSection> {
    Some(UncookedSection {
        len,
        version: SECTION_VERSION,
    })
}

fn sectioned_assets() -> Vec<EmbeddedAsset> {
    vec![
        listed("mesh/0", 1, 4, 4),
        EmbeddedAsset {
            uncooked: sectioned(3),
            ..listed("mesh/1", 2, 8, 4)
        },
    ]
}

struct Malformed {
    name: &'static str,
    bytes: Vec<u8>,
    asked: &'static str,
    the_host_reads: bool,
}

#[test]
fn a_malformed_directory_is_rejected() {
    let sound = host_file(vec![listed("mesh/0", 1, 4, 4), listed("mesh/1", 2, 8, 4)]);
    assert_eq!(
        read_content_asset(&asset_record(&sound, Some("mesh/1")).unwrap())
            .unwrap()
            .1,
        b"BBBB"
    );
    let sectioned_host =
        host_file_with_sections(sectioned(2), sectioned_assets(), b"hostAAAABBBBhsbbb");
    let parts = read_content_asset_parts(&sectioned_host).unwrap();
    assert_eq!(parts.payload, b"host");
    assert_eq!(parts.uncooked, Some(&b"hs"[..]));
    let record = asset_record(&sectioned_host, Some("mesh/1")).unwrap();
    let parts = read_content_asset_parts(&record).unwrap();
    assert_eq!(parts.payload, b"BBBB");
    assert_eq!(parts.uncooked, Some(&b"bbb"[..]));
    let record = asset_record(&sectioned_host, Some("mesh/0")).unwrap();
    assert_eq!(read_content_asset_parts(&record).unwrap().uncooked, None);
    let cases = [
        Malformed {
            name: "bytes between the last asset and the sections",
            bytes: host_file_with_sections(
                sectioned(2),
                sectioned_assets(),
                b"hostAAAABBBBzzhsbbb",
            ),
            asked: "mesh/0",
            the_host_reads: false,
        },
        Malformed {
            name: "a loose file shorter than its blob and its section",
            bytes: framed(
                &AtVersion {
                    format_version: CONTENT_FORMAT_VERSION,
                    header: &ContentAssetHeader {
                        uncooked: sectioned(4),
                        derived: Some(DerivedSection {
                            derived_version: 1,
                            key: 7,
                            len: 4,
                        }),
                        ..header()
                    },
                },
                b"7 bytes",
            ),
            asked: "mesh/0",
            the_host_reads: false,
        },
        Malformed {
            name: "a range past the end",
            bytes: host_file(vec![listed("mesh/0", 1, 4, 4), listed("mesh/1", 2, 8, 40)]),
            asked: "mesh/0",
            the_host_reads: false,
        },
        Malformed {
            name: "an uncooked section past the end",
            bytes: host_file(vec![
                listed("mesh/0", 1, 4, 4),
                EmbeddedAsset {
                    uncooked: Some(UncookedSection {
                        len: 40,
                        version: SECTION_VERSION,
                    }),
                    ..listed("mesh/1", 2, 8, 4)
                },
            ]),
            asked: "mesh/0",
            the_host_reads: false,
        },
        Malformed {
            name: "overlapping ranges",
            bytes: host_file(vec![listed("mesh/0", 1, 4, 4), listed("mesh/1", 2, 6, 4)]),
            asked: "mesh/0",
            the_host_reads: false,
        },
        Malformed {
            name: "an unknown name",
            bytes: sound.clone(),
            asked: "mesh/9",
            the_host_reads: true,
        },
        Malformed {
            name: "a truncated file",
            bytes: sound[..sound.len() - 2].to_vec(),
            asked: "mesh/0",
            the_host_reads: false,
        },
    ];
    for case in cases {
        assert!(
            asset_record(&case.bytes, Some(case.asked)).is_err(),
            "{}",
            case.name
        );
        assert_eq!(
            read_content_asset_parts(&case.bytes).is_ok(),
            case.the_host_reads,
            "{}",
            case.name
        );
        assert_eq!(
            asset_record(&case.bytes, None).is_ok(),
            case.the_host_reads,
            "{}",
            case.name
        );
        assert_eq!(
            without_uncooked(&case.bytes).is_ok(),
            case.the_host_reads,
            "{}",
            case.name
        );
        let path = written_to_disk("malformed", &case.bytes);
        assert_eq!(
            read_content_without_uncooked(&path).unwrap(),
            case.bytes,
            "{}",
            case.name
        );
        std::fs::remove_file(&path).ok();
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
fn each_reader_reads_version_9_and_refuses_anything_else() {
    let written = write_content_asset(&header(), b"payload").unwrap();
    let mut no_magic = written.clone();
    no_magic[0] = b'X';
    let version = |format_version: u32| {
        framed(
            &AtVersion {
                format_version,
                header: &header(),
            },
            b"payload",
        )
    };
    let cases = [
        Read {
            name: "a cooked file",
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
            name: "the fixture's host",
            bytes: asset_record(
                &std::fs::read(fixture_dir().join("content_asset.v9.bin")).expect("the fixture"),
                None,
            )
            .unwrap(),
            expected: Ok(fixture_host()),
            header_from_file: true,
        },
        Read {
            name: "version 8",
            bytes: version(8),
            expected: refused(&[
                "is content format 8; this build reads only 9. Import its source again, or re-create it if it was authored in the editor.",
            ]),
            header_from_file: true,
        },
        Read {
            name: "version 10",
            bytes: version(10),
            expected: refused(&["is content format 10; this build reads only 9."]),
            header_from_file: true,
        },
        Read {
            name: "no magic",
            bytes: no_magic,
            expected: refused(&["GRDY"]),
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
        if case.expected.is_ok() {
            assert_eq!(
                without_uncooked(&case.bytes).unwrap(),
                case.bytes,
                "{}",
                case.name
            );
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

fn fixture_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

fn fixture_derived() -> DerivedSection {
    DerivedSection {
        derived_version: 3,
        key: 0xfedc_ba98_7654_3210,
        len: 7,
    }
}

fn fixture_header() -> ContentAssetHeader {
    ContentAssetHeader {
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
        import: None,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    }
}

fn fixture_host() -> ContentAssetHeader {
    ContentAssetHeader {
        kind: "Scene".to_string(),
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".to_string(),
            sub_asset: "scene".to_string(),
        }),
        import: Some(ImportRecord {
            importer: "gltf".to_string(),
            settings_version: 1,
            settings: vec![1, 2, 3],
            source: SourceFile {
                path: "assets/hero.glb".to_string(),
                hash: 0x0123_4567_89ab_cdef,
            },
            dependencies: vec![SourceFile {
                path: "../shared/hero.png".to_string(),
                hash: 0xfedc_ba98_7654_3210,
            }],
        }),
        ..fixture_header()
    }
}

#[test]
fn the_framing_matches_its_fixture() {
    let dir = fixture_dir();
    let mesh = ContentAssetHeader {
        asset_id: AssetId::from_bytes([0x5a; 16]),
        references: Vec::new(),
        kind: "Mesh".to_string(),
        kind_version: 2,
        provenance: Some(ImportProvenance {
            source: "assets/hero.glb".to_string(),
            sub_asset: "mesh/0".to_string(),
        }),
        import: None,
        uncooked: Some(UncookedSection { len: 6, version: 4 }),
        derived: Some(fixture_derived()),
        embedded: Vec::new(),
    };
    let bytes = write_content_asset_with_embedded(
        &fixture_host(),
        b"payload",
        &[EmbeddedPayload {
            header: mesh,
            payload: b"embedded",
            uncooked: Some(b"source"),
            derived_blob: Some(b"derived"),
        }],
    )
    .expect("write");

    let path = dir.join("content_asset.v9.bin");
    if std::env::var("CONCERTO_BLESS_ENCODING").as_deref() == Ok("1") {
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|error| panic!("failed to create '{}': {error}", dir.display()));
        std::fs::write(&path, &bytes)
            .unwrap_or_else(|error| panic!("failed to write '{}': {error}", path.display()));
        return;
    }
    let Ok(fixture) = std::fs::read(&path) else {
        panic!(
            "no encoding fixture for the content asset framing version 9 at {}; run this test with CONCERTO_BLESS_ENCODING=1 to write it",
            path.display()
        );
    };
    assert!(
        fixture == bytes,
        "the content asset framing changed but CONTENT_FORMAT_VERSION is still 9: raise it to 10, then run with CONCERTO_BLESS_ENCODING=1"
    );
}
