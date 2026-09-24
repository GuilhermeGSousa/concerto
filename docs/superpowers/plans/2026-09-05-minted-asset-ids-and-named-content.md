# Minted Asset IDs and Named Content Assets — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give content assets minted, rename-safe identities and human names taken from the source file, and make the runtime resolve addresses through a registry that is preloaded once per app.

**Architecture:** An asset's `AssetId` stops being `AssetId::from_path(address)` and becomes a v4 UUID minted on first write and reused on re-import by reading the existing header. The registry becomes derived data, rebuilt by scanning the content tree's headers, which is what makes a hand-rename survivable. Sub-asset names come from glTF/OBJ names instead of indices, and `content_address` turns the name's `/` segments into real directories. At runtime, `AssetManagerPlugin` preloads the registry through the existing `build → ready → finish` plugin lifecycle (`block_on` native, `spawn_local` wasm), and `AssetServer::load` resolves address → id from that in-memory index.

**Tech Stack:** Rust workspace; `bincode` + `serde` for the content envelope, `toml` for the registry, `pollster`/`wasm-bindgen-futures` for the platform-split preload, `gltf`/`tobj` in the importers.

**Spec:** `docs/superpowers/specs/2026-09-05-minted-asset-ids-and-named-content-design.md`

## Global Constraints

- CI gates, run in this exact form after every task: `cargo build --workspace` (zero warnings), `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings` (no `--workspace`, no `--all-targets` on clippy).
- Commit messages end with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- Stage only the paths a task actually changes. Never `git add -A` — `docs/superpowers/` is gitignored and must stay untracked, and stray files have been bundled into commits on this branch before.
- The content tree (`examples/*/content/`) is committed to version control.
- `AssetServer::load()` stays synchronous. No task may make it `async` or change its signature.
- Identity is minted, never derived from the address. After Task 3, no code may compute a content asset's id with `AssetId::from_path(<content address>)`. `AssetId::from_path` itself stays (other callers, tests).
- The registry file `import` writes is a full rebuild from a tree scan, not a merge.
- Every example must keep working at runtime after every task. Unlike the previous plan, there is no accepted window of visually-broken examples: Task 7 switches the examples to the plugin builder in the same commit that makes the registry mandatory, and Tasks 8-10 re-import one example each.

---

## Task 1: Header prefix-reader, registry address index, registry-from-tree scan

**Files:**
- Modify: `crates/essential/src/assets/content.rs`
- Test: `crates/essential/tests/asset_registry.rs`

**Interfaces:**
- Produces: `asset::content::read_content_asset_header(path: &Path) -> anyhow::Result<ContentAssetHeader>`; `AssetRegistry::id_for_address(&self, address: &str) -> Option<AssetId>`; `AssetRegistry::from_content_tree(project_root: &Path, content_root: &str, extension: &str) -> anyhow::Result<Self>`.
- Consumes: nothing new.
- Purely additive — no existing behaviour changes in this task.

- [ ] **Step 1: Write the failing tests**

Append to `crates/essential/tests/asset_registry.rs`:

```rust
#[test]
fn read_content_asset_header_does_not_read_the_payload() {
    let dir = temp_root("prefix-read");
    let address = "content/big/thing.gasset";
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Thing".to_string(),
        provenance: None,
    };
    // A payload far larger than the header, so a whole-file read would be
    // obviously wasteful and a prefix read is provably enough.
    let payload = vec![7u8; 4 * 1024 * 1024];
    std::fs::create_dir_all(dir.join("content/big")).unwrap();
    std::fs::write(
        dir.join(address),
        write_content_asset(&header, &payload).unwrap(),
    )
    .unwrap();

    let read = read_content_asset_header(&dir.join(address)).expect("header reads");
    assert_eq!(read, header, "the prefix read returns the whole header");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn id_for_address_is_the_inverse_of_get() {
    let mut registry = AssetRegistry::new();
    let id = AssetId::new();
    registry.insert(id, "content/hero/scene.gasset");

    assert_eq!(registry.get(id), Some("content/hero/scene.gasset"));
    assert_eq!(registry.id_for_address("content/hero/scene.gasset"), Some(id));
    assert_eq!(registry.id_for_address("content/nope.gasset"), None);
}

#[test]
fn parse_rejects_two_ids_sharing_one_address() {
    let text = format!(
        "[assets]\n\"{}\" = \"content/a.gasset\"\n\"{}\" = \"content/a.gasset\"\n",
        AssetId::new().simple_hex(),
        AssetId::new().simple_hex(),
    );
    let err = AssetRegistry::parse(&text).expect_err("a duplicate address is malformed");
    assert!(
        format!("{err:#}").contains("content/a.gasset"),
        "the error must name the duplicated address, got: {err:#}"
    );
}

#[test]
fn from_content_tree_indexes_every_gasset_by_its_header_id() {
    let dir = temp_root("scan");
    let mut expected = Vec::new();
    for (sub, kind) in [("mesh/Body", "Mesh"), ("animation/Idle", "AnimationClip")] {
        let address = format!("content/hero/{}.gasset", sub.replace('/', "_"));
        let id = AssetId::new();
        let header = ContentAssetHeader {
            format_version: CONTENT_FORMAT_VERSION,
            asset_id: id,
            references: Vec::new(),
            kind: kind.to_string(),
            provenance: None,
        };
        std::fs::create_dir_all(dir.join("content/hero")).unwrap();
        std::fs::write(
            dir.join(&address),
            write_content_asset(&header, b"payload").unwrap(),
        )
        .unwrap();
        expected.push((id, address));
    }

    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset").expect("scan");

    assert_eq!(registry.iter().count(), 2);
    for (id, address) in &expected {
        assert_eq!(
            registry.get(*id),
            Some(address.as_str()),
            "the scan keys each file by the id in its own header"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_repoints_a_moved_file_at_its_new_address() {
    let dir = temp_root("scan-moved");
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    let bytes = write_content_asset(&header, b"payload").unwrap();
    std::fs::write(dir.join("content/hero/old.gasset"), &bytes).unwrap();

    let before = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(before.get(id), Some("content/hero/old.gasset"));

    // The rename an artist (or a future editor) would do by hand.
    std::fs::rename(
        dir.join("content/hero/old.gasset"),
        dir.join("content/hero/new.gasset"),
    )
    .unwrap();

    let after = AssetRegistry::from_content_tree(&dir, "content", "gasset").unwrap();
    assert_eq!(
        after.get(id),
        Some("content/hero/new.gasset"),
        "the id follows the file, which is what keeps baked references resolving"
    );
    assert_eq!(after.iter().count(), 1, "the old address is gone, not merged");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_rejects_two_files_sharing_an_id() {
    let dir = temp_root("scan-dup-id");
    let id = AssetId::new();
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: Vec::new(),
        kind: "Mesh".to_string(),
        provenance: None,
    };
    let bytes = write_content_asset(&header, b"payload").unwrap();
    std::fs::create_dir_all(dir.join("content/hero")).unwrap();
    std::fs::write(dir.join("content/hero/a.gasset"), &bytes).unwrap();
    std::fs::write(dir.join("content/hero/b.gasset"), &bytes).unwrap();

    let err = AssetRegistry::from_content_tree(&dir, "content", "gasset")
        .expect_err("a copy-pasted .gasset is a malformed tree");
    let message = format!("{err:#}");
    assert!(
        message.contains("a.gasset") && message.contains("b.gasset"),
        "the error must name both paths, got: {message}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn from_content_tree_of_an_absent_tree_is_empty() {
    let dir = temp_root("scan-absent");
    let registry = AssetRegistry::from_content_tree(&dir, "content", "gasset")
        .expect("a project with no content tree yet is not an error");
    assert_eq!(registry.iter().count(), 0);
    std::fs::remove_dir_all(&dir).ok();
}
```

Extend that file's existing `use` line to:

```rust
use asset::content::{
    read_content_asset, read_content_asset_header, save_content_asset, write_content_asset,
    AssetRegistry, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p essential --test asset_registry`
Expected: FAIL — `read_content_asset_header`, `id_for_address`, `from_content_tree` do not exist.

- [ ] **Step 3: Add the prefix header reader**

In `crates/essential/src/assets/content.rs`, after `read_content_asset`:

```rust
/// Reads just the header of the content asset at `path`, never the payload.
///
/// A content tree holds whole textures and meshes — tens of megabytes each —
/// so indexing one by reading every file whole is not viable. This reads the
/// 8-byte magic-and-length prefix, then exactly `header_len` more bytes.
pub fn read_content_asset_header(path: &Path) -> anyhow::Result<ContentAssetHeader> {
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
    ) as usize;

    let mut header_bytes = vec![0u8; header_len];
    file.read_exact(&mut header_bytes).with_context(|| {
        format!(
            "content asset '{}' truncated: header claims {header_len} bytes",
            path.display()
        )
    })?;

    let header: ContentAssetHeader = bincode::deserialize(&header_bytes)
        .with_context(|| format!("failed to deserialize header of '{}'", path.display()))?;
    if header.format_version != CONTENT_FORMAT_VERSION {
        bail!(
            "unsupported content asset format version {} in '{}' (this build expects {CONTENT_FORMAT_VERSION})",
            header.format_version,
            path.display()
        );
    }
    Ok(header)
}
```

- [ ] **Step 4: Give `AssetRegistry` an address index**

Replace the `AssetRegistry` struct definition and its `parse`, and add the two new methods. The struct gains a second map kept in step with `entries`:

```rust
#[derive(Debug, Clone, Default)]
pub struct AssetRegistry {
    entries: BTreeMap<AssetId, String>,
    by_address: HashMap<String, AssetId>,
}
```

Add `use std::collections::HashMap;` to the file's imports.

`parse` builds both and rejects a duplicated address:

```rust
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let file: RegistryFile = toml::from_str(text).context("failed to parse asset registry")?;
        let mut registry = Self::default();
        for (hex, address) in file.assets {
            let id = AssetId::from_simple_hex(&hex)
                .map_err(|err| anyhow::anyhow!("invalid asset id '{hex}' in registry: {err}"))?;
            if let Some(existing) = registry.by_address.get(&address) {
                bail!(
                    "asset registry maps two ids ({} and {}) to the same address '{address}'",
                    existing.simple_hex(),
                    id.simple_hex()
                );
            }
            registry.insert(id, address);
        }
        Ok(registry)
    }
```

`insert` and `remove` maintain both maps:

```rust
    pub fn insert(&mut self, id: AssetId, address: impl Into<String>) {
        let address = address.into();
        if let Some(previous) = self.entries.insert(id, address.clone()) {
            self.by_address.remove(&previous);
        }
        self.by_address.insert(address, id);
    }

    pub fn remove(&mut self, id: AssetId) -> Option<String> {
        let address = self.entries.remove(&id)?;
        self.by_address.remove(&address);
        Some(address)
    }
```

And the new lookup:

```rust
    /// The id of the asset at `address`, for a path-based load. The inverse
    /// of [`AssetRegistry::get`].
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.by_address.get(address).copied()
    }
```

`save` is unchanged — it serializes `entries` only, so the on-disk shape does not change.

- [ ] **Step 5: Add the tree scan**

Also in `content.rs`:

```rust
    /// Builds a registry by scanning `<project_root>/<content_root>` for
    /// `*.<extension>` files and reading each one's header.
    ///
    /// This is the authoritative way to produce a registry: identity lives in
    /// the header, so a scan re-points an id at wherever its file actually is
    /// now — which is what lets a content asset be renamed or moved without
    /// breaking the references that name its id. An absent content tree is an
    /// empty registry, not an error.
    pub fn from_content_tree(
        project_root: &Path,
        content_root: &str,
        extension: &str,
    ) -> anyhow::Result<Self> {
        let root = project_root.join(content_root);
        let mut registry = Self::default();
        let mut source_of: HashMap<AssetId, String> = HashMap::new();

        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to read '{}'", dir.display()))
                }
            };
            for entry in entries {
                let path = entry
                    .with_context(|| format!("failed to read an entry of '{}'", dir.display()))?
                    .path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some(extension) {
                    continue;
                }

                let header = read_content_asset_header(&path)?;
                let address = path
                    .strip_prefix(project_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");

                if let Some(previous) = source_of.get(&header.asset_id) {
                    bail!(
                        "content tree is malformed: '{previous}' and '{address}' both carry asset id {}",
                        header.asset_id.simple_hex()
                    );
                }
                source_of.insert(header.asset_id, address.clone());
                registry.insert(header.asset_id, address);
            }
        }

        Ok(registry)
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p essential --test asset_registry`
Expected: PASS, all tests including the pre-existing ones.

- [ ] **Step 7: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/essential/src/assets/content.rs crates/essential/tests/asset_registry.rs
git commit -m "feat(essential): header prefix-reader, registry address index, tree scan

AssetRegistry can now be rebuilt from a content tree by reading each
file's header, and answers address -> id as well as id -> address.
read_content_asset_header reads only the prefix, so scanning a tree of
multi-megabyte assets stays cheap.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 2: `save_content_asset` mints and reuses

**Files:**
- Modify: `crates/essential/src/assets/content.rs`
- Test: `crates/essential/tests/asset_registry.rs`

**Interfaces:**
- Consumes: `read_content_asset_header` (Task 1).
- Produces: `save_content_asset` no longer derives the id from the address.

- [ ] **Step 1: Write the failing test**

Append to `crates/essential/tests/asset_registry.rs`:

```rust
#[test]
fn save_content_asset_mints_an_id_not_derived_from_the_address() {
    let dir = temp_root("save-mints");
    let address = "content/things/one.gasset";

    save_content_asset(&Thing, &dir, address).expect("save");

    let raw = std::fs::read(dir.join(address)).unwrap();
    let (header, _) = read_content_asset(&raw).expect("readable");
    assert_ne!(
        header.asset_id,
        AssetId::from_path(address),
        "identity must be minted, not derived from where the file happens to sit"
    );
    assert_eq!(
        AssetRegistry::load(&dir).unwrap().get(header.asset_id),
        Some(address),
        "the registry points at the minted id"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn saving_over_an_existing_asset_reuses_its_id() {
    let dir = temp_root("save-reuses");
    let address = "content/things/two.gasset";

    save_content_asset(&Thing, &dir, address).expect("first save");
    let first = read_content_asset_header(&dir.join(address)).unwrap().asset_id;

    save_content_asset(&Thing, &dir, address).expect("second save");
    let second = read_content_asset_header(&dir.join(address)).unwrap().asset_id;

    assert_eq!(
        first, second,
        "re-saving must keep the identity every existing reference names"
    );

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p essential --test asset_registry`
Expected: FAIL — `save_content_asset` still uses `AssetId::from_path(address)`, so the first test's `assert_ne!` fails.

- [ ] **Step 3: Mint or reuse in `save_content_asset`**

In `crates/essential/src/assets/content.rs`, replace the id line and the doc comment's identity claim:

```rust
/// Writes `value` as a content asset at `project_root/address`, creating
/// parent directories as needed, and upserts the asset registry so a
/// path-less (`AssetServer::load_by_id`) load can find it later.
///
/// The asset's id is *minted* the first time an address is written and
/// reused every time after, by reading the header already on disk — so a
/// re-save keeps the identity that existing references name. `address` is
/// the project-relative path (`"content/hero/body.gasset"`); `project_root`
/// is the source tree an editor saves into, which is deliberately *not* the
/// exe-relative runtime root — a save must land in the tree under version
/// control, not beside the binary where the next build overwrites it.
pub fn save_content_asset<A: Asset>(
    value: &A,
    project_root: &Path,
    address: &str,
) -> anyhow::Result<()> {
    let path = project_root.join(address);
    let asset_id = mint_or_reuse_id(&path)?;
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id,
        references: value.referenced_sub_assets(),
        kind: A::name().to_string(),
        provenance: None,
    };
    let payload = bincode::serialize(value).context("failed to serialize content asset payload")?;
    let bytes = write_content_asset(&header, &payload)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create '{}'", parent.display()))?;
    }
    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write content asset '{}'", path.display()))?;

    let mut registry = AssetRegistry::load(project_root)?;
    registry.insert(asset_id, address);
    registry.save(project_root)
}

/// The id to write at `path`: the one already in the file's header if a
/// content asset is there, otherwise a freshly minted one. Identity is
/// assigned once and then belongs to the asset, not to its location.
pub fn mint_or_reuse_id(path: &Path) -> anyhow::Result<AssetId> {
    if path.exists() {
        return Ok(read_content_asset_header(path)?.asset_id);
    }
    Ok(AssetId::new())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p essential --test asset_registry`
Expected: PASS.

Note: `crates/import/tests/end_to_end.rs::an_editor_saved_scene_round_trips` asserts `header.asset_id == AssetId::from_path(address)`. That assertion is now false. Delete that one line from it — the surrounding test (payload round-trips, `kind` matches, `provenance` is `None`) still holds and still passes.

- [ ] **Step 5: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/essential/src/assets/content.rs crates/essential/tests/asset_registry.rs crates/import/tests/end_to_end.rs
git commit -m "feat(essential): save_content_asset mints an id and reuses it on re-save

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 3: `import` mints ids and rebuilds the registry by scanning

**Files:**
- Modify: `crates/import/src/lib.rs`
- Test: `crates/import/tests/import_gltf.rs`

**Interfaces:**
- Consumes: `mint_or_reuse_id`, `AssetRegistry::from_content_tree` (Tasks 1-2).
- Produces: `import_source` writes minted ids and leaves the registry a full rebuild of the tree.
- Addresses are unchanged in this task — naming lands in Tasks 4-6.

- [ ] **Step 1: Write the failing tests**

In `crates/import/tests/import_gltf.rs`, replace the header-identity assertion inside `writes_content_assets_with_content_path_cross_references` — the line asserting `header.asset_id == AssetId::from_path("content/triangle/scene.gasset")` — with:

```rust
    assert_ne!(
        header.asset_id,
        AssetId::from_path("content/triangle/scene.gasset"),
        "identity is minted, not derived from the address"
    );
```

And append:

```rust
#[test]
fn re_importing_reuses_the_ids_already_on_disk() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-reuse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    let first = import::import_source(&fixture(), &project_root, &Default::default())
        .expect("first import");
    let ids_before: Vec<AssetId> = first
        .iter()
        .map(|a| {
            read_content_asset_header(&project_root.join(&a.address))
                .unwrap()
                .asset_id
        })
        .collect();

    let second = import::import_source(&fixture(), &project_root, &Default::default())
        .expect("second import");
    let ids_after: Vec<AssetId> = second
        .iter()
        .map(|a| {
            read_content_asset_header(&project_root.join(&a.address))
                .unwrap()
                .asset_id
        })
        .collect();

    assert_eq!(
        ids_before, ids_after,
        "a re-import must keep every id, or every baked cross-reference breaks"
    );

    std::fs::remove_dir_all(&project_root).ok();
}

#[test]
fn import_writes_a_registry_rebuilt_from_the_tree() {
    let project_root =
        std::env::temp_dir().join(format!("import-gltf-rebuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project_root);
    std::fs::create_dir_all(&project_root).unwrap();

    // A stale entry that no file backs. A merge would keep it; a rebuild
    // from the tree must drop it.
    let mut stale = AssetRegistry::new();
    stale.insert(AssetId::new(), "content/gone/removed.gasset");
    stale.save(&project_root).unwrap();

    let written = import::import_source(&fixture(), &project_root, &Default::default())
        .expect("import");

    let registry = AssetRegistry::load(&project_root).expect("registry loads");
    assert_eq!(
        registry.iter().count(),
        written.len(),
        "the registry is exactly the tree, with no stale entries left over"
    );
    for asset in &written {
        let id = read_content_asset_header(&project_root.join(&asset.address))
            .unwrap()
            .asset_id;
        assert_eq!(registry.get(id), Some(asset.address.as_str()));
        assert_eq!(registry.id_for_address(&asset.address), Some(id));
    }

    std::fs::remove_dir_all(&project_root).ok();
}
```

Extend that file's `content` import to:

```rust
use asset::content::{
    read_content_asset, read_content_asset_header, AssetRegistry, ImportProvenance,
};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p import --test import_gltf`
Expected: FAIL — ids are still `from_path`-derived, and the stale entry survives.

- [ ] **Step 3: Mint through a memoised resolver**

In `crates/import/src/lib.rs`, replace the resolver construction inside `import_source` with a memoising one, and thread it into the write loop. Add to the imports:

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use asset::content::{
    mint_or_reuse_id, write_content_asset, AssetRegistry, ContentAssetHeader, ImportProvenance,
    CONTENT_FORMAT_VERSION,
};
```

Then, replacing the current `let resolver: SubAssetIdResolver = …` block:

```rust
    // Identity is minted per address and reused from the header already on
    // disk, so it must be decided once per sub-asset name and shared between
    // the cross-references baked during the importer pass and the headers
    // written afterwards. `content_address` is a pure function of the name,
    // so this memo is the single place an id is decided for this run.
    let minted: Arc<Mutex<HashMap<String, AssetId>>> = Arc::new(Mutex::new(HashMap::new()));

    let owned_source = source.to_path_buf();
    let owned_config = config.clone();
    let owned_root = project_root.to_path_buf();
    let memo = Arc::clone(&minted);
    let resolver: SubAssetIdResolver = Box::new(move |sub_name| {
        let address = content_address(&owned_config, &owned_source, sub_name);
        let mut memo = memo.lock().expect("mint memo poisoned");
        if let Some(id) = memo.get(&address) {
            return *id;
        }
        // A failure to read an existing header would mean a corrupt tree;
        // mint rather than panic in a resolver that cannot return an error,
        // and let the write below surface the real problem.
        let id = mint_or_reuse_id(&owned_root.join(&address)).unwrap_or_else(|_| AssetId::new());
        memo.insert(address, id);
        id
    });
```

The write loop's header construction is unchanged — `sub_asset.asset_id` already carries whatever the resolver returned.

- [ ] **Step 4: Rebuild the registry from the tree**

Replace the registry handling in `import_source`. Delete the `let mut registry = AssetRegistry::load(project_root)?;` line before the loop and the `registry.insert(…)` call inside it, and replace the `registry.save(project_root)?;` after the loop with:

```rust
    // The registry is derived data: rebuilt from the tree rather than merged
    // into, so a content asset that was renamed or moved by hand is
    // re-pointed at where it actually is, and an entry whose file is gone
    // does not linger.
    AssetRegistry::from_content_tree(project_root, &config.root, &config.extension)?
        .save(project_root)?;
```

- [ ] **Step 5: Add the end-to-end minted-id test**

The spec asks for one test that walks the whole loop: import, load a
sub-asset by address, then load a second one by the id *baked into the
first's payload* — which is the thing minting has to get right, and which
no unit test covers. Append to `crates/import/tests/end_to_end.rs`:

```rust
#[test]
fn a_baked_reference_resolves_to_the_minted_id_of_its_target() {
    let root = std::env::temp_dir().join(format!("content-minted-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let written = import::import_source(&fixture(), &root, &Default::default()).expect("import");

    let scene_address = &written
        .iter()
        .find(|a| a.sub_asset_name == "scene")
        .expect("the fixture emits a scene")
        .address;
    let mesh_address = &written
        .iter()
        .find(|a| a.sub_asset_name.starts_with("mesh/"))
        .expect("the fixture emits a mesh")
        .address;

    // Load the scene by address, exactly as game code would.
    let bytes = pollster::block_on(load_content_asset_bytes(
        &ContentAssetRoot::Directory(root.clone()),
        scene_address,
        Scene::name(),
    ))
    .expect("the scene loads by address");
    let scene: Scene = bincode::deserialize(&bytes).expect("payload is a Scene");

    // The id it references is the mesh's *minted* id — not derivable from
    // either address — and the registry is what connects the two.
    let referenced = scene
        .referenced_assets
        .first()
        .copied()
        .expect("the scene references its mesh");
    let registry = AssetRegistry::load(&root).expect("registry loads");
    assert_eq!(
        registry.get(referenced),
        Some(mesh_address.as_str()),
        "a baked reference must resolve through the registry to the file it names"
    );
    assert_eq!(
        read_content_asset_header(&root.join(mesh_address)).unwrap().asset_id,
        referenced,
        "and that file's own header must carry the same id"
    );

    std::fs::remove_dir_all(&root).ok();
}
```

Add to that file's imports: `use asset::content::{read_content_asset_header, AssetRegistry};`

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p import`
Expected: PASS — `import_gltf` (4 tests) and `end_to_end` (3 tests).

- [ ] **Step 7: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/import/src/lib.rs crates/import/tests
git commit -m "feat(import): mint asset ids, rebuild the registry from the tree

Ids are minted per address and reused from the header on disk, decided
once per run through a memoised resolver so baked cross-references and
written headers agree. The registry is now a full rebuild of the content
tree, which is what lets a hand-renamed asset keep its identity.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 4: `content_address` turns name segments into directories

**Files:**
- Modify: `crates/import/src/config.rs` (including its `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `content_address` maps sub-asset name `a/b/c` to `<root>/<stem>/a/b/c.<ext>`, each segment sanitised; `config::sanitize_segment(&str) -> String` for importers to share.
- Consumes: nothing new. Addresses for today's index-based names are unchanged in shape (`mesh/0` → `content/x/mesh/0.gasset` instead of `content/x/mesh_0.gasset`) — the examples are re-imported in Tasks 8-10.

- [ ] **Step 1: Write the failing tests**

Replace the `#[cfg(test)] mod tests` block in `crates/import/src/config.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_config_fills_the_missing_field_from_default() {
        let config: ContentConfig = toml::from_str(r#"root = "content""#).expect("parses");
        assert_eq!(config.root, "content");
        assert_eq!(
            config.extension, "gasset",
            "the unset field takes its default"
        );
    }

    #[test]
    fn name_segments_become_directories() {
        assert_eq!(
            content_address(&ContentConfig::default(), Path::new("hero.gltf"), "mesh/Body"),
            "content/hero/mesh/Body.gasset"
        );
    }

    #[test]
    fn a_name_without_a_category_is_a_file_at_the_source_root() {
        assert_eq!(
            content_address(&ContentConfig::default(), Path::new("hero.gltf"), "scene"),
            "content/hero/scene.gasset"
        );
    }

    #[test]
    fn every_segment_is_sanitized() {
        assert_eq!(
            content_address(
                &ContentConfig::default(),
                Path::new("hero.gltf"),
                "animation/Idle Loop"
            ),
            "content/hero/animation/Idle_Loop.gasset"
        );
        assert_eq!(
            content_address(
                &ContentConfig::default(),
                Path::new("hero.gltf"),
                "skeleton/mixamorig:Hips"
            ),
            "content/hero/skeleton/mixamorig_Hips.gasset"
        );
    }

    #[test]
    fn sanitize_segment_collapses_runs_and_trims() {
        assert_eq!(sanitize_segment("Idle  Loop"), "Idle_Loop");
        assert_eq!(sanitize_segment("a/b\\c#d"), "a_b_c_d");
        assert_eq!(sanitize_segment("__lead and trail__"), "lead_and_trail");
        assert_eq!(sanitize_segment(".hidden."), "hidden");
        assert_eq!(sanitize_segment("Body.001"), "Body.001", "dots inside are kept");
        assert_eq!(sanitize_segment("???"), "", "nothing usable survives");
    }

    #[test]
    fn an_unusable_segment_falls_back_to_a_placeholder() {
        assert_eq!(
            content_address(&ContentConfig::default(), Path::new("hero.gltf"), "mesh/???"),
            "content/hero/mesh/_.gasset",
            "a name that sanitizes away still has to produce a usable path"
        );
    }

    #[test]
    fn a_source_with_no_stem_still_produces_an_address() {
        assert_eq!(
            content_address(&ContentConfig::default(), Path::new(""), "scene"),
            "content/asset/scene.gasset"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p import --lib`
Expected: FAIL — `sanitize_segment` does not exist and `content_address` still flattens.

- [ ] **Step 3: Rewrite `content_address` and add `sanitize_segment`**

In `crates/import/src/config.rs`, replace the whole `content_address` function with:

```rust
/// `<content-root>/<source-stem>/<name segments…>.<ext>`, the
/// project-relative address a sub-asset is written to and referenced by.
///
/// A `/` in the sub-asset name is a real path separator: `mesh/Body` becomes
/// `…/mesh/Body.gasset`, so a source's assets group by category on disk.
/// Every segment is sanitised, because these names come from artist-authored
/// source files and end up as filenames.
pub fn content_address(config: &ContentConfig, source: &Path, sub_name: &str) -> String {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "asset".to_string());

    let mut segments: Vec<String> = sub_name
        .split('/')
        .map(sanitize_segment)
        .map(|segment| if segment.is_empty() { "_".to_string() } else { segment })
        .collect();
    if segments.is_empty() {
        segments.push("_".to_string());
    }
    let leaf = segments.pop().expect("segments is never empty");

    let mut address = format!("{}/{}", config.root, sanitize_segment(&stem));
    for segment in segments {
        address.push('/');
        address.push_str(&segment);
    }
    format!("{address}/{leaf}.{}", config.extension)
}

/// Reduces one path segment to `[A-Za-z0-9._-]`: every other character
/// becomes `_`, runs of `_` collapse, and leading/trailing `_` and `.` are
/// trimmed. Returns an empty string when nothing usable is left, which the
/// caller replaces with a placeholder.
pub fn sanitize_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for ch in segment.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches(|c| c == '_' || c == '.').to_string()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p import --lib`
Expected: PASS (8 tests).

Then run `cargo test -p import` — `import_gltf` and `end_to_end` assert literal addresses like `"content/triangle/mesh_0.gasset"`, which are now `"content/triangle/mesh/0.gasset"`. Update every such literal in both files to the new shape.

- [ ] **Step 5: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/import/src/config.rs crates/import/tests
git commit -m "feat(import): sub-asset name segments become directories, sanitized

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 5: glTF importer names sub-assets from the source

**Files:**
- Modify: `crates/gltf-loader/src/gltf_importer.rs`
- Modify: `crates/gltf-loader/Cargo.toml` (add the `import` config dependency — see Step 3)
- Test: `crates/gltf-loader/tests/gltf_importer.rs`

**Interfaces:**
- Consumes: `import::config::sanitize_segment` (Task 4).
- Produces: sub-asset names of the form `mesh/<Name>`, `material/<Name>`, `animation/<Name>`, `skeleton/<Name>`, `texture/<Name>`(`_linear`), with index fallbacks and `.1`/`.2` disambiguation.

**Note on the shape of this change:** `PrimRef` and `SkinInfo` currently store *indices* and every reference site rebuilds the name with `format!("mesh/{}", idx)`. Names are not derivable from an index, so both structs change to carry the resolved sub-asset **name**, and the reference sites pass that name straight to `ctx.sub_asset_id`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/gltf-loader/tests/gltf_importer.rs`:

```rust
#[test]
fn sub_assets_are_named_from_the_source() {
    // The fixtures were checked before writing this: triangle.gltf names its
    // *material* ("Red") but leaves its mesh unnamed, and skinned.gltf names
    // its animation ("Wiggle") but not its mesh or skin. So this asserts both
    // halves of the policy against real data — a name where the source has
    // one, an index where it does not.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle.gltf");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("triangle.gltf"));
    GltfImporter.import(&fixture, &mut ctx).expect("import");
    let names: Vec<String> = ctx
        .into_parts()
        .sub_assets
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert!(
        names.iter().any(|n| n == "material/Red"),
        "the fixture's material is named 'Red', so its sub-asset must be too \
         rather than material/0; got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "mesh/0"),
        "the fixture's mesh is unnamed, so it falls back to its index; got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "scene"),
        "the synthetic scene sub-asset keeps its fixed name; got: {names:?}"
    );
}

#[test]
fn a_named_animation_is_addressed_by_its_name() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skinned.gltf");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("skinned.gltf"));
    GltfImporter.import(&fixture, &mut ctx).expect("import");
    let names: Vec<String> = ctx
        .into_parts()
        .sub_assets
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert!(
        names.iter().any(|n| n == "animation/Wiggle"),
        "the fixture's animation is named 'Wiggle'; got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "skeleton/0"),
        "its skin is unnamed, so the skeleton falls back to its index; got: {names:?}"
    );
}

#[test]
fn an_unnamed_sub_asset_falls_back_to_its_index() {
    // `SubAssetNamer` is the single place naming policy lives; drive it
    // directly for the cases the fixture cannot produce.
    let mut namer = SubAssetNamer::default();
    assert_eq!(namer.name("mesh", None, 0), "mesh/0");
    assert_eq!(namer.name("mesh", Some(""), 1), "mesh/1");
    assert_eq!(namer.name("mesh", Some("???"), 2), "mesh/2");
}

#[test]
fn duplicate_names_are_disambiguated_within_a_category() {
    let mut namer = SubAssetNamer::default();
    assert_eq!(namer.name("mesh", Some("Body"), 0), "mesh/Body");
    assert_eq!(namer.name("mesh", Some("Body"), 1), "mesh/Body.1");
    assert_eq!(namer.name("mesh", Some("Body"), 2), "mesh/Body.2");
    assert_eq!(
        namer.name("material", Some("Body"), 0),
        "material/Body",
        "categories are disambiguated independently"
    );
}

#[test]
fn names_are_sanitized_before_disambiguation() {
    let mut namer = SubAssetNamer::default();
    assert_eq!(namer.name("skeleton", Some("mixamorig:Hips"), 0), "skeleton/mixamorig_Hips");
    assert_eq!(
        namer.name("skeleton", Some("mixamorig Hips"), 1),
        "skeleton/mixamorig_Hips.1",
        "two names that sanitize alike still have to differ on disk"
    );
}
```

Add to that file's imports: `use gltf_loader::gltf_importer::{GltfImporter, SubAssetNamer};`

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p gltf-loader`
Expected: FAIL — `SubAssetNamer` does not exist; names are index-based.

- [ ] **Step 3: Add the namer**

`gltf-loader` **cannot** depend on `import` — `crates/import/Cargo.toml` already
lists `gltf-loader` (its `registered_importers` constructs `GltfImporter`), so
that edge is a circular dependency Cargo rejects outright. Do not try it.

Instead, add a private copy of the sanitiser to `crates/gltf-loader/src/gltf_importer.rs`,
with a comment naming the original — the same pattern the examples' `build.rs`
uses to restate `REGISTRY_FILE_NAME`:

```rust
/// Same rules as `import::config::sanitize_segment`, restated here because
/// `import` depends on this crate and the edge cannot go both ways.
fn sanitize_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for ch in segment.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches(|c| c == '_' || c == '.').to_string()
}
```

In `crates/gltf-loader/src/gltf_importer.rs`, add:

```rust
/// Assigns sub-asset names within one source file: the artist's name when
/// the source has one, an index when it does not, and a `.1`/`.2` suffix
/// when two assets in the same category want the same name.
///
/// Naming policy lives here rather than at each emit site so every
/// `ctx.emit` and every `ctx.sub_asset_id` in this importer agrees.
#[derive(Default)]
pub struct SubAssetNamer {
    used: HashMap<String, u32>,
}

impl SubAssetNamer {
    /// `<category>/<name-or-index>`, disambiguated within `category`.
    pub fn name(&mut self, category: &str, source_name: Option<&str>, index: usize) -> String {
        let sanitized = source_name.map(sanitize_segment).unwrap_or_default();
        let base = if sanitized.is_empty() {
            index.to_string()
        } else {
            sanitized
        };

        let key = format!("{category}/{base}");
        let seen = self.used.entry(key.clone()).or_insert(0);
        *seen += 1;
        if *seen == 1 {
            key
        } else {
            format!("{key}.{}", *seen - 1)
        }
    }
}
```

with `use import::config::sanitize_segment;` (or the private copy per the note above) and `HashMap` already imported.

- [ ] **Step 4: Name every emitted sub-asset**

All of these are in `GltfImporter::import`. Create one namer at the top of the function: `let mut namer = SubAssetNamer::default();`

1. **Textures.** The name must be identical at the two places a texture is referred to — the `texture_ref` closure (which bakes the id into a material) and the emit loop — and the namer must be consulted exactly once per texture, or it would disambiguate a texture against itself. So the closure populates a map lazily and the emit loop reads it.

Declare the map *before* the block that owns `texture_ref`, so it outlives the closure:

```rust
        let mut texture_names: HashMap<TextureKey, String> = HashMap::new();
```

and inside that block:

```rust
            let mut texture_ref =
                |texture: gltf::Texture<'_>, srgb: bool| -> AssetHandle<Texture> {
                    let image_index = texture.source().index();
                    let key = TextureKey { image_index, srgb };
                    needed_textures.insert(key);
                    let name = texture_names.entry(key).or_insert_with(|| {
                        let image_name = texture.source().name().map(str::to_string);
                        let base = namer.name("texture", image_name.as_deref(), image_index);
                        if srgb { base } else { format!("{base}_linear") }
                    });
                    AssetHandle::weak(ctx.sub_asset_id(name))
                };
```

Both `texture_names` and `namer` are captured mutably by the closure, so `namer` cannot be used elsewhere until the block ends — order the naming of materials/meshes/skeletons/animations *after* it, which is already the file's order.

The emit loop then becomes:

```rust
        for key in &needed_textures {
            let rgba = decoded_images[key.image_index].to_rgba8();
            let texture = Texture { /* unchanged */ };
            ctx.emit(&texture_names[key], &texture)?;
        }
```

Delete the free function `texture_name`.

2. **Materials.** Resolve a name per material index before the emit loop so `PrimRef` can carry it:

```rust
        let material_names: Vec<String> = document
            .materials()
            .enumerate()
            .map(|(index, material)| namer.name("material", material.name(), index))
            .collect();

        for (index, material) in materials.iter().enumerate() {
            ctx.emit(&material_names[index], material)?;
        }
```

The synthetic default material becomes `material/default`:

```rust
        let default_material_name = namer.name("material", Some("default"), materials.len());
        …
        if default_material_used {
            ctx.emit(&default_material_name, &StandardMaterial::default())?;
        }
```

3. **Meshes.** `PrimRef` changes to carry names:

```rust
struct PrimRef {
    mesh_sub_asset: String,
    material_sub_asset: String,
}
```

and the mesh loop names each primitive, suffixing `.primitive<k>` only when a mesh has more than one:

```rust
        let mut mesh_counter: usize = 0;
        let mut mesh_prims: Vec<Vec<PrimRef>> = Vec::new();
        for mesh in document.meshes() {
            let primitive_count = mesh.primitives().count();
            let mesh_name = namer.name("mesh", mesh.name(), mesh_counter);
            let mut prims = Vec::new();
            for (k, gltf_primitive) in mesh.primitives().enumerate() {
                let m = load_primitive(source_path, mesh.name(), &buffers, &gltf_primitive)?;
                // One primitive keeps the mesh's own name; several share it
                // with the same `.primitiveN` suffix the generated child
                // nodes use, so they sort together.
                let sub_asset = if primitive_count > 1 {
                    format!("{mesh_name}.primitive{k}")
                } else {
                    mesh_name.clone()
                };
                ctx.emit(&sub_asset, &m)?;

                let material_sub_asset = match gltf_primitive.material().index() {
                    Some(material_index) => material_names[material_index].clone(),
                    None => {
                        default_material_used = true;
                        default_material_name.clone()
                    }
                };
                prims.push(PrimRef {
                    mesh_sub_asset: sub_asset,
                    material_sub_asset,
                });
                mesh_counter += 1;
            }
            mesh_prims.push(prims);
        }
```

4. **Skeletons.** `SkinInfo` gains the name; keep `skeleton_index` for the existing lookups:

```rust
struct SkinInfo {
    skeleton_index: usize,
    skeleton_sub_asset: String,
    bones: Vec<SceneEntityRef>,
    bone_ids: Vec<Uuid>,
}
```

In the skin loop: `let skeleton_sub_asset = namer.name("skeleton", skin.name(), skin_index);`, emit with it, and store it on the `SkinInfo`.

5. **Animations.** `ctx.emit(&namer.name("animation", animation.name(), animation_index), &animation_clip)?;`

6. **Reference sites.** Every `ctx.sub_asset_id(&format!("…"))` that rebuilt a name from an index now passes the stored name:
   - the single-primitive branch: `ctx.sub_asset_id(&p.mesh_sub_asset)` and `ctx.sub_asset_id(&p.material_sub_asset)`
   - the multi-primitive branch: the same two, inside the `for (k, p)` loop
   - both skeleton reference sites: `ctx.sub_asset_id(&info.skeleton_sub_asset)` (found via the existing `skins.iter().find(|info| info.skeleton_index == skin_index)`)

7. **The scene** keeps `ctx.emit("scene", …)`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p gltf-loader`
Expected: PASS after updating the pre-existing tests' name literals. The
fixtures were inspected while writing this plan, so here is exactly what
moves and what does not — but still run the tests and read each failure
rather than trusting this list blindly:

| Site | Today | After | Why |
|---|---|---|---|
| `gltf_importer.rs:51` | `material/0` | `material/Red` | `triangle.gltf` names its material "Red" |
| `gltf_importer.rs:134` | `animation/0` | `animation/Wiggle` | `skinned.gltf` names its animation "Wiggle" |
| `gltf_importer.rs:215` | `["mesh/0", "mesh/1", …]` | `["mesh/0.primitive0", "mesh/0.primitive1", …]` | `triangle_two_prims.gltf`'s mesh is unnamed (so `mesh/0`) and has two primitives, so both take the `.primitiveN` suffix |
| `gltf_importer.rs:251-252` | `#mesh/0`, `#mesh/1` | `#mesh/0.primitive0`, `#mesh/0.primitive1` | same |
| `gltf_importer.rs:47,130` | `mesh/0`, `skeleton/0` | unchanged | `triangle.gltf`'s mesh and `skinned.gltf`'s skin are both unnamed, so both keep the index fallback |
| `gltf_importer.rs:69,70,79,164` | `#mesh/0`, `#skeleton/0` | unchanged | same reason |

Do **not** touch `crates/asset-import/tests/*`: those build an
`ImportContext` directly and pass their own literal sub-asset names
(`ctx.sub_asset_id("mesh/0")`, `ctx.emit("material/0", …)`). They never run
`GltfImporter`, so naming policy does not reach them.

- [ ] **Step 6: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/gltf-loader
git commit -m "feat(gltf-loader): name sub-assets from the source, not by index

Meshes, materials, animations, skeletons and textures take the name the
glTF gives them, falling back to the index when unnamed and suffixing
.1/.2 within a category on collision. PrimRef and SkinInfo carry the
resolved name so reference sites no longer rebuild it from an index.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 6: OBJ and image importers name their sub-assets

**Files:**
- Modify: `crates/obj-loader/src/obj_importer.rs`, `crates/render/src/importers/image_importer.rs`
- Test: `crates/obj-loader/tests/obj_importer.rs`, `crates/render/tests/image_importer.rs`, `crates/render/tests/texture_pipeline_e2e.rs`

**Interfaces:**
- Consumes: the same naming policy as Task 5. `SubAssetNamer` lives in `gltf-loader`; rather than depend on it, both importers use the two-line equivalent inline (they have at most one category each with real collision risk).

- [ ] **Step 1: Write the failing test**

Append to `crates/obj-loader/tests/obj_importer.rs`:

```rust
#[test]
fn meshes_are_named_from_the_obj_object_name() {
    // The fixture declares `o Square`, so tobj gives the model that name.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/square.obj");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("square.obj"));

    ObjImporter.import(&fixture, &mut ctx).expect("import");
    let names: Vec<String> = ctx
        .into_parts()
        .sub_assets
        .into_iter()
        .map(|s| s.name)
        .collect();

    assert!(
        names.iter().any(|n| n == "mesh/Square"),
        "an OBJ object named 'Square' must produce mesh/Square, not mesh/0; got: {names:?}"
    );
}
```

The file inlines its fixture path exactly this way in its existing tests — match that rather than adding a helper.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p obj-loader`
Expected: FAIL — meshes are `mesh/<index>`.

- [ ] **Step 3: Name OBJ meshes**

In `crates/obj-loader/src/obj_importer.rs`, resolve a name per model index once, before both loops, and use it in the emit loop and the `sub_asset_id` site:

```rust
        // tobj gives every model the `o`/`g` name from the file; fall back to
        // the index when a file has none. Duplicates get the same `.N`
        // treatment the glTF importer applies.
        let mut used: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let mesh_names: Vec<String> = models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let base = sanitize_obj_name(&model.name);
                let base = if base.is_empty() { index.to_string() } else { base };
                let key = format!("mesh/{base}");
                let seen = used.entry(key.clone()).or_insert(0);
                *seen += 1;
                if *seen == 1 { key } else { format!("{key}.{}", *seen - 1) }
            })
            .collect();
```

with a private `sanitize_obj_name` mirroring `sanitize_segment`'s rules (same body; a comment pointing at `import::config::sanitize_segment` as the original). Then `ctx.emit(&mesh_names[index], &mesh)?;` and `ctx.sub_asset_id(&mesh_names[index])`.

`material/<mtl stem>` and `scene` are already name-based — leave them, but pass the stem through `sanitize_obj_name` when building `material/{mtl_stem}` at both its emit and reference site.

- [ ] **Step 4: Name the image importer's sub-asset**

In `crates/render/src/importers/image_importer.rs`, change `ctx.emit("main", &texture)` to `ctx.emit("texture/main", &texture)`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p obj-loader -p render`
Expected: PASS. `crates/render/tests/texture_pipeline_e2e.rs` writes its content asset at a hand-built address and asserts on it — update its `address` constant from `"content/swatch/main.gasset"` to `"content/swatch/texture/main.gasset"` to match what `content_address` now produces for `texture/main`.

- [ ] **Step 6: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/obj-loader crates/render
git commit -m "feat(importers): name OBJ meshes from the object name; image emits texture/main

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 7: Runtime — preloaded registry, address-resolved loads, plugin builder

**Files:**
- Modify: `crates/essential/src/assets/asset_server.rs` (including its `#[cfg(test)] mod tests`)
- Modify: `crates/app/src/plugins/mod.rs`, `crates/app/Cargo.toml`
- Modify: `src/lib.rs` (`DefaultPlugins`)
- Modify: `examples/render-test/src/main.rs`, `examples/tech-demo/src/main.rs`, `examples/animation-test/src/main.rs`

**Interfaces:**
- Consumes: `AssetRegistry::id_for_address` (Task 1), `utils::load_registry` (already exists).
- Produces: `AssetServer::{set_registry, registry}`; `AssetServer::resolve_by_id` becomes synchronous; `AssetManagerPlugin::{new, with_content_root}`; `DefaultPlugins::with_content_root`.
- The examples switch to the builder **in this task**, so they never run against a cleared registry cache.

- [ ] **Step 1: Write the failing tests**

Replace the three `resolve_by_id_*` tests in `crates/essential/src/assets/asset_server.rs`'s test module (they currently rely on the lazy async load) and add address-resolution coverage:

```rust
    #[test]
    fn resolve_by_id_finds_a_registered_asset() {
        let id = AssetId::from_path("content/hero/scene.gasset");
        let mut registry = AssetRegistry::new();
        registry.insert(id, "content/hero/scene.gasset");

        let server = AssetServer::new();
        server.set_registry(registry);

        assert_eq!(
            server.resolve_by_id(id).as_deref(),
            Some("content/hero/scene.gasset")
        );
    }

    #[test]
    fn resolve_by_id_returns_none_for_an_unregistered_id() {
        let server = AssetServer::new();
        server.set_registry(AssetRegistry::new());
        assert_eq!(server.resolve_by_id(AssetId::new()), None);
    }

    #[test]
    fn resolve_by_id_returns_none_when_no_registry_was_ever_loaded() {
        let server = AssetServer::new();
        assert_eq!(server.resolve_by_id(AssetId::new()), None);
    }

    #[test]
    fn id_for_address_resolves_a_path_load_to_the_minted_id() {
        // The id is deliberately unrelated to the address, which is the whole
        // point of minting: only the registry can connect the two.
        let minted = AssetId::new();
        let mut registry = AssetRegistry::new();
        registry.insert(minted, "content/hero/scene.gasset");

        let server = AssetServer::new();
        server.set_registry(registry);

        assert_eq!(
            server.id_for_address("content/hero/scene.gasset"),
            Some(minted)
        );
        assert_eq!(server.id_for_address("content/hero/absent.gasset"), None);
    }
```

Also update `load_by_id_loads_a_registered_content_asset` and `load_by_id_fails_for_an_unregistered_id`: they currently call `set_content_root` and rely on the lazy load. Add `server.set_registry(AssetRegistry::load(&dir).expect("registry"))` after the `save_content_asset` call, so the preload is explicit.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p essential --lib`
Expected: FAIL — `set_registry` and `id_for_address` do not exist; `resolve_by_id` is `async`.

- [ ] **Step 3: Make the registry explicit on `AssetServer`**

In `crates/essential/src/assets/asset_server.rs`:

Add next to `set_content_root`:

```rust
    /// Installs the asset registry this server resolves addresses and
    /// path-less ids through. `AssetManagerPlugin` calls this once during
    /// startup; there is no lazy load, so a server without a registry
    /// cannot resolve anything.
    pub fn set_registry(&self, registry: AssetRegistry) {
        *self.data.registry.write().unwrap() = Some(Arc::new(registry));
    }

    /// The id of the asset at `address`, or `None` if no registry is loaded
    /// or the address is not in it.
    pub fn id_for_address(&self, address: &str) -> Option<AssetId> {
        self.data
            .registry
            .read()
            .unwrap()
            .as_ref()?
            .id_for_address(address)
    }
```

Replace `resolve_by_id` with a synchronous lookup (the lazy load is gone — the plugin preloads):

```rust
    /// Resolves `id` to its content-tree address through the registry, for a
    /// path-less (`load_by_id`) load.
    fn resolve_by_id(&self, id: AssetId) -> Option<String> {
        self.data
            .registry
            .read()
            .unwrap()
            .as_ref()?
            .get(id)
            .map(str::to_owned)
    }
```

In `request_load`'s spawned task, drop the `.await` on that call: `match server.resolve_by_id(id) {`.

`set_content_root` keeps clearing the registry cache — an override that moves the content root invalidates the index built for the old one.

- [ ] **Step 4: Resolve path loads through the registry**

In `load_internal`, replace the id derivation. Today:

```rust
        let id = match self.data.path_to_id.write().unwrap().entry(path.clone()) {
            std::collections::hash_map::Entry::Occupied(occupied_entry) => *occupied_entry.get(),
            std::collections::hash_map::Entry::Vacant(vacant_entry) => {
                *vacant_entry.insert(AssetId::from_path(&path.address()))
            }
        };
```

becomes:

```rust
        let address = path.address();
        let id = match self.data.path_to_id.read().unwrap().get(&path) {
            Some(id) => Some(*id),
            None => self.id_for_address(&address),
        };
        let Some(id) = id else {
            // Identity is minted at import time and only the registry connects
            // an address to it, so an address that is not in the registry
            // cannot be loaded at all — there is nothing to derive.
            if self.data.registry.read().unwrap().is_none() {
                log::error!(
                    "asset registry not loaded; register AssetManagerPlugin with a content root before loading '{address}'"
                );
            } else {
                log::error!("no asset registered at '{address}' — run import");
            }
            return self.data.handle_provider.request_handle(AssetId::new(), Some(path));
        };
        self.data.path_to_id.write().unwrap().insert(path.clone(), id);
```

The rest of `load_internal` (the pending/loaded check, `request_load`, `request_handle`) is unchanged.

- [ ] **Step 5: Preload the registry in `AssetManagerPlugin`**

Add `pollster = "0.4.0"` to `[dependencies]` in `crates/app/Cargo.toml`.

Replace `AssetManagerPlugin` in `crates/app/src/plugins/mod.rs`:

```rust
use std::sync::{Arc, Mutex};

use asset::content::AssetRegistry;
use asset::ContentAssetRoot;

/// Plugin that inserts an [`AssetServer`] resource and the asset-event
/// handler, and preloads the asset registry the server resolves through.
///
/// The registry read is async (a file natively, an HTTP fetch on wasm), so
/// it uses the same `build`/`ready`/`finish` handshake `RenderPlugin` uses
/// for device init: `build` starts it, `ready` reports when it has landed,
/// `finish` installs it. `AssetServer::load` stays synchronous because by
/// the time any system runs, the index is already in memory.
#[derive(Default)]
pub struct AssetManagerPlugin {
    content_root: Option<ContentAssetRoot>,
}

impl AssetManagerPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves content assets against `root` instead of
    /// [`ContentAssetRoot::default_for_platform`].
    pub fn with_content_root(root: ContentAssetRoot) -> Self {
        Self {
            content_root: Some(root),
        }
    }
}

#[derive(ecs::resource::Resource)]
struct FutureAssetRegistry(Arc<Mutex<Option<AssetRegistry>>>);

impl Plugin for AssetManagerPlugin {
    fn build(&self, app: &mut App) {
        let server = AssetServer::new();
        if let Some(root) = self.content_root.clone() {
            server.set_content_root(root);
        }
        let root = server.content_root();
        app.insert_resource(server);
        app.register_event::<AssetLifetimeEvent>();
        app.add_system(LateUpdate, handle_asset_load_events);

        let slot = Arc::new(Mutex::new(None));
        app.insert_resource(FutureAssetRegistry(Arc::clone(&slot)));

        let load = async move {
            // A registry that fails to load becomes an empty one: the slot
            // must always fill or `ready` never returns true and the app
            // hangs. Every subsequent load then fails with its own message.
            let registry = match asset::utils::load_registry(&root).await {
                Ok(registry) => registry,
                Err(error) => {
                    log::error!("failed to load the asset registry: {error:#}");
                    AssetRegistry::default()
                }
            };
            *slot.lock().unwrap() = Some(registry);
        };

        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(load);
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(load);
    }

    fn ready(&self, app: &App) -> bool {
        app.get_resource::<FutureAssetRegistry>()
            .and_then(|future| future.0.try_lock().map(|slot| slot.is_some()).ok())
            .unwrap_or(true)
    }

    fn finish(&self, app: &mut App) {
        let Some(registry) = app
            .remove_resource::<FutureAssetRegistry>()
            .and_then(|future| future.0.lock().unwrap().take())
        else {
            return;
        };
        if let Some(server) = app.get_resource::<AssetServer>() {
            server.set_registry(registry);
        }
    }
}
```

Check how `Resource` is derived elsewhere in this crate and match it — if `ecs::resource::Resource` is a derive macro imported differently here, use the form the file's other resources use.

- [ ] **Step 6: Forward the root through `DefaultPlugins`**

In `src/lib.rs`:

```rust
/// Registers all standard engine plugins in the conventional order.
#[derive(Default)]
pub struct DefaultPlugins {
    headless: bool,
    content_root: Option<asset::ContentAssetRoot>,
}

impl DefaultPlugins {
    pub fn headless() -> Self {
        Self {
            headless: true,
            content_root: None,
        }
    }

    /// Resolves content assets against `root` rather than the
    /// executable-relative default — what an app with its own content
    /// directory uses.
    pub fn with_content_root(mut self, root: asset::ContentAssetRoot) -> Self {
        self.content_root = Some(root);
        self
    }
}
```

and in `build`, replace `.register_plugin(AssetManagerPlugin)` with:

```rust
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(match self.content_root.clone() {
                Some(root) => AssetManagerPlugin::with_content_root(root),
                None => AssetManagerPlugin::new(),
            })
            .register_plugin(TimePlugin);
```

- [ ] **Step 7: Switch the examples to the builder**

In each of `examples/render-test/src/main.rs`, `examples/tech-demo/src/main.rs`, `examples/animation-test/src/main.rs`: delete the `set_own_content_root` function (both `cfg` arms) and its call, delete the now-unused `AssetServer`/`ContentAssetRoot` imports it needed, and pass the root at registration instead. For `tech-demo` and `animation-test`:

```rust
    app.register_plugin(DefaultPlugins::default().with_content_root(own_content_root()));
```

with, in each example:

```rust
/// This example's own content directory, `<exe-dir>/<pkg-name>-content`,
/// populated by `build.rs`. Every example in the workspace builds into one
/// Cargo target directory, so each keeps its content tree — and therefore
/// its registry — separate. On wasm each example is served from its own
/// Trunk origin, so the platform default is already correct.
fn own_content_root() -> ContentAssetRoot {
    #[cfg(target_arch = "wasm32")]
    {
        ContentAssetRoot::default_for_platform()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        ContentAssetRoot::Directory(exe_dir.join(format!("{}-content", env!("CARGO_PKG_NAME"))))
    }
}
```

`render-test` registers `DefaultPlugins` in two `cfg` branches — apply `.with_content_root(own_content_root())` to both.

- [ ] **Step 8: Test the plugin handshake**

Create `crates/app/tests/asset_manager_plugin.rs` — the spec requires the
preload itself be covered, not just the `AssetServer` methods it feeds:

```rust
//! AssetManagerPlugin's registry preload: the `build`/`ready`/`finish`
//! handshake must leave the AssetServer able to resolve an address, and
//! must always complete — a registry that fails to load becomes an empty
//! one rather than hanging the app in its readiness loop.
use app::plugins::AssetManagerPlugin;
use app::{App, Plugin};
use asset::asset_server::AssetServer;
use asset::content::{save_content_asset, AssetRegistry};
use asset::{Asset, AssetId, ContentAssetRoot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Thing;

impl Asset for Thing {
    fn name() -> &'static str {
        "Thing"
    }
}

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("asset-plugin-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_preload_leaves_the_server_able_to_resolve_an_address() {
    let dir = temp_root("resolves");
    let address = "content/things/one.gasset";
    save_content_asset(&Thing, &dir, address).expect("save");
    let id = AssetRegistry::load(&dir).unwrap().id_for_address(address).unwrap();

    let mut app = App::new();
    let plugin = AssetManagerPlugin::with_content_root(ContentAssetRoot::Directory(dir.clone()));
    plugin.build(&mut app);
    assert!(plugin.ready(&app), "the native preload completes within build");
    plugin.finish(&mut app);

    let server = app.get_resource::<AssetServer>().expect("AssetServer inserted");
    assert_eq!(
        server.id_for_address(address),
        Some(id),
        "the preload landed and the server can resolve an address through it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_content_root_with_no_registry_still_becomes_ready() {
    let dir = temp_root("empty");

    let mut app = App::new();
    let plugin = AssetManagerPlugin::with_content_root(ContentAssetRoot::Directory(dir.clone()));
    plugin.build(&mut app);
    assert!(
        plugin.ready(&app),
        "readiness must not depend on a registry existing, or the app hangs"
    );
    plugin.finish(&mut app);

    let server = app.get_resource::<AssetServer>().expect("AssetServer inserted");
    assert_eq!(server.id_for_address("content/anything.gasset"), None);

    std::fs::remove_dir_all(&dir).ok();
}
```

`resolve_by_id` stays private to `essential`; do not widen it for the test.
`id_for_address` is public and already proves the registry landed.

`crates/app/Cargo.toml` needs a `[dev-dependencies]` section with
`serde = { version = "1", features = ["derive"] }` for the fixture type
(`essential` is already a normal dependency and is usable from tests).

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p essential --lib`, `cargo test -p app`, then `cargo test --workspace`
Expected: PASS.

- [ ] **Step 10: Verify the examples still run**

The committed content trees still hold the old addresses and the old `from_path`-derived ids — self-consistent, so every example must still work. Run each and confirm zero errors:

```bash
cargo build -p render-test -p tech-demo -p animation-test
for e in render-test tech-demo animation-test; do
  env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/debug/$e > /tmp/t7-$e.log 2>&1 &
  sleep 25; kill %1
  echo "$e ERROR lines: $(grep -c ERROR /tmp/t7-$e.log)"
done
```

Expected: `0` for each. (`render-test` needs the longer wait — give it 65s.) If a display is not reachable, say so in the report and leave visual confirmation to the controller.

- [ ] **Step 11: Run the CI gates and commit**

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add crates/essential/src/assets/asset_server.rs crates/app src/lib.rs examples/render-test/src/main.rs examples/tech-demo/src/main.rs examples/animation-test/src/main.rs
git commit -m "feat(assets): preload the registry in AssetManagerPlugin; resolve loads by address

AssetServer no longer derives a content asset's id from its address: it
looks the address up in a registry the plugin preloads through the
build/ready/finish handshake, the same one RenderPlugin uses for device
init. load() stays synchronous. Examples pass their content root at
registration instead of poking set_content_root afterwards.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 8: Re-import `render-test`

**Files:**
- Delete and regenerate: `examples/render-test/content/`
- Modify: `examples/render-test/src/main.rs`

- [ ] **Step 1: Re-import Sponza**

```bash
cd examples/render-test
rm -rf content
cargo run --release -p import -- assets/Sponza/Sponza.gltf
cd -
```

- [ ] **Step 2: Find the new scene address**

```bash
ls examples/render-test/content/Sponza/
grep -c '^[0-9a-f]\{32\} = ' examples/render-test/content/.registry.toml
```

The scene sub-asset is emitted as `scene`, so its address is `content/Sponza/scene.gasset` — unchanged. Confirm that file exists; if the tree shape differs from expectation, use the actual path.

- [ ] **Step 3: Update the load address if it changed**

`examples/render-test/src/main.rs`'s `SPONZA_PATH` stays `"content/Sponza/scene.gasset"` unless Step 2 shows otherwise. Mesh/material/texture addresses all moved under category directories, but nothing in this example names them directly — they are reached through the scene's baked references.

- [ ] **Step 4: Verify**

```bash
cargo build -p render-test
env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/debug/render-test > /tmp/t8.log 2>&1 &
sleep 65
grep -c ERROR /tmp/t8.log   # expect 0
kill %1
```

Screenshot it if a display is reachable (`/tmp/xshot/target/release/xshot "winit example" /tmp/t8.png`) and confirm the Sponza atrium renders; otherwise report that visual confirmation is deferred.

- [ ] **Step 5: Run the CI gates and commit**

```bash
cargo build --workspace && cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add examples/render-test
git commit -m "chore(render-test): re-import content with minted ids and source names

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 9: Re-import `tech-demo` and replace the magic animation indices

**Files:**
- Delete and regenerate: `examples/tech-demo/content/`
- Modify: `examples/tech-demo/src/scene.rs`, `examples/tech-demo/src/character.rs`

- [ ] **Step 1: Re-import both sources**

```bash
cd examples/tech-demo
rm -rf content
cargo run -p import -- assets/forest.glb
cargo run -p import -- assets/UAL1.glb
cd -
```

- [ ] **Step 2: Map each old index to its new name, mechanically**

This is the payoff — the twelve constants stop being indices — but the
mapping must not be guessed. `import` prints `<sub-asset name> -> <address>`
in emission order, and animations are emitted in glTF document order, so
the Nth `animation/…` line printed is exactly what used to be
`animation_<N>.gasset`. Capture that mapping directly:

```bash
cd examples/tech-demo
cargo run -p import -- assets/UAL1.glb \
  | grep -oP '^\s+animation/\K\S+' \
  | nl -v0 -ba \
  | awk '$1==53||$1==67||$1==64||$1==68||$1==69||$1==70||$1==62||$1==61||$1==63||$1==73||$1==72||$1==71 {print}'
cd -
```

That prints `<old index>\t<new name>` for exactly the twelve the example
uses. The old constants were: `53` idle, `67` jog forward, `64` jog
forward-left, `68` jog forward-right, `69` jog left, `70` jog right, `62`
jog backward, `61` jog back-left, `63` jog back-right, `73` jump start,
`72` jump loop, `71` jump land. Sanity-check that each printed name reads
like the movement it is bound to before rewriting anything — if a name
plainly contradicts its slot, stop and report it rather than wiring it up.

(If the `grep -oP` form is unavailable, read the plain `cargo run -p import`
output and count the `animation/` lines by hand — the ordering guarantee is
the point, not the exact shell.)

- [ ] **Step 3: Rewrite the constants**

In `examples/tech-demo/src/character.rs`, replace `CHAR_SCENE` and the twelve animation constants with the new addresses, e.g.:

Substitute the name Step 2 printed for each index into this block — every
line is listed so nothing is inferred, and the comment on each says which
old index it replaces:

```rust
const CHAR_SCENE: &str = "content/UAL1/scene.gasset";

const IDLE_LOOP: &str = "content/UAL1/animation/<name for 53>.gasset";
const JOG_FWD_LOOP: &str = "content/UAL1/animation/<name for 67>.gasset";
const JOG_FWD_L_LOOP: &str = "content/UAL1/animation/<name for 64>.gasset";
const JOG_FWD_R_LOOP: &str = "content/UAL1/animation/<name for 68>.gasset";
const JOG_LEFT_LOOP: &str = "content/UAL1/animation/<name for 69>.gasset";
const JOG_RIGHT_LOOP: &str = "content/UAL1/animation/<name for 70>.gasset";
const JOG_BWD_LOOP: &str = "content/UAL1/animation/<name for 62>.gasset";
const JOG_BWD_L_LOOP: &str = "content/UAL1/animation/<name for 61>.gasset";
const JOG_BWD_R_LOOP: &str = "content/UAL1/animation/<name for 63>.gasset";
const JUMP_START: &str = "content/UAL1/animation/<name for 73>.gasset";
const JUMP_LOOP: &str = "content/UAL1/animation/<name for 72>.gasset";
const JUMP_LAND: &str = "content/UAL1/animation/<name for 71>.gasset";
```

No `<…>` may survive into the committed file — if Step 2 did not produce a
name for one of the twelve, stop and report which, rather than inventing a
path or leaving the old index form in place.

Delete the `// TODO(asset-import-pipeline): magic indices — an animation-name manifest sub-asset would replace these.` comment above them: the names *are* the manifest now.

In `examples/tech-demo/src/scene.rs`, `FOREST_SCENE` stays `"content/forest/scene.gasset"` unless Step 1's output shows otherwise.

- [ ] **Step 4: Verify**

```bash
cargo build -p tech-demo
env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/debug/tech-demo > /tmp/t9.log 2>&1 &
sleep 25
grep -c ERROR /tmp/t9.log   # expect 0
kill %1
```

Screenshot and confirm the forest scene renders with the character in an idle pose — an animation address wired to the wrong clip shows up here and nowhere else, so this check is load-bearing, not decorative.

- [ ] **Step 5: Run the CI gates and commit**

```bash
cargo build --workspace && cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add examples/tech-demo
git commit -m "chore(tech-demo): re-import content; animations addressed by name

The twelve hard-coded animation indices become real clip names, which is
what the magic-indices TODO was waiting for.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 10: Re-import `animation-test`

**Files:**
- Delete and regenerate: `examples/animation-test/content/`
- Modify: `examples/animation-test/src/movement_animation.rs`

- [ ] **Step 1: Re-import all five sources**

```bash
cd examples/animation-test
rm -rf content
for s in ninja idle walk strafe_left strafe_right; do
  cargo run -p import -- assets/ninja/$s.glb
done
cd -
```

- [ ] **Step 2: Read the new addresses**

```bash
find examples/animation-test/content -name '*.gasset' | sort
```

- [ ] **Step 3: Rewrite the five constants**

In `examples/animation-test/src/movement_animation.rs`, update `NINJA_SCENE`, `IDLE_ANIM`, `WALK_ANIM`, `STRAFE_LEFT_ANIM`, `STRAFE_RIGHT_ANIM` to the addresses Step 2 lists. Each animation-only source emits one clip, so each is `content/<stem>/animation/<name>.gasset`.

- [ ] **Step 4: Verify**

```bash
cargo build -p animation-test
env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/debug/animation-test > /tmp/t10.log 2>&1 &
sleep 20
grep -c ERROR /tmp/t10.log   # expect 0
kill %1
```

Screenshot and confirm the ninja is in a posed (non-T-pose) idle stance — a T-pose means the clip address is wrong or the skeleton reference did not resolve.

- [ ] **Step 5: Run the CI gates and commit**

```bash
cargo build --workspace && cargo test --workspace
cargo fmt --all -- --check
cargo clippy -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings

git add examples/animation-test
git commit -m "chore(animation-test): re-import content with minted ids and source names

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 11: Update the asset-importing docs

**Files:**
- Modify: `docs/asset-importing.md`

- [ ] **Step 1: Rewrite the affected sections**

- **The content tree** — addresses are now `<root>/<source-stem>/<category>/<Name>.<ext>`; names come from the source with index fallback and `.1` disambiguation; every segment is sanitised to `[A-Za-z0-9._-]`.
- **The `.gasset` file format** — the `asset_id` row changes from "`AssetId::from_path(address)` — a v5 UUID derived from the address string" to "a v4 UUID minted when the asset is first written and reused on every re-import, read back from this header".
- **The asset registry** — it is derived data, rebuilt by `import` from a scan of the tree's headers, and it now answers address → id as well as id → address. Both path loads and `load_by_id` go through it.
- **How the runtime finds content assets** — replace the `set_content_root`-after-registration snippet with `DefaultPlugins::default().with_content_root(…)`, and say that `AssetManagerPlugin` preloads the registry through the plugin `ready` handshake on both platforms.
- **Loading at runtime** — a path load resolves its address through the registry; an unregistered address fails loudly rather than guessing an id.

- [ ] **Step 2: Add a "Renaming a content asset" section**

After "The asset registry":

```markdown
## Renaming or moving a content asset

Identity lives in the file's header, not in its path, so a `.gasset` can be
renamed or moved within the content tree and everything that references it
keeps working:

1. Move or rename the file.
2. Re-run `import` for any source in that project (or any command that
   rebuilds the registry) so the registry re-points the id at its new
   address.
3. Update any `load("content/…")` string literal in game code that named
   the old path — asset-to-asset references need no changes, but a hard-coded
   address in your own source does.

Renaming a mesh or animation *in the DCC tool* is a different thing: it
changes the sub-asset's name, so the next import writes a new file with a
new id and leaves the old one behind as an orphan to delete.
```

- [ ] **Step 3: Update the limitations list**

Drop the "`AssetId` is derived from the address, so renaming … breaks every reference" bullet. Add:

```markdown
- **A clean regeneration changes ids.** `rm -rf content && import` mints
  fresh ids, because identity is assigned rather than computed. The
  committed content tree's ids are real data — delete the tree only when
  you mean to.
- **A DCC rename orphans the old asset.** Renaming a mesh or animation in
  Blender changes its address, so the next import writes a new file and
  leaves the old one behind.
```

- [ ] **Step 4: Commit**

```bash
git add docs/asset-importing.md
git commit -m "docs: minted ids, named content assets, registry preload

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```
