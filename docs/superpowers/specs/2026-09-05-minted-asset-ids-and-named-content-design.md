# Minted Asset IDs and Named Content Assets — Design

**Status:** approved, ready for an implementation plan
**Branch:** `asset-store-rework`
**Supersedes parts of:** `2026-09-04-game-ready-content-assets-design.md` (the
"identity is the path" decision, and seam A's "reserved for a future
minted-id layer" note — this spec builds that layer)

## Problem

Two coupled problems with the content-asset system as it stands.

**Identity is derived from the address.** `AssetId` for a content asset is
`AssetId::from_path("content/hero/mesh_0.gasset")` — recomputed from the
address string at every call site. Renaming or moving a `.gasset` file
therefore changes its identity and silently breaks every reference to it:
the serialized `AssetHandle`s inside sibling assets still carry the old id,
and nothing repoints them. Reorganising a content tree is not currently a
safe operation.

**Content assets have machine names.** `import` names sub-assets by index —
`content/UAL1/animation_53.gasset`, `content/Sponza/mesh_17.gasset` — even
though the source usually knows better. glTF exposes `Option<&str>` names
for meshes, materials, animations, skins and images. `tech-demo` currently
hard-codes twelve `"content/UAL1/animation_<N>.gasset"` constants with a
`TODO(asset-import-pipeline): magic indices` comment because of this.

## Goals

- An asset's identity survives renaming or moving its `.gasset` file, and
  survives a re-import that does not rename it.
- Content assets are addressed by the name the artist gave them.
- `AssetServer::load()` stays synchronous.
- One preload mechanism that works identically on native and wasm.

**Note on the limit of this.** Identity survives *content-tree* renames —
the reorganisation case, and the one a future editor would drive. It does
not survive a rename made in the DCC tool: that changes the sub-asset's
name, hence its address, so the next import writes a *new* file with a
fresh id and orphans the old one. Preserving identity across a DCC rename
would need either a stable per-object id in the source format (glTF has
none) or an explicit "this replaces that" instruction from the user, and
neither is in scope.

## Non-goals

- Incremental import (still regenerates everything every run).
- Cross-source references (still warn-and-continue).
- A rename-fixup tool that rewrites source code referencing a renamed
  address. Renaming a `.gasset` keeps *asset-to-asset* references working;
  a `load("content/…")` string literal in game code still has to be updated
  by hand.
- Deleting orphans. Renaming a sub-asset in the DCC tool and re-importing
  leaves the old `.gasset` behind; cleaning that up is manual.

---

## 1. Identity: minted, stored in the header, reused on re-import

`AssetId` stays a `Uuid` newtype with its existing two constructors. What
changes is which one content assets use.

**Rule.** When `import` or `save_content_asset` is about to write a content
asset at `address`:

- if `<project_root>/<address>` already exists, read its
  `ContentAssetHeader` and **reuse** `header.asset_id`;
- otherwise **mint** `AssetId::new()` (v4 random).

The id is written to the header and to `content/.registry.toml`. It is
never again derived from the address.

**Consequences, accepted.**

- A clean regeneration (`rm -rf content && import`) produces *fresh* ids.
  The committed content tree's ids are real data, not a pure function of
  the sources — deleting the tree is a semantic change, not a cache flush.
- A DCC rename that changes a sub-asset's name changes its *address*, so
  the file at the new address is new and gets a fresh id; the old file is
  left behind as an orphan for the user to delete.

**Reading a header cheaply.** Mint-or-reuse and the registry rebuild (1.1)
both need only the header of an existing `.gasset`, and content assets get
large — the current example trees hold 503 files totalling 699 MB, with
individual textures up to 64 MB. `asset::content` therefore
gains a prefix-reading helper:

```rust
pub fn read_content_asset_header(path: &Path) -> anyhow::Result<ContentAssetHeader>
```

which reads the 8-byte magic-plus-length prefix, then exactly `header_len`
more bytes, and never touches the payload. A full-tree scan is then a few
hundred KB of I/O, not the whole tree.

### 1.1 The registry is derived from the tree

Identity living in the header is only half of rename-safety: the registry
maps `id → address`, so after a `.gasset` is moved or renamed by hand that
entry points at a path that no longer exists and `load_by_id` follows it
into a failure. A merge-upsert cannot repair this — nothing tells it the
old entry is stale — and re-running `import` would be worse, minting a
*fresh* id for a recreated file at the convention address.

So the registry stops being incrementally maintained and becomes **derived
data, rebuilt from the tree**. After `import_source` has written its
outputs, `import` walks `<project_root>/<content-root>/` for
`*.<extension>` files, reads each header with the prefix helper above, and
writes the registry as exactly `{header.asset_id → tree-relative address}`
for every file found. `AssetRegistry::insert`/`remove` remain for
in-memory use and for `save_content_asset`, but the file `import` writes is
a full rebuild, not a merge.

This makes a hand-rename safe end to end: move the file, re-run `import`,
and the scan re-points the id at its new address while every baked
reference — which names the id, not the path — keeps resolving.

Two files carrying the same `asset_id` (a copy-pasted `.gasset`) is a
malformed tree: the scan fails with an error naming both paths, rather
than silently keeping whichever it saw last.

**Import-time resolution.** Importers ask for cross-reference ids through
`ImportContext::sub_asset_id(name)` *while* importing, before all
sub-assets exist. `import_source`'s `SubAssetIdResolver` closure therefore
becomes memoised:

```
sub_name -> content_address(config, source, sub_name) -> mint-or-reuse
```

backed by a `Mutex<HashMap<String, AssetId>>` living in the closure, so a
name asked for twice in one run yields the same id, and the disk read for
an existing asset happens at most once per address.

## 2. Naming: source names, category as a directory

### 2.1 Address shape

`content_address` currently flattens the whole sub-asset name into one
filename segment (`mesh/0` → `mesh_0.gasset`). It changes to treat `/` in
the sub-asset name as a real path separator:

```
<content-root>/<source-stem>/<category-segments…>/<leaf>.<ext>
```

- Split the sub-asset name on `/`. All but the last segment become
  directories; the last becomes the filename stem.
- Sanitise **each** segment (see 2.3).
- A sub-asset name with no `/` (e.g. `scene`) yields
  `content/<stem>/scene.gasset`, unchanged from today.

Examples:

| sub-asset name          | address                                        |
|-------------------------|------------------------------------------------|
| `scene`                 | `content/UAL1/scene.gasset`                    |
| `animation/Idle Loop`   | `content/UAL1/animation/Idle_Loop.gasset`      |
| `mesh/Body.primitive0`  | `content/UAL1/mesh/Body.primitive0.gasset`     |
| `texture/Albedo_linear` | `content/Sponza/texture/Albedo_linear.gasset`  |

### 2.2 Where names come from

`GltfImporter` replaces index-based sub-asset names with source names:

| sub-asset   | named form                            | fallback when unnamed |
|-------------|---------------------------------------|-----------------------|
| mesh        | `mesh/<mesh.name()>`                  | `mesh/<counter>`      |
| mesh, multi-primitive | `mesh/<name>.primitive<k>`  | `mesh/<counter>.primitive<k>` |
| material    | `material/<material.name()>`          | `material/<index>`    |
| animation   | `animation/<animation.name()>`        | `animation/<index>`   |
| skeleton    | `skeleton/<skin.name()>`              | `skeleton/<index>`    |
| texture     | `texture/<image.name()>` plus the existing `_linear` suffix for the linear variant | `texture/<image index>` |
| scene       | `scene` (synthetic, unchanged)        | —                     |
| default material | `material/default` (synthetic)   | —                     |

`ObjImporter` keeps `material/<mtl stem>` and `scene`, and uses the object
name from the `.obj` for meshes where present, falling back to
`mesh/<index>`. `ImageImporter` emits `texture/main` (was `main`) — a
single-asset source, where the address already carries the file stem.

Multi-primitive meshes keep the `.primitive<k>` convention already used for
the generated child node names, so a mesh's primitives sort together.

### 2.3 Sanitising and disambiguation

**Sanitise** each address segment: every character outside
`[A-Za-z0-9._-]` becomes `_`; runs of `_` collapse to one; leading and
trailing `_` and `.` are trimmed. A segment that is empty after sanitising
falls back to the index form. This turns Mixamo's `mixamorig:Hips` into
`mixamorig_Hips` and `Idle Loop` into `Idle_Loop`.

**Disambiguate** within a category, per source file: the importer keeps a
`HashMap<String, u32>` of names already emitted under each category and
appends `.1`, `.2`, … to the second and later occurrences. Two meshes both
named `Body` become `mesh/Body` and `mesh/Body.1`. Disambiguation happens
*after* sanitising, so two names that sanitise to the same string also
disambiguate.

Sanitising and disambiguation both live in the importer, so the sub-asset
name an importer emits is already the final one and cross-references keyed
on it are consistent. `content_address` still sanitises defensively.

## 3. Runtime: preloaded registry, one mechanism for both platforms

### 3.1 `AssetRegistry` gains an address index

The on-disk TOML is unchanged (`[assets]`, `<simple-hex> = "<address>"`).
In memory, `AssetRegistry` additionally builds `address → AssetId` when
parsing, and exposes:

```rust
pub fn id_for_address(&self, address: &str) -> Option<AssetId>
```

Two entries mapping different ids to the same address is a malformed
registry and fails `parse` with an error naming the address.

### 3.2 The preload moves into `AssetManagerPlugin`

`AssetManagerPlugin` becomes configurable and takes over the async preload,
using the `build → ready → finish` lifecycle exactly as `RenderPlugin`
already does for device init:

```rust
pub struct AssetManagerPlugin {
    content_root: Option<ContentAssetRoot>,
}
impl AssetManagerPlugin {
    pub fn new() -> Self;
    pub fn with_content_root(root: ContentAssetRoot) -> Self;
}
```

- **`build`** — inserts the `AssetServer` (with `content_root` applied if
  given), then kicks off `utils::load_registry(&root)` into a shared
  `Arc<Mutex<Option<AssetRegistry>>>`: `pollster::block_on` on native,
  `wasm_bindgen_futures::spawn_local` on wasm. Same shape as
  `RenderPlugin::build`.
- **`ready`** — `true` once that slot is filled.
- **`finish`** — moves the registry into the `AssetServer`'s cache.

`DefaultPlugins` gains a matching builder that forwards the root:

```rust
app.register_plugin(DefaultPlugins::default().with_content_root(root));
```

This replaces the `set_own_content_root(&mut app)` helper each example
calls today after registering `DefaultPlugins` — one call site instead of
two, and it works identically on wasm, where the current helper is
`cfg`'d out entirely.

`AssetServer::set_content_root` stays for runtime overrides. It already
clears the cached registry; after this change, clearing it means path loads
hard-fail until a registry is loaded for the new root, which is the honest
behaviour for an override that changes where everything lives.

### 3.3 `load_internal` resolves through the index

```rust
let address = path.address();
let id = match self.registry_id_for_address(&address) {
    Some(id) => id,
    None => { /* log, dead handle, return */ }
};
```

`load_internal` stays synchronous — the lookup is an in-memory hash probe.
Its existing `path_to_id` dedup cache and the `request_load` call are
unchanged below this point.

**Miss behaviour**, both logged at error level and returning a handle that
never resolves:

- registry loaded, address absent →
  `no asset registered at '<address>' — run import`
- registry never loaded →
  `asset registry not loaded; register AssetManagerPlugin with a content root before loading '<address>'`

`load_by_id` is unaffected — it already resolves id → address through the
registry, and its miss path already behaves this way.

`AssetServer::add` (runtime-only assets) is unaffected: it mints
`AssetId::new()` and has no address.

## 4. Fallout

- **All three examples are re-imported.** Every `.gasset` changes name and
  id; `content/.registry.toml` regenerates. Their address constants are
  rewritten to the named form — including `tech-demo`'s twelve
  `animation_<N>` magic indices, which become real clip names and let the
  `TODO(asset-import-pipeline): magic indices` comment go.
- **Examples switch to the plugin builder** and drop their
  `set_own_content_root` helper.
- **Importer tests** that assert `AssetId::from_path("triangle.gltf#mesh/0")`
  or `header.asset_id == AssetId::from_path(address)` no longer hold —
  neither identity survives. They are rewritten to assert the *properties*
  that still hold: that a re-import reuses the id already on disk, that
  cross-references resolve to the ids of the sub-assets actually emitted,
  and that the registry maps each written address to the id in its header.
- **`docs/asset-importing.md`** is updated: identity section (minted,
  reused, registry derived by scan), naming section, the preload/builder
  call, a short "renaming a content asset" workflow (move the file, re-run
  `import`), and the limitations list — the "renaming changes identity"
  entry goes away, replaced by "a clean regen changes ids" and "a DCC
  rename orphans the old file".

## 5. Testing

- `content_address` — category segments become directories; each segment
  sanitised; no-slash names unchanged.
- Name sanitising and disambiguation — `mixamorig:Hips` → `mixamorig_Hips`;
  two `Body` meshes → `Body`, `Body.1`; unnamed → index fallback; a name
  that sanitises to empty → index fallback.
- Minting — a fresh address mints a new id; re-importing over an existing
  tree reuses the ids already in the headers; two sub-assets in one run get
  distinct ids.
- `read_content_asset_header` returns the same header
  `read_content_asset` does, without reading the payload (assert on a file
  whose payload is larger than the prefix read).
- Registry rebuild — a scan of a tree produces one entry per `.gasset`
  keyed by the id in its header; a file moved between two scans is
  re-pointed at its new address under the same id; two files sharing an
  `asset_id` fail the scan with both paths named.
- `AssetRegistry::id_for_address` round-trips; a duplicate address in the
  TOML fails `parse`.
- `AssetServer::load` resolves an address through a preloaded registry to
  the minted id; an unregistered address and a never-preloaded registry
  each fail with their own message and do not panic.
- `AssetManagerPlugin` preload — `ready()` is false until the registry
  lands and true after; `finish()` leaves the server able to resolve an
  address. Native path exercised in tests; the wasm path is verified by
  running the examples.
- End-to-end: import a fixture, load a sub-asset by address, and load a
  second one by the id baked into the first's payload.
