//! Framing for game-ready content assets: `magic | u32 header_len |
//! bincode(u32 format version, ContentAssetHeader) | payload (verbatim) | derived blob |
//! uncooked section`, the blob and the section present only when the header describes them. The header is read
//! without touching the payload, so a future asset registry can index a
//! whole content tree by scanning headers alone.
use std::path::Path;

use anyhow::Context;

use super::registry::{AssetEntry, AssetRegistry};
use super::{Asset, AssetId, UncookedData};

pub use concerto_asset_format::{
    asset_record, read_content_asset, read_content_asset_header, read_content_asset_parts,
    read_content_without_uncooked, read_uncooked_section, sub_asset_file_stem,
    without_import_record, without_uncooked, write_content_asset,
    write_content_asset_with_embedded, write_content_asset_with_uncooked,
    write_cooked_content_asset, AssetRecord, ContentAssetHeader, ContentAssetParts, ContentFile,
    CookedSection, DerivedSection, EmbeddedAsset, EmbeddedPayload, ImportProvenance, ImportRecord,
    SourceFile, UncookedBytes, UncookedSection, UncookedSlice, CONTENT_ASSET_MAGIC,
    CONTENT_FORMAT_VERSION,
};

/// Decodes a content asset as `A`, checking its kind and type version and migrating an older version.
pub fn decode_content_asset<A: Asset>(bytes: &[u8]) -> anyhow::Result<(ContentAssetHeader, A)> {
    let (header, payload) = read_content_asset(bytes)?;
    let value = decode_content_payload::<A>(&header.kind, header.kind_version, payload)?;
    Ok((header, value))
}

pub(crate) fn decode_content_payload<A: Asset>(
    kind: &str,
    version: u32,
    payload: &[u8],
) -> anyhow::Result<A> {
    if kind != A::name() {
        anyhow::bail!(
            "content asset holds a {kind} but a {} was requested",
            A::name()
        );
    }
    let current = A::FORMAT_VERSION;
    let value = match version.cmp(&current) {
        std::cmp::Ordering::Equal => bincode::deserialize(payload)
            .with_context(|| format!("failed to deserialize {} payload", A::name()))?,
        std::cmp::Ordering::Less => A::migrate(version, payload).with_context(|| {
            format!(
                "{name} payload is version {version}; this build's {name} is version {current}",
                name = A::name()
            )
        })?,
        std::cmp::Ordering::Greater => anyhow::bail!(
            "{} payload is version {version}, newer than this build's version {current}",
            A::name()
        ),
    };
    Ok(value)
}

/// Decodes an uncooked section of a record of type `kind` as `E`, checking the kind and the section's version and migrating an older version.
pub fn decode_uncooked<E: UncookedData>(
    kind: &str,
    version: u32,
    bytes: &[u8],
) -> anyhow::Result<E> {
    let name = E::Asset::name();
    if kind != name {
        anyhow::bail!(
            "content asset holds the uncooked data of a {kind} but a {name}'s was requested"
        );
    }
    let current = E::VERSION;
    let value = match version.cmp(&current) {
        std::cmp::Ordering::Equal => bincode::deserialize(bytes)
            .with_context(|| format!("failed to deserialize the uncooked data of a {name}"))?,
        std::cmp::Ordering::Less => E::migrate(version, bytes).with_context(|| {
            format!("{name} uncooked data is version {version}; this build's is version {current}")
        })?,
        std::cmp::Ordering::Greater => anyhow::bail!(
            "{name} uncooked data is version {version}, newer than this build's version {current}"
        ),
    };
    Ok(value)
}

/// Compares `value`'s payload encoding with `<dir>/<A::name()>.v<A::FORMAT_VERSION>.bin`, for a crate's encoding test.
pub fn check_encoding<A: Asset>(value: &A, dir: &Path) {
    let bytes = bincode::serialize(value)
        .unwrap_or_else(|error| panic!("failed to serialize {}: {error}", A::name()));
    EncodingFixture {
        name: A::name(),
        version: A::FORMAT_VERSION,
        what: "encoding",
        constant: "FORMAT_VERSION",
        file: format!("{}.v{}.bin", A::name(), A::FORMAT_VERSION),
    }
    .check(&bytes, dir);
}

/// Compares `value`'s encoding with `<dir>/<E::Asset::name()>.uncooked.v<E::VERSION>.bin`, for a crate's encoding test.
pub fn check_uncooked_encoding<E: UncookedData>(value: &E, dir: &Path) {
    let name = E::Asset::name();
    let bytes = bincode::serialize(value)
        .unwrap_or_else(|error| panic!("failed to serialize the uncooked data of {name}: {error}"));
    EncodingFixture {
        name,
        version: E::VERSION,
        what: "uncooked encoding",
        constant: "uncooked type's VERSION",
        file: format!("{name}.uncooked.v{}.bin", E::VERSION),
    }
    .check(&bytes, dir);
}

struct EncodingFixture {
    name: &'static str,
    version: u32,
    what: &'static str,
    constant: &'static str,
    file: String,
}

impl EncodingFixture {
    fn check(&self, bytes: &[u8], dir: &Path) {
        let Self {
            name,
            version,
            what,
            constant,
            file,
        } = self;
        let path = dir.join(file);
        if std::env::var("CONCERTO_BLESS_ENCODING").as_deref() == Ok("1") {
            std::fs::create_dir_all(dir)
                .unwrap_or_else(|error| panic!("failed to create '{}': {error}", dir.display()));
            std::fs::write(&path, bytes)
                .unwrap_or_else(|error| panic!("failed to write '{}': {error}", path.display()));
            return;
        }
        let Ok(fixture) = std::fs::read(&path) else {
            panic!(
                "no {what} fixture for {name} version {version} at {}; run this test with CONCERTO_BLESS_ENCODING=1 to write it",
                path.display()
            );
        };
        if fixture != bytes {
            panic!(
                "the {what} of {name} changed but its {constant} is still {version}: raise it to {next}, add a migration from {version}, or accept that version-{version} files are rejected (docs/asset-importing.md, \"Changing an asset type's encoding\"), then run with CONCERTO_BLESS_ENCODING=1",
                next = version + 1
            );
        }
    }
}

/// The standalone record of `entry`, read from `<project_root>/<entry.address>`.
pub fn read_asset_record(project_root: &Path, entry: &AssetEntry) -> anyhow::Result<Vec<u8>> {
    let path = project_root.join(&entry.address);
    let file = std::fs::read(&path)
        .with_context(|| format!("failed to read content asset '{}'", path.display()))?;
    let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
    asset_record(&file, name)
        .with_context(|| format!("malformed content asset '{}'", entry.location()))
}

/// The standalone record of `entry` without its uncooked section, which is not read from disk.
pub fn read_asset_record_without_uncooked(
    project_root: &Path,
    entry: &AssetEntry,
) -> anyhow::Result<Vec<u8>> {
    let path = project_root.join(&entry.address);
    let file = read_content_without_uncooked(&path)
        .with_context(|| format!("failed to read content asset '{}'", path.display()))?;
    let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
    asset_record(&file, name)
        .with_context(|| format!("malformed content asset '{}'", entry.location()))
}

/// The uncooked data of `entry`, read alone from `<project_root>/<entry.address>`; `None` when its record has none.
pub fn read_uncooked<E: UncookedData>(
    project_root: &Path,
    entry: &AssetEntry,
) -> anyhow::Result<Option<E>> {
    let path = project_root.join(&entry.address);
    let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
    let Some(section) = read_uncooked_section(&path, name)? else {
        return Ok(None);
    };
    decode_uncooked::<E>(&section.kind, section.version, &section.bytes)
        .with_context(|| format!("malformed content asset '{}'", entry.location()))
        .map(Some)
}

struct SectionToWrite {
    version: u32,
    bytes: Vec<u8>,
}

impl SectionToWrite {
    fn of<E: UncookedData>(value: &E) -> anyhow::Result<Self> {
        Ok(Self {
            version: E::VERSION,
            bytes: bincode::serialize(value).context("failed to serialize uncooked data")?,
        })
    }

    fn kept(
        path: &Path,
        existing: Option<&ContentAssetHeader>,
        kind: &str,
    ) -> anyhow::Result<Option<Self>> {
        let holds = existing.is_some_and(|header| header.kind == kind && header.uncooked.is_some());
        if !holds {
            return Ok(None);
        }
        Ok(read_uncooked_section(path, None)?.map(|section| Self {
            version: section.version,
            bytes: section.bytes,
        }))
    }
}

fn written(
    header: &ContentAssetHeader,
    payload: &[u8],
    section: Option<&SectionToWrite>,
) -> anyhow::Result<Vec<u8>> {
    match section {
        Some(section) => {
            write_content_asset_with_uncooked(header, payload, section.version, &section.bytes)
        }
        None => write_content_asset(header, payload),
    }
}

/// Writes `value` as a content asset at `project_root/address`, records it in `registry`, and returns its id, reusing the one already in the file's header and keeping the file's uncooked section.
pub fn save_content_asset_into<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    save_into(value, None, project_root, address, registry)
}

/// Writes `value` as [`save_content_asset_into`] does, with `uncooked` as the file's uncooked section.
pub fn save_content_asset_with_uncooked<E: UncookedData>(
    value: &E::Asset,
    uncooked: &E,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let section = SectionToWrite::of(uncooked)?;
    save_into(value, Some(section), project_root, address, registry)
}

fn save_into<A: Asset>(
    value: &A,
    section: Option<SectionToWrite>,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let path = project_root.join(address);
    let existing = match path.exists() {
        true => {
            let existing = read_content_asset_header(&path)?;
            refuse_a_host(&existing, address)?;
            Some(existing)
        }
        false => None,
    };
    let asset_id = existing
        .as_ref()
        .map_or_else(AssetId::new, |existing| existing.asset_id);
    let section = match section {
        Some(section) => Some(section),
        None => SectionToWrite::kept(&path, existing.as_ref(), A::name())?,
    };
    let header = ContentAssetHeader {
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: None,
        import: None,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    };
    let kind = header.kind.clone();
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = written(&header, &payload, section.as_ref())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write content asset '{}'", path.display()))?;

    registry.insert(AssetEntry {
        embedded_in: None,
        id: asset_id,
        address: address.to_owned(),
        kind,
        kind_version: Some(A::FORMAT_VERSION),
        provenance: None,
    });
    Ok(asset_id)
}

/// Writes `value` over the content asset at `address`, keeping its id, its import provenance and record and its uncooked section, and upserts its registry entry.
pub fn resave_content_asset<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
    registry: &mut AssetRegistry,
) -> anyhow::Result<AssetId> {
    let path = project_root.join(address);
    let existing = read_content_asset_header(&path)
        .with_context(|| format!("no content asset to resave at '{address}'"))?;
    refuse_a_host(&existing, address)?;
    if existing.kind != A::name() {
        anyhow::bail!(
            "'{address}' holds a {} but a {} was resaved",
            existing.kind,
            A::name()
        );
    }
    let section = SectionToWrite::kept(&path, Some(&existing), A::name())?;
    let header = ContentAssetHeader {
        asset_id: existing.asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: existing.provenance,
        import: existing.import,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
    };
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = written(&header, &payload, section.as_ref())?;
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write content asset '{}'", path.display()))?;

    registry.insert(AssetEntry {
        embedded_in: None,
        id: header.asset_id,
        address: address.to_owned(),
        kind: header.kind,
        kind_version: Some(header.kind_version),
        provenance: header.provenance,
    });
    Ok(header.asset_id)
}

fn refuse_a_host(existing: &ContentAssetHeader, address: &str) -> anyhow::Result<()> {
    if existing.embedded.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "'{address}' belongs to an import: its file holds {} embedded assets, is written whole by the import and is read-only; set `file = true` for an output in the source's `.import.toml` and import again to make that output a file of its own",
        existing.embedded.len()
    )
}
