//! Framing for game-ready content assets: `magic | u32 header_len |
//! bincode(u32 format version, ContentAssetHeader) | payload (verbatim) | editor-only section |
//! derived blob`, the section and the blob present only when the header describes them. The header is read
//! without touching the payload, so a future asset registry can index a
//! whole content tree by scanning headers alone.
use std::path::Path;

use anyhow::Context;

use super::registry::{AssetEntry, AssetRegistry};
use super::{Asset, AssetId};

pub use concerto_asset_format::{
    asset_record, read_content_asset, read_content_asset_header, read_content_asset_parts,
    read_content_without_editor_only, read_editor_only_section, sub_asset_file_stem,
    without_editor_only, write_content_asset, write_content_asset_with_editor_only,
    write_content_asset_with_embedded, write_cooked_content_asset, AssetRecord, ContentAssetHeader,
    ContentAssetParts, ContentFile, CookedSection, DerivedSection, EditorOnlyBytes,
    EditorOnlySection, EmbeddedAsset, EmbeddedPayload, ImportProvenance, CONTENT_ASSET_MAGIC,
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

/// Decodes an editor-only section as `A`'s, checking its kind and type version and migrating an older version.
pub fn decode_editor_only<A: Asset>(
    kind: &str,
    version: u32,
    bytes: &[u8],
) -> anyhow::Result<A::EditorOnly> {
    if kind != A::name() {
        anyhow::bail!(
            "content asset holds the editor-only data of a {kind} but a {}'s was requested",
            A::name()
        );
    }
    let current = A::FORMAT_VERSION;
    let value = match version.cmp(&current) {
        std::cmp::Ordering::Equal => bincode::deserialize(bytes).with_context(|| {
            format!(
                "failed to deserialize the editor-only data of a {}",
                A::name()
            )
        })?,
        std::cmp::Ordering::Less => A::migrate_editor_only(version, bytes).with_context(|| {
            format!(
                "{name} editor-only data is version {version}; this build's {name} is version {current}",
                name = A::name()
            )
        })?,
        std::cmp::Ordering::Greater => anyhow::bail!(
            "{} editor-only data is version {version}, newer than this build's version {current}",
            A::name()
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
        file: format!("{}.v{}.bin", A::name(), A::FORMAT_VERSION),
    }
    .check(&bytes, dir);
}

/// Compares the editor-only encoding of `value` with `<dir>/<A::name()>.editor-only.v<A::FORMAT_VERSION>.bin`, for a crate's encoding test.
pub fn check_editor_only_encoding<A: Asset>(value: &A::EditorOnly, dir: &Path) {
    let bytes = bincode::serialize(value).unwrap_or_else(|error| {
        panic!(
            "failed to serialize the editor-only data of {}: {error}",
            A::name()
        )
    });
    EncodingFixture {
        name: A::name(),
        version: A::FORMAT_VERSION,
        what: "editor-only encoding",
        file: format!("{}.editor-only.v{}.bin", A::name(), A::FORMAT_VERSION),
    }
    .check(&bytes, dir);
}

struct EncodingFixture {
    name: &'static str,
    version: u32,
    what: &'static str,
    file: String,
}

impl EncodingFixture {
    fn check(&self, bytes: &[u8], dir: &Path) {
        let Self {
            name,
            version,
            what,
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
                "the {what} of {name} changed but its FORMAT_VERSION is still {version}: raise it to {next}, add a migration from {version}, in `migrate` and, for a type with editor-only data, in `migrate_editor_only`, or accept that version-{version} files are rejected (docs/asset-importing.md, \"Changing an asset type's encoding\"), then run with CONCERTO_BLESS_ENCODING=1",
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

/// The standalone record of `entry` without its editor-only section, which is not read from disk.
pub fn read_asset_record_without_editor_only(
    project_root: &Path,
    entry: &AssetEntry,
) -> anyhow::Result<Vec<u8>> {
    let path = project_root.join(&entry.address);
    let file = read_content_without_editor_only(&path)
        .with_context(|| format!("failed to read content asset '{}'", path.display()))?;
    let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
    asset_record(&file, name)
        .with_context(|| format!("malformed content asset '{}'", entry.location()))
}

/// The editor-only data of `entry`, read alone from `<project_root>/<entry.address>`; `None` when its record has none.
pub fn read_editor_only<A: Asset>(
    project_root: &Path,
    entry: &AssetEntry,
) -> anyhow::Result<Option<A::EditorOnly>> {
    let path = project_root.join(&entry.address);
    let name = entry.embedded_in.as_ref().map(|held| held.name.as_str());
    let Some(section) = read_editor_only_section(&path, name)? else {
        return Ok(None);
    };
    decode_editor_only::<A>(&section.kind, section.kind_version, &section.bytes)
        .with_context(|| format!("malformed content asset '{}'", entry.location()))
        .map(Some)
}

fn editor_only_to_write<A: Asset>(
    given: Option<&A::EditorOnly>,
    path: &Path,
    existing: Option<&ContentAssetHeader>,
) -> anyhow::Result<Vec<u8>> {
    if let Some(value) = given {
        return bincode::serialize(value).context("failed to serialize editor-only data");
    }
    let kept = existing.filter(|header| header.kind == A::name() && header.editor_only.is_some());
    if kept.is_none() {
        return Ok(Vec::new());
    }
    let Some(section) = read_editor_only_section(path, None)? else {
        return Ok(Vec::new());
    };
    if section.kind_version == A::FORMAT_VERSION {
        return Ok(section.bytes);
    }
    let migrated = decode_editor_only::<A>(&section.kind, section.kind_version, &section.bytes)
        .with_context(|| {
            format!(
                "failed to keep the editor-only data of '{}'",
                path.display()
            )
        })?;
    bincode::serialize(&migrated).context("failed to serialize editor-only data")
}

/// Writes `value` as a content asset at `project_root/address`, records it in `registry`, and returns its id, reusing the one already in the file's header; `editor_only` of `None` keeps the file's editor-only section.
pub fn save_content_asset_into<A: Asset>(
    value: &A,
    editor_only: Option<&A::EditorOnly>,
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
    let editor_only = editor_only_to_write::<A>(editor_only, &path, existing.as_ref())?;
    let header = ContentAssetHeader {
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: None,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let kind = header.kind.clone();
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = write_content_asset_with_editor_only(&header, &payload, &editor_only)?;

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

/// Writes `value` over the content asset at `address`, keeping its id and import provenance, and upserts its registry entry; `editor_only` of `None` keeps the file's editor-only section.
pub fn resave_content_asset<A: Asset>(
    value: &A,
    editor_only: Option<&A::EditorOnly>,
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
    let editor_only = editor_only_to_write::<A>(editor_only, &path, Some(&existing))?;
    let header = ContentAssetHeader {
        asset_id: existing.asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        kind_version: A::FORMAT_VERSION,
        provenance: existing.provenance,
        editor_only: None,
        derived: None,
        embedded: Vec::new(),
    };
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = write_content_asset_with_editor_only(&header, &payload, &editor_only)?;
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
