//! Shared persistent asset identity and binary framing, independent of the engine.
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetId(Uuid);

/// Fixed namespace for deriving AssetIds from asset paths, so the same
/// path string always hashes to the same UUID (v5) regardless of process
/// or machine. Generated once via `uuid::Uuid::new_v4()` and hard-coded —
/// it must never change once assets have been addressed with it.
const ASSET_PATH_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6d, 0x1a, 0x9a, 0x3e, 0x2f, 0x0b, 0x4a, 0x77, 0x8e, 0x92, 0x1a, 0x64, 0xaf, 0x03, 0x5c, 0x11,
]);

impl AssetId {
    /// Constructs an identity from its UUID bytes, including in constant expressions.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// The UUID bytes embedded by compile-time asset references.
    pub const fn to_bytes(self) -> [u8; 16] {
        *self.0.as_bytes()
    }

    pub fn new() -> Self {
        AssetId(Uuid::new_v4())
    }

    /// The id that names no asset.
    pub const fn nil() -> Self {
        AssetId(Uuid::nil())
    }

    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }

    /// Derives a deterministic UUID for legacy content and callers that need
    /// a namespaced path hash. Imported asset identity is minted and persisted
    /// separately; runtime content loading must resolve it through the registry.
    pub fn from_path(path: &str) -> Self {
        AssetId(Uuid::new_v5(&ASSET_PATH_NAMESPACE, path.as_bytes()))
    }

    /// The AssetId's underlying UUID as 32 lowercase hex digits, no hyphens —
    /// used as the `AssetRegistry` key (a text-friendly, collision-free pure
    /// function of the ID).
    pub fn simple_hex(&self) -> String {
        self.0.simple().to_string()
    }

    /// Inverse of `simple_hex()` — parses an AssetId back from its
    /// 32-lowercase-hex-digit form. Used by `AssetRegistry` to reconstruct
    /// ids read back from registry TOML.
    pub fn from_simple_hex(hex: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(hex).map(AssetId)
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

/// Leading bytes of every content asset file.
pub const CONTENT_ASSET_MAGIC: [u8; 4] = *b"GRDY";

/// The version of the framing and `ContentAssetHeader`, written before the header; it changes only when they change, never for a payload.
pub const CONTENT_FORMAT_VERSION: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentAssetHeader {
    /// Identity: a UUID minted when the asset is first written and carried
    /// here from then on, so it survives the file being renamed or moved.
    /// Deliberately not derived from the address — only the asset registry
    /// connects an address back to the id.
    pub asset_id: AssetId,
    /// Outbound references, so a registry scan never reads payloads.
    pub references: Vec<AssetId>,
    /// Authoritative type tag; must equal the loading type's `Asset::name()`.
    pub kind: String,
    /// The `Asset::FORMAT_VERSION` of `kind` that the payload was written with.
    pub kind_version: u32,
    /// Where this content asset came from, if `import` produced it from a
    /// DCC source rather than an editor saving it directly.
    pub provenance: Option<ImportProvenance>,
    /// The record's uncooked bytes at the end of the file, if any.
    pub uncooked: Option<UncookedSection>,
    /// The cooked derived data trailing the payload, if any.
    pub derived: Option<DerivedSection>,
    /// The assets stored in this file after the host's payload and blob, in file order.
    pub embedded: Vec<EmbeddedAsset>,
}

impl ContentAssetHeader {
    /// The header a loose file of the host (`None`) or of the named embedded asset would have.
    pub fn record_header(&self, embedded_as: Option<&str>) -> Option<ContentAssetHeader> {
        let Some(name) = embedded_as else {
            return Some(ContentAssetHeader {
                asset_id: self.asset_id,
                references: self.references.clone(),
                kind: self.kind.clone(),
                kind_version: self.kind_version,
                provenance: self.provenance.clone(),
                uncooked: self.uncooked,
                derived: self.derived,
                embedded: Vec::new(),
            });
        };
        self.embedded
            .iter()
            .find(|asset| asset.name() == name)
            .map(EmbeddedAsset::record_header)
    }

    fn uncooked_len(&self) -> u64 {
        self.uncooked.map_or(0, |section| section.len)
    }

    /// Whether the host or any embedded asset has an uncooked section.
    pub fn has_uncooked(&self) -> bool {
        self.uncooked.is_some() || self.embedded.iter().any(|asset| asset.uncooked.is_some())
    }
}

/// An asset stored inside another asset's file: what its own header would hold, and where its payload is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedAsset {
    /// The asset's identity, as in a loose file's header.
    pub asset_id: AssetId,
    /// Outbound references, so a registry scan never reads payloads.
    pub references: Vec<AssetId>,
    /// Authoritative type tag; must equal the loading type's `Asset::name()`.
    pub kind: String,
    /// The `Asset::FORMAT_VERSION` of `kind` that the payload was written with.
    pub kind_version: u32,
    /// The import output this asset is; its `sub_asset` is the asset's name inside the file.
    pub provenance: ImportProvenance,
    /// The record's uncooked bytes at the end of the file, if any.
    pub uncooked: Option<UncookedSection>,
    /// The cooked derived data trailing the payload, if any.
    pub derived: Option<DerivedSection>,
    /// Where the payload starts, in bytes from the end of the file's header.
    pub offset: u64,
    /// The payload's length in bytes, without the derived blob that follows it.
    pub len: u64,
}

impl EmbeddedAsset {
    /// The asset's name inside its file, unique there.
    pub fn name(&self) -> &str {
        &self.provenance.sub_asset
    }

    fn record_header(&self) -> ContentAssetHeader {
        ContentAssetHeader {
            asset_id: self.asset_id,
            references: self.references.clone(),
            kind: self.kind.clone(),
            kind_version: self.kind_version,
            provenance: Some(self.provenance.clone()),
            uncooked: self.uncooked,
            derived: self.derived,
            embedded: Vec::new(),
        }
    }

    fn uncooked_len(&self) -> u64 {
        self.uncooked.map_or(0, |section| section.len)
    }
}

/// Describes a record's uncooked bytes, which sit with every other section at the end of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncookedSection {
    /// The section's length in bytes.
    pub len: u64,
    /// The `UncookedData::VERSION` the section's bytes were written with.
    pub version: u32,
}

/// One record's uncooked section read from a file, with the record's type tag and the version the section was written under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncookedBytes {
    /// The `Asset::name()` of the type the record holds.
    pub kind: String,
    /// The `UncookedData::VERSION` the section was written with.
    pub version: u32,
    /// The section's bytes.
    pub bytes: Vec<u8>,
}

/// One record's uncooked section, borrowed from its file, with the version it was written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UncookedSlice<'a> {
    /// The `UncookedData::VERSION` the section was written with.
    pub version: u32,
    /// The section's bytes.
    pub bytes: &'a [u8],
}

/// Describes the derived blob a cook appended after a content asset's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedSection {
    /// The derived type's `DERIVED_VERSION` at cook time.
    pub derived_version: u32,
    /// The `derived_key` of the inputs the blob was derived from.
    pub key: u64,
    /// The blob's length in bytes.
    pub len: u64,
}

impl DerivedSection {
    fn cooked(self, blob: &[u8]) -> CookedSection<'_> {
        CookedSection {
            derived_version: self.derived_version,
            key: self.key,
            blob,
        }
    }
}

/// The derived blob and its description, for [`write_cooked_content_asset`].
#[derive(Debug, Clone, Copy)]
pub struct CookedSection<'a> {
    /// The derived type's `DERIVED_VERSION` at cook time.
    pub derived_version: u32,
    /// The `derived_key` of the inputs the blob was derived from.
    pub key: u64,
    /// The derived bytes, written after the payload.
    pub blob: &'a [u8],
}

/// A content asset split into its header, payload and optional uncooked section and derived blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAssetParts<'a> {
    /// The decoded header.
    pub header: ContentAssetHeader,
    /// The payload, without the uncooked section and the derived blob.
    pub payload: &'a [u8],
    /// The uncooked bytes the header's `uncooked` section describes, if any.
    pub uncooked: Option<&'a [u8]>,
    /// The derived blob the header's `derived` section describes, if any.
    pub derived_blob: Option<&'a [u8]>,
}

/// One embedded asset as a loose file of it would hold it, for [`write_content_asset_with_embedded`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedPayload<'a> {
    /// The header a loose file of the asset would have; its provenance names the asset inside the file.
    pub header: ContentAssetHeader,
    /// The payload, without the uncooked section and the derived blob.
    pub payload: &'a [u8],
    /// The asset's uncooked bytes, which its header describes; `None` writes no section.
    pub uncooked: Option<&'a [u8]>,
    /// The derived blob the header's `derived` section describes, if any.
    pub derived_blob: Option<&'a [u8]>,
}

#[derive(Deserialize)]
struct HeaderV7 {
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    kind_version: u32,
    provenance: Option<ImportProvenance>,
    derived: Option<DerivedSection>,
    embedded: Vec<EmbeddedAssetV7>,
}

#[derive(Deserialize)]
struct EmbeddedAssetV7 {
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    kind_version: u32,
    provenance: ImportProvenance,
    derived: Option<DerivedSection>,
    offset: u64,
    len: u64,
}

#[derive(Deserialize)]
struct HeaderV6 {
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    kind_version: u32,
    provenance: Option<ImportProvenance>,
    derived: Option<DerivedSection>,
}

#[derive(Deserialize)]
struct HeaderV5 {
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    kind_version: u32,
    provenance: Option<ImportProvenance>,
}

#[derive(Deserialize)]
struct HeaderV4 {
    asset_id: AssetId,
    references: Vec<AssetId>,
    kind: String,
    provenance: Option<ImportProvenance>,
}

fn decode_header(bytes: &[u8]) -> anyhow::Result<ContentAssetHeader> {
    let mut rest = bytes;
    let version: u32 = bincode::deserialize_from(&mut rest)
        .context("failed to deserialize content asset header")?;
    match version {
        8 => {
            let header: ContentAssetHeader =
                bincode::deserialize(rest).context("failed to deserialize content asset header")?;
            check_directory(&header, None)?;
            Ok(header)
        }
        7 => {
            let header: HeaderV7 = bincode::deserialize(rest)
                .context("failed to deserialize version 7 content asset header")?;
            let header = ContentAssetHeader {
                asset_id: header.asset_id,
                references: header.references,
                kind: header.kind,
                kind_version: header.kind_version,
                provenance: header.provenance,
                uncooked: None,
                derived: header.derived,
                embedded: header
                    .embedded
                    .into_iter()
                    .map(|asset| EmbeddedAsset {
                        asset_id: asset.asset_id,
                        references: asset.references,
                        kind: asset.kind,
                        kind_version: asset.kind_version,
                        provenance: asset.provenance,
                        uncooked: None,
                        derived: asset.derived,
                        offset: asset.offset,
                        len: asset.len,
                    })
                    .collect(),
            };
            check_directory(&header, None)?;
            Ok(header)
        }
        6 => {
            let header: HeaderV6 = bincode::deserialize(rest)
                .context("failed to deserialize version 6 content asset header")?;
            Ok(ContentAssetHeader {
                asset_id: header.asset_id,
                references: header.references,
                kind: header.kind,
                kind_version: header.kind_version,
                provenance: header.provenance,
                uncooked: None,
                derived: header.derived,
                embedded: Vec::new(),
            })
        }
        5 => {
            let header: HeaderV5 = bincode::deserialize(rest)
                .context("failed to deserialize version 5 content asset header")?;
            Ok(ContentAssetHeader {
                asset_id: header.asset_id,
                references: header.references,
                kind: header.kind,
                kind_version: header.kind_version,
                provenance: header.provenance,
                uncooked: None,
                derived: None,
                embedded: Vec::new(),
            })
        }
        4 => {
            let header: HeaderV4 = bincode::deserialize(rest)
                .context("failed to deserialize version 4 content asset header")?;
            Ok(ContentAssetHeader {
                asset_id: header.asset_id,
                references: header.references,
                kind: header.kind,
                kind_version: 1,
                provenance: header.provenance,
                uncooked: None,
                derived: None,
                embedded: Vec::new(),
            })
        }
        other => bail!(
            "unsupported content asset format version {other} (this build reads versions 4 to 8)"
        ),
    }
}

fn check_directory(header: &ContentAssetHeader, body_len: Option<u64>) -> anyhow::Result<()> {
    let mut ids = HashSet::from([header.asset_id]);
    let mut names = HashSet::new();
    let limit = match body_len {
        Some(body_len) => Some(sections_start(header, body_len)?),
        None => None,
    };
    let mut cursor = header.derived.map_or(0, |section| section.len);
    for asset in &header.embedded {
        let name = asset.name();
        if !names.insert(name) {
            bail!("content asset directory is malformed: two embedded assets are named '{name}'");
        }
        if !ids.insert(asset.asset_id) {
            bail!(
                "content asset directory is malformed: asset id {} appears twice, the second time on '{name}'",
                asset.asset_id.simple_hex()
            );
        }
        if asset.offset < cursor {
            bail!(
                "content asset directory is malformed: embedded asset '{name}' overlaps the asset before it"
            );
        }
        let end = asset
            .offset
            .checked_add(asset.len)
            .and_then(|end| end.checked_add(asset.derived.map_or(0, |section| section.len)))
            .with_context(|| {
                format!(
                    "content asset directory is malformed: the byte range of embedded asset '{name}' overflows"
                )
            })?;
        if let Some(limit) = limit.filter(|limit| end > *limit) {
            bail!(
                "content asset truncated: embedded asset '{name}' runs to byte {end} of the {limit} that follow the header and come before the uncooked sections"
            );
        }
        cursor = end;
    }
    let gap = limit
        .filter(|_| !header.embedded.is_empty() && header.has_uncooked())
        .map_or(0, |limit| limit - cursor);
    if gap != 0 {
        bail!(
            "content asset directory is malformed: {gap} bytes lie between the last embedded asset and the uncooked sections"
        );
    }
    Ok(())
}

fn sections_start(header: &ContentAssetHeader, body_len: u64) -> anyhow::Result<u64> {
    let sections = header
        .embedded
        .iter()
        .map(EmbeddedAsset::uncooked_len)
        .try_fold(header.uncooked_len(), u64::checked_add);
    sections
        .and_then(|sections| body_len.checked_sub(sections))
        .with_context(|| {
            format!(
                "content asset truncated: its uncooked sections claim more than the {body_len} bytes that follow the header"
            )
        })
}

fn section_start(
    header: &ContentAssetHeader,
    embedded_as: Option<&str>,
    body_len: u64,
) -> anyhow::Result<u64> {
    let start = sections_start(header, body_len)?;
    let Some(name) = embedded_as else {
        return Ok(start);
    };
    let index = header
        .embedded
        .iter()
        .position(|asset| asset.name() == name)
        .with_context(|| format!("content asset holds no embedded asset named '{name}'"))?;
    let before: u64 = header.embedded[..index]
        .iter()
        .map(EmbeddedAsset::uncooked_len)
        .sum();
    Ok(start + header.uncooked_len() + before)
}

/// The offline source (and sub-asset within it) that `import` produced a
/// content asset from — lets a future editor show "re-import" provenance
/// without re-deriving it from the address string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProvenance {
    /// Project-relative source path, or an absolute path for external sources.
    /// Older assets can retain the path spelling used by the original importer.
    pub source: String,
    /// The sub-asset name within that source, e.g. `"mesh/0"`.
    pub sub_asset: String,
}

pub fn write_content_asset(header: &ContentAssetHeader, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    if header.derived.is_some() {
        bail!("a content asset header with a derived section needs its blob; write it with write_cooked_content_asset");
    }
    if header.uncooked.is_some() {
        bail!("a content asset header with an uncooked section needs its bytes; write it with write_content_asset_with_uncooked");
    }
    refuse_a_directory(header)?;
    frame_content_asset(header, &[payload])
}

/// Writes a loose file with `uncooked` as its section, encoded at `version`.
pub fn write_content_asset_with_uncooked(
    header: &ContentAssetHeader,
    payload: &[u8],
    version: u32,
    uncooked: &[u8],
) -> anyhow::Result<Vec<u8>> {
    if header.derived.is_some() {
        bail!("a content asset written with an uncooked section cannot carry a derived section");
    }
    refuse_a_directory(header)?;
    let header = ContentAssetHeader {
        uncooked: Some(UncookedSection {
            len: uncooked.len() as u64,
            version,
        }),
        ..header.clone()
    };
    frame_content_asset(&header, &[payload, uncooked])
}

fn refuse_a_directory(header: &ContentAssetHeader) -> anyhow::Result<()> {
    if !header.embedded.is_empty() {
        bail!("a content asset header that lists embedded assets needs their payloads; write it with write_content_asset_with_embedded");
    }
    Ok(())
}

/// Writes a host file: `header` and `payload`, then each embedded asset's payload and blob, listed in the header's directory, then their uncooked sections.
pub fn write_content_asset_with_embedded(
    header: &ContentAssetHeader,
    payload: &[u8],
    embedded: &[EmbeddedPayload<'_>],
) -> anyhow::Result<Vec<u8>> {
    if !header.embedded.is_empty() {
        bail!("the host header's directory is filled from the embedded payloads; pass it empty");
    }
    if header.derived.is_some() {
        bail!("a host written with embedded assets cannot carry a derived section");
    }
    if header.uncooked.is_some() {
        bail!("a host written with embedded assets cannot carry an uncooked section");
    }
    let mut directory = Vec::with_capacity(embedded.len());
    let mut pieces = vec![payload];
    let mut sections = Vec::new();
    let mut offset = payload.len() as u64;
    for asset in embedded {
        let Some(provenance) = &asset.header.provenance else {
            bail!(
                "embedded asset {} has no provenance, so it has no name inside the file",
                asset.header.asset_id.simple_hex()
            );
        };
        let name = &provenance.sub_asset;
        if !asset.header.embedded.is_empty() {
            bail!("embedded asset '{name}' lists embedded assets of its own");
        }
        let blob = asset.derived_blob.unwrap_or(&[]);
        let described = asset.header.derived.map(|section| section.len);
        if described != asset.derived_blob.map(|blob| blob.len() as u64) {
            bail!("embedded asset '{name}' has a derived section that does not match its blob");
        }
        let described = asset.header.uncooked.map(|described| described.len);
        if described != asset.uncooked.map(|section| section.len() as u64) {
            bail!("embedded asset '{name}' has an uncooked section that does not match its bytes");
        }
        directory.push(EmbeddedAsset {
            asset_id: asset.header.asset_id,
            references: asset.header.references.clone(),
            kind: asset.header.kind.clone(),
            kind_version: asset.header.kind_version,
            provenance: provenance.clone(),
            uncooked: asset.header.uncooked,
            derived: asset.header.derived,
            offset,
            len: asset.payload.len() as u64,
        });
        pieces.push(asset.payload);
        pieces.push(blob);
        sections.extend(asset.uncooked);
        offset += (asset.payload.len() + blob.len()) as u64;
    }
    pieces.extend(sections);
    let header = ContentAssetHeader {
        embedded: directory,
        ..header.clone()
    };
    check_directory(&header, None)?;
    frame_content_asset(&header, &pieces)
}

/// Writes a content asset whose header describes `section` and whose payload is followed by its blob, with no uncooked section.
pub fn write_cooked_content_asset(
    header: &ContentAssetHeader,
    payload: &[u8],
    section: CookedSection<'_>,
) -> anyhow::Result<Vec<u8>> {
    refuse_a_directory(header)?;
    let header = ContentAssetHeader {
        uncooked: None,
        derived: Some(DerivedSection {
            derived_version: section.derived_version,
            key: section.key,
            len: section.blob.len() as u64,
        }),
        ..header.clone()
    };
    frame_content_asset(&header, &[payload, section.blob])
}

fn frame_content_asset(header: &ContentAssetHeader, pieces: &[&[u8]]) -> anyhow::Result<Vec<u8>> {
    let header_bytes = bincode::serialize(&(CONTENT_FORMAT_VERSION, header))
        .context("failed to serialize content asset header")?;

    let header_len =
        u32::try_from(header_bytes.len()).context("content asset header exceeds 4 GiB")?;

    let body_len: usize = pieces.iter().map(|piece| piece.len()).sum();
    let mut out = Vec::with_capacity(8 + header_bytes.len() + body_len);
    out.extend_from_slice(&CONTENT_ASSET_MAGIC);
    out.extend_from_slice(&header_len.to_le_bytes());
    out.extend_from_slice(&header_bytes);
    for piece in pieces {
        out.extend_from_slice(piece);
    }
    Ok(out)
}

pub fn read_content_asset(bytes: &[u8]) -> anyhow::Result<(ContentAssetHeader, &[u8])> {
    let parts = read_content_asset_parts(bytes)?;
    Ok((parts.header, parts.payload))
}

/// Reads a content asset's header, its payload and the uncooked section and derived blob trailing it, if any; of a host, the host's own.
pub fn read_content_asset_parts(bytes: &[u8]) -> anyhow::Result<ContentAssetParts<'_>> {
    let file = ContentFile::parse(bytes)?;
    let extent = HostExtent::of(&file.header, file.body.len() as u64)?;
    let body = file.body;
    Ok(ContentAssetParts {
        payload: &body[extent.payload()],
        uncooked: file
            .header
            .uncooked
            .map(|_| &body[extent.uncooked(&file.header)]),
        derived_blob: file.header.derived.map(|_| &body[extent.blob()]),
        header: file.header,
    })
}

struct HostExtent {
    payload_end: usize,
    host_end: usize,
    section_start: usize,
}

impl HostExtent {
    fn of(header: &ContentAssetHeader, body_len: u64) -> anyhow::Result<Self> {
        let blob_len = header.derived.map_or(0, |section| section.len);
        let section_start = sections_start(header, body_len)?;
        let host_end = header
            .embedded
            .first()
            .map_or(section_start, |first| first.offset.min(section_start));
        let payload_end = host_end.checked_sub(blob_len).with_context(|| {
            format!(
                "content asset truncated: derived section claims {blob_len} bytes, only {body_len} follow the header"
            )
        })?;
        Ok(Self {
            payload_end: payload_end as usize,
            host_end: host_end as usize,
            section_start: section_start as usize,
        })
    }

    fn payload(&self) -> std::ops::Range<usize> {
        0..self.payload_end
    }

    fn blob(&self) -> std::ops::Range<usize> {
        self.payload_end..self.host_end
    }

    fn uncooked(&self, header: &ContentAssetHeader) -> std::ops::Range<usize> {
        self.section_start..self.section_start + header.uncooked_len() as usize
    }
}

/// One asset of a content file, borrowed from it: its type tag, its payload, its uncooked section and its derived blob.
#[derive(Debug, Clone, Copy)]
pub struct AssetRecord<'a> {
    /// The `Asset::name()` of the type the payload holds.
    pub kind: &'a str,
    /// The `Asset::FORMAT_VERSION` the payload was written with.
    pub kind_version: u32,
    /// The payload, without the uncooked section and the derived blob.
    pub payload: &'a [u8],
    /// The record's uncooked section, if any.
    pub uncooked: Option<UncookedSlice<'a>>,
    /// The cooked derived data trailing the payload, if any.
    pub cooked: Option<CookedSection<'a>>,
}

impl AssetRecord<'_> {
    /// The record's uncooked section as owned bytes, with the record's type tag.
    pub fn uncooked_section(&self) -> Option<UncookedBytes> {
        self.uncooked.map(|section| UncookedBytes {
            kind: self.kind.to_string(),
            version: section.version,
            bytes: section.bytes.to_vec(),
        })
    }
}

/// A content file with its header parsed once; the host's record and each embedded asset's are slices of it.
#[derive(Debug)]
pub struct ContentFile<'a> {
    header: ContentAssetHeader,
    body: &'a [u8],
}

impl<'a> ContentFile<'a> {
    /// Parses the header of `bytes` and checks its directory against the bytes that follow.
    pub fn parse(bytes: &'a [u8]) -> anyhow::Result<Self> {
        if bytes.len() < 8 || bytes[..4] != CONTENT_ASSET_MAGIC {
            bail!("not a content asset: missing the GRDY magic prefix");
        }

        let header_len = u32::from_le_bytes(
            bytes[4..8]
                .try_into()
                .expect("slice of exactly 4 bytes is always a [u8; 4]"),
        ) as usize;
        let header_end = 8usize
            .checked_add(header_len)
            .context("content asset header length overflows")?;
        if bytes.len() < header_end {
            bail!(
                "content asset truncated: header claims {header_len} bytes, only {} available",
                bytes.len() - 8
            );
        }

        let header = decode_header(&bytes[8..header_end])?;
        let body = &bytes[header_end..];
        check_directory(&header, Some(body.len() as u64))?;
        Ok(Self { header, body })
    }

    /// The file's header, with its directory of embedded assets.
    pub fn header(&self) -> &ContentAssetHeader {
        &self.header
    }

    /// The record of the host, or of the one asset of a loose file.
    pub fn host(&self) -> anyhow::Result<AssetRecord<'_>> {
        let body = self.body;
        let header = &self.header;
        let extent = HostExtent::of(header, body.len() as u64)?;
        Ok(AssetRecord {
            kind: &header.kind,
            kind_version: header.kind_version,
            payload: &body[extent.payload()],
            uncooked: header.uncooked.map(|section| UncookedSlice {
                version: section.version,
                bytes: &body[extent.uncooked(header)],
            }),
            cooked: header
                .derived
                .map(|section| section.cooked(&body[extent.blob()])),
        })
    }

    /// The record of `asset`, an entry of this file's directory.
    pub fn embedded<'file>(
        &'file self,
        asset: &'file EmbeddedAsset,
    ) -> anyhow::Result<AssetRecord<'file>> {
        let asset = named_asset(&self.header, asset.name())?;
        let outside = || {
            format!(
                "embedded asset '{}' is not in this file's directory",
                asset.name()
            )
        };
        let start = asset.offset as usize;
        let blob_end =
            start + asset.len as usize + asset.derived.map_or(0, |section| section.len as usize);
        let bytes = self.body.get(start..blob_end).with_context(outside)?;
        let (payload, blob) = bytes.split_at(asset.len as usize);
        let section_start =
            section_start(&self.header, Some(asset.name()), self.body.len() as u64)? as usize;
        let section = self
            .body
            .get(section_start..section_start + asset.uncooked_len() as usize)
            .with_context(outside)?;
        Ok(AssetRecord {
            kind: &asset.kind,
            kind_version: asset.kind_version,
            payload,
            uncooked: asset.uncooked.map(|described| UncookedSlice {
                version: described.version,
                bytes: section,
            }),
            cooked: asset.derived.map(|section| section.cooked(blob)),
        })
    }
}

/// The bytes a loose file of the host (`None`) or of the named embedded asset would have.
pub fn asset_record(file: &[u8], embedded_as: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let Some(name) = embedded_as else {
        let parts = read_content_asset_parts(file)?;
        let header = ContentAssetHeader {
            embedded: Vec::new(),
            ..parts.header
        };
        return frame_content_asset(
            &header,
            &[
                parts.payload,
                parts.derived_blob.unwrap_or(&[]),
                parts.uncooked.unwrap_or(&[]),
            ],
        );
    };
    let file = ContentFile::parse(file)?;
    let asset = named_asset(&file.header, name)?;
    let record = file.embedded(asset)?;
    frame_content_asset(
        &asset.record_header(),
        &[
            record.payload,
            record.cooked.map_or(&[], |cooked| cooked.blob),
            record.uncooked.map_or(&[], |section| section.bytes),
        ],
    )
}

fn named_asset<'header>(
    header: &'header ContentAssetHeader,
    name: &str,
) -> anyhow::Result<&'header EmbeddedAsset> {
    let Some(asset) = header.embedded.iter().find(|asset| asset.name() == name) else {
        let held: Vec<&str> = header.embedded.iter().map(EmbeddedAsset::name).collect();
        bail!(
            "content asset holds no embedded asset named '{name}'; it holds [{}]",
            held.join(", ")
        );
    };
    Ok(asset)
}

fn without_sections(header: &ContentAssetHeader) -> ContentAssetHeader {
    ContentAssetHeader {
        uncooked: None,
        embedded: header
            .embedded
            .iter()
            .map(|asset| EmbeddedAsset {
                uncooked: None,
                ..asset.clone()
            })
            .collect(),
        ..header.clone()
    }
}

/// The file with every uncooked section removed, which is the file cut where they start under a header that describes none; a file with none comes back byte for byte.
pub fn without_uncooked(file: &[u8]) -> anyhow::Result<Vec<u8>> {
    let parsed = ContentFile::parse(file)?;
    if !parsed.header.has_uncooked() {
        return Ok(file.to_vec());
    }
    let kept = HostExtent::of(&parsed.header, parsed.body.len() as u64)?.section_start;
    frame_content_asset(&without_sections(&parsed.header), &[&parsed.body[..kept]])
}

struct OpenedContentAsset {
    file: std::fs::File,
    header_bytes: Vec<u8>,
    body_start: u64,
    body_len: u64,
}

impl OpenedContentAsset {
    fn open(path: &Path) -> anyhow::Result<Self> {
        use std::io::Read;

        let mut file = std::fs::File::open(path)
            .with_context(|| format!("failed to open '{}'", path.display()))?;

        let mut prefix = [0u8; 8];
        file.read_exact(&mut prefix)
            .with_context(|| format!("'{}' is too short to be a content asset", path.display()))?;
        if prefix[..4] != CONTENT_ASSET_MAGIC {
            bail!(
                "not a content asset: '{}' is missing the GRDY magic prefix",
                path.display()
            );
        }

        let header_len = u32::from_le_bytes(
            prefix[4..8]
                .try_into()
                .expect("slice of exactly 4 bytes is always a [u8; 4]"),
        );
        let file_len = file
            .metadata()
            .with_context(|| format!("failed to read the size of '{}'", path.display()))?
            .len();
        let body_start = prefix.len() as u64 + u64::from(header_len);
        if file_len < body_start {
            bail!(
                "content asset '{}' truncated: header claims {header_len} bytes",
                path.display()
            );
        }

        let mut header_bytes = vec![0u8; header_len as usize];
        file.read_exact(&mut header_bytes).with_context(|| {
            format!(
                "content asset '{}' truncated: header claims {header_len} bytes",
                path.display()
            )
        })?;
        Ok(Self {
            file,
            header_bytes,
            body_start,
            body_len: file_len - body_start,
        })
    }

    fn header(&self, path: &Path) -> anyhow::Result<ContentAssetHeader> {
        let header = decode_header(&self.header_bytes)
            .with_context(|| format!("failed to read the header of '{}'", path.display()))?;
        if !header.embedded.is_empty() {
            check_directory(&header, Some(self.body_len))
                .with_context(|| format!("failed to read the header of '{}'", path.display()))?;
        }
        Ok(header)
    }

    fn read_body(&mut self, range: std::ops::Range<u64>, out: &mut Vec<u8>) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};

        let start = out.len();
        out.resize(start + (range.end - range.start) as usize, 0);
        self.file
            .seek(SeekFrom::Start(self.body_start + range.start))?;
        self.file.read_exact(&mut out[start..])
    }
}

/// Reads the file at `path` as [`without_uncooked`] would give it, stopping where the sections start; a file it cannot understand is read whole.
pub fn read_content_without_uncooked(path: &Path) -> std::io::Result<Vec<u8>> {
    let Ok(mut opened) = OpenedContentAsset::open(path) else {
        return std::fs::read(path);
    };
    let body_len = opened.body_len;
    let stripped = decode_header(&opened.header_bytes)
        .ok()
        .filter(ContentAssetHeader::has_uncooked)
        .and_then(|header| {
            check_directory(&header, Some(body_len)).ok()?;
            let kept = HostExtent::of(&header, body_len).ok()?.section_start as u64;
            let framed = frame_content_asset(&without_sections(&header), &[]).ok()?;
            Some(Stripped { framed, kept })
        });
    let Some(Stripped {
        framed: mut out,
        kept,
    }) = stripped
    else {
        return std::fs::read(path);
    };
    opened.read_body(0..kept, &mut out)?;
    Ok(out)
}

struct Stripped {
    framed: Vec<u8>,
    kept: u64,
}

/// Reads only the uncooked section of the host (`None`) or of the named embedded asset of the file at `path`; `None` when the record has none.
pub fn read_uncooked_section(
    path: &Path,
    embedded_as: Option<&str>,
) -> anyhow::Result<Option<UncookedBytes>> {
    let mut opened = OpenedContentAsset::open(path)?;
    let header = opened.header(path)?;
    let in_file = || format!("failed to read an uncooked section of '{}'", path.display());
    let record = match embedded_as {
        None => header.record_header(None),
        Some(name) => Some(
            named_asset(&header, name)
                .with_context(in_file)?
                .record_header(),
        ),
    };
    let Some(record) = record else {
        return Ok(None);
    };
    let Some(section) = record.uncooked else {
        return Ok(None);
    };
    let start = section_start(&header, embedded_as, opened.body_len).with_context(in_file)?;
    let mut bytes = Vec::new();
    opened
        .read_body(start..start + section.len, &mut bytes)
        .with_context(in_file)?;
    Ok(Some(UncookedBytes {
        kind: record.kind,
        version: section.version,
        bytes,
    }))
}

/// Reads just the header of the content asset at `path`, never the payload.
///
/// A content tree holds whole textures and meshes — tens of megabytes each —
/// so indexing one by reading every file whole is not viable. This reads the
/// 8-byte magic-and-length prefix, then exactly `header_len` more bytes.
pub fn read_content_asset_header(path: &Path) -> anyhow::Result<ContentAssetHeader> {
    OpenedContentAsset::open(path)?.header(path)
}

/// Where an embedded asset is stored: the asset whose file holds it, and its name there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedIn {
    pub host: AssetId,
    pub name: String,
}

/// One content asset: what a scan reads from its header, or a registry line holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEntry {
    pub id: AssetId,
    /// The path relative to the scan's base, `/`-separated (`"content/hero/body.gasset"`).
    pub address: String,
    pub kind: String,
    /// The header's `kind_version`; `None` for a registry line.
    pub kind_version: Option<u32>,
    /// Where an imported asset came from; `None` for an authored asset and for a registry line.
    pub provenance: Option<ImportProvenance>,
    /// The host whose file at `address` holds the asset, and its name there; `None` for the host or a loose asset.
    pub embedded_in: Option<EmbeddedIn>,
}

impl ScannedEntry {
    /// `address`, or `address#name` for an embedded asset.
    pub fn location(&self) -> String {
        let name = self.embedded_in.as_ref().map(|held| held.name.as_str());
        asset_location(&self.address, name)
    }
}

/// An asset's written location: `address`, or `address#name` for an embedded asset.
pub fn asset_location(address: &str, embedded_as: Option<&str>) -> String {
    match embedded_as {
        Some(name) => format!("{address}#{name}"),
        None => address.to_string(),
    }
}

const MAX_FILE_NAME_BYTES: usize = 200;

fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The file stem a sub-asset name gets: path separators and `#` replaced, capped so the name with `extension` fits a file name.
pub fn sub_asset_file_stem(sub_asset: &str, extension: &str) -> String {
    let sanitized = sub_asset.replace(['/', '\\', '#'], "_");
    if sanitized.len() + 1 + extension.len() <= MAX_FILE_NAME_BYTES {
        return sanitized;
    }
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(extension.len() + 1 + 9);
    let mut end = budget.min(sanitized.len());
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}_{:08x}", &sanitized[..end], fnv1a32(sub_asset))
}

fn normalise_lexically(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Lists every asset in the `*.<extension>` files under `dir` by their headers, addressed relative to `base`: each file's host, then its embedded assets.
pub fn scan_content(base: &Path, dir: &Path, extension: &str) -> anyhow::Result<Vec<ScannedEntry>> {
    let base = normalise_lexically(base);
    let dir = normalise_lexically(dir);

    let mut entries = Vec::new();
    let mut index_by_id: HashMap<AssetId, usize> = HashMap::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            let canonical = match path.canonicalize() {
                Ok(canonical) => canonical,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to resolve '{}'", path.display()))
                }
            };
            if !visited.insert(canonical) {
                continue;
            }
            let listing = match std::fs::read_dir(&path) {
                Ok(listing) => listing,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err).with_context(|| format!("failed to read '{}'", path.display()))
                }
            };
            let mut children = Vec::new();
            for entry in listing {
                children.push(
                    entry
                        .with_context(|| {
                            format!("failed to read an entry of '{}'", path.display())
                        })?
                        .path(),
                );
            }
            children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            stack.extend(children.into_iter().rev());
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }

        let header = read_content_asset_header(&path)?;
        let address = path
            .strip_prefix(&base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        let host_id = header.asset_id;
        let host = ScannedEntry {
            id: host_id,
            address: address.clone(),
            kind: header.kind,
            kind_version: Some(header.kind_version),
            provenance: header.provenance,
            embedded_in: None,
        };
        let embedded = header.embedded.into_iter().map(|asset| ScannedEntry {
            id: asset.asset_id,
            address: address.clone(),
            kind: asset.kind,
            kind_version: Some(asset.kind_version),
            embedded_in: Some(EmbeddedIn {
                host: host_id,
                name: asset.provenance.sub_asset.clone(),
            }),
            provenance: Some(asset.provenance),
        });
        for entry in std::iter::once(host).chain(embedded) {
            if let Some(&previous) = index_by_id.get(&entry.id) {
                let previous: &ScannedEntry = &entries[previous];
                bail!(
                    "content tree is malformed: '{}' and '{}' both carry asset id {}",
                    previous.location(),
                    entry.location(),
                    entry.id.simple_hex()
                );
            }
            index_by_id.insert(entry.id, entries.len());
            entries.push(entry);
        }
    }

    Ok(entries)
}

#[derive(Deserialize)]
struct RegistryRecord {
    address: String,
    kind: String,
    #[serde(default)]
    embedded_as: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredRecord {
    Record(RegistryRecord),
    Legacy(String),
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    assets: BTreeMap<String, StoredRecord>,
}

/// The registry text: one `id = { address, kind }` line per entry, with `embedded_as` for an embedded asset, ordered by id.
pub fn write_registry(entries: &[ScannedEntry]) -> String {
    let mut ordered: Vec<&ScannedEntry> = entries.iter().collect();
    ordered.sort_by_key(|entry| entry.id);
    let mut text = String::from("[assets]\n");
    for entry in ordered {
        let embedded_as = entry.embedded_in.as_ref().map_or(String::new(), |held| {
            format!(", embedded_as = {}", toml::Value::String(held.name.clone()))
        });
        text.push_str(&format!(
            "{} = {{ address = {}, kind = {}{embedded_as} }}\n",
            entry.id.simple_hex(),
            toml::Value::String(entry.address.clone()),
            toml::Value::String(entry.kind.clone()),
        ));
    }
    text
}

struct ParsedRecord {
    id: AssetId,
    record: RegistryRecord,
}

fn two_ids_at(location: &str, first: AssetId, second: AssetId) -> anyhow::Error {
    anyhow::anyhow!(
        "asset registry maps two ids ({} and {}) to the same address '{location}'",
        first.simple_hex(),
        second.simple_hex(),
    )
}

/// Reads registry text written by [`write_registry`], ordered by id.
pub fn parse_registry(text: &str) -> anyhow::Result<Vec<ScannedEntry>> {
    let file: RegistryFile = toml::from_str(text).context("failed to parse asset registry")?;
    let mut records = Vec::with_capacity(file.assets.len());
    let mut id_by_address: HashMap<String, AssetId> = HashMap::new();
    for (hex, record) in file.assets {
        let id = AssetId::from_simple_hex(&hex)
            .map_err(|err| anyhow::anyhow!("invalid asset id '{hex}' in registry: {err}"))?;
        let record = match record {
            StoredRecord::Record(record) => record,
            StoredRecord::Legacy(address) => bail!(
                "asset registry predates asset kinds ('{address}' has none); rebuild the game so its build script writes a new one"
            ),
        };
        if record.embedded_as.is_none() {
            if let Some(existing) = id_by_address.insert(record.address.clone(), id) {
                return Err(two_ids_at(&record.address, existing, id));
            }
        }
        records.push(ParsedRecord { id, record });
    }
    let mut entries: Vec<ScannedEntry> = Vec::with_capacity(records.len());
    let mut embedded_by_host: HashMap<AssetId, Vec<usize>> = HashMap::new();
    for ParsedRecord { id, record } in records {
        let embedded_in = match record.embedded_as {
            Some(name) => {
                let Some(&host) = id_by_address.get(&record.address) else {
                    bail!(
                        "asset registry places {} in '{}' as '{name}', but lists no asset at that address to hold it",
                        id.simple_hex(),
                        record.address
                    );
                };
                let held = embedded_by_host.entry(host).or_default();
                let named = |at: &&usize| {
                    entries[**at]
                        .embedded_in
                        .as_ref()
                        .is_some_and(|held| held.name == name)
                };
                if let Some(&existing) = held.iter().find(named) {
                    return Err(two_ids_at(
                        &asset_location(&record.address, Some(&name)),
                        entries[existing].id,
                        id,
                    ));
                }
                held.push(entries.len());
                Some(EmbeddedIn { host, name })
            }
            None => None,
        };
        entries.push(ScannedEntry {
            id,
            address: record.address,
            kind: record.kind,
            kind_version: None,
            provenance: None,
            embedded_in,
        });
    }
    Ok(entries)
}

/// `content.toml` at a project root, read by the import, the editor's scan and `track_assets`, never at runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ContentConfig {
    /// Uniform extension for every content asset, without the dot.
    pub extension: String,
    /// Content tree root, relative to the project root; must be `content` for now.
    pub root: String,
    /// The address of the scene the editor opens with the project, such as `content/levels/forest.gasset`.
    pub default_scene: Option<String>,
}

impl Default for ContentConfig {
    fn default() -> Self {
        Self {
            extension: "gasset".to_string(),
            root: "content".to_string(),
            default_scene: None,
        }
    }
}

impl ContentConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read '{}'", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse '{}'", path.display()))?;
        config
            .validate()
            .with_context(|| format!("invalid '{}'", path.display()))?;
        Ok(config)
    }

    /// Rejects a root other than `content` and an extension that is empty or holds a dot or path separator.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.root == "content",
            "content root must be 'content'; a custom root is not supported yet"
        );
        anyhow::ensure!(
            !self.extension.is_empty() && !self.extension.contains(['.', '/', '\\']),
            "content extension must be a nonempty extension without dots or path separators"
        );
        Ok(())
    }

    /// Loads `content.toml` from `dir` if present, else the defaults.
    pub fn load_or_default(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("content.toml");
        if path.try_exists()? {
            Self::load(&path)
        } else {
            Ok(Self::default())
        }
    }
}
