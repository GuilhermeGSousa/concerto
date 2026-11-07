# Asset importing

The engine does not load DCC files (`.gltf`, `.glb`, `.obj`, `.bvh`, `.png`, …)
directly at runtime. They are **imported** once, ahead of time, into a tree
of self-describing binary files that the game references by UUID. This is
the Unreal/Godot model: a source artist file is an input to a build step,
not something the shipped game reads.

```
assets/hero.glb   ──(import)──▶   content/hero/mesh_0.gasset
                                  content/hero/material_0.gasset
                                  content/hero/skeleton_0.gasset
                                  content/hero/scene.gasset
assets/hero.glb.import.toml       (persistent source/output identities)
```

- **Source files** live wherever you like (`examples/*/assets/` by
  convention). They are never read by the running game.
- **Content assets** (`*.gasset`) are the game-ready form: one file per
  sub-asset, each carrying a small header plus a `bincode` payload. They
  are committed to version control alongside the code.
- **Source metadata** (`<source>.import.toml`) is committed beside each
  imported source. It records which UUID belongs to each sub-asset.
- **Addresses** are plain paths (`content/hero/mesh_0.gasset`). Code uses
  `asset_id!("content/hero/mesh_0.gasset")` to resolve a UUID during compilation;
  the runtime uses UUIDs to locate files through the registry.
- **The registry** maps each UUID to its address and kind. It is never
  committed and never a file at runtime: each game's `build.rs` builds it from
  the content tree and the executable embeds it, and the editor builds it by
  scanning when a project opens.

## Importing a source file

Sources are imported in the editor, with **Import…** beside the Asset Browser
search; [crates/editor/README.md](../crates/editor/README.md#importing)
describes the dialog. There is no import command line. The project the editor
has open is the *project root*, and the import writes `content/…` under it. A
source from outside the project is first copied to the destination the dialog
shows, with every file it refers to. To re-import an example's sources, run its
editor over a scratch copy of the project, never over `examples/*` itself:

```sh
cargo run -p tech-demo-editor -- --project <scratch copy of examples/tech-demo>
```

The editor's import engine is the `concerto-import` library
(`import_source_into`). It returns an `AssetEntry`, provenance included, for
each content asset it wrote, and the editor adds those entries to its registry.

The import picks the importer by file extension:

| Extensions           | Importer        | Emits                                              |
|----------------------|-----------------|---------------------------------------------------|
| `.gltf` `.glb`       | `GltfImporter`  | `mesh/*`, `material/*`, `texture/*`, `skeleton/*`, `animation/<clip name>`, `scene` |
| `.obj` (+ its `.mtl`)| `ObjImporter`   | `mesh/*`, one `material/<mtl stem>`, `scene`       |
| `.bvh`               | `BvhImporter`   | `skeleton/0`, `animation/<file stem>`, `scene`     |
| `.png` `.jpg` `.jpeg`| `ImageImporter` | one `main` texture                                |

### `content.toml`

Optional, at the project root. The import, the editor's scan, and
`track_assets` read it. The runtime learns the extension from the address and
the root from `ContentAssetRoot` (below).

```toml
extension = "gasset"   # default
root = "content"       # default, and the only accepted value
```

`root` must be `content`. Custom roots are not supported yet, because the
runtime's content discovery and every `build.rs` assume `content`; they are
tracked in issue #140. A `content.toml` that cannot be parsed, or that sets any
other `root`, fails the import, the editor's project open, and the build
instead of being ignored.

`default_scene`, unset by default, names the scene the editor opens when it
opens the project, by its content address:

```toml
default_scene = "content/forest/scene.gasset"
```

When it is unset, or names an address that is missing or is not a Scene, the
editor opens the first scene in the content instead, and warns in the second
case. The game ignores it; the scene a game starts in is the one it declares
with `App::set_startup_scene`.

## The content tree

For a new sub-asset, the import chooses an initial address:

```
<root>/<source-relative-directories>/<source-file-stem>/<sanitised-sub-asset-name>.<ext>
```

- Source directories relative to the project are preserved, with a leading
  `assets/` removed: `assets/enemies/hero.glb` → `content/enemies/hero/…`.
- `<source-file-stem>` is the source file name without its extension.
- The sub-asset name (`mesh/0`, `animation/Idle_Loop`, `material/steel`) has
  `/`, `\`, and `#` replaced with `_`, so `animation/Idle_Loop` →
  `animation_Idle_Loop.gasset`.
- A file name over 200 bytes is cut on a character boundary and ends in `_` plus
  8 hex digits of a hash of the full sub-asset name, so long names stay distinct.

### Animation clip names

A clip's sub-asset name is `animation/<name>`, where `<name>` comes from the
source:

- glTF uses the animation's name. BVH has no clip name, so it uses the source
  file stem: `walk.bvh` → `animation/walk` → `animation_walk.gasset`.
- `/`, `\`, `#`, `:`, `*`, `?`, `"`, `<`, `>`, `|` and control characters are
  replaced with `_`, and whitespace and dots are trimmed from both ends.
- A missing or empty name (after sanitising) falls back to the clip's index:
  `animation/7`.
- Names are unique case-insensitively. A clip whose name an earlier clip
  already took gets `_{index}` appended (`animation/Walk_4`), and again if that
  is taken too.

Existing sub-assets are located by UUID, so an asset moved within the content
tree is updated at its new location on re-import. Older imported assets keep
their existing paths even when those paths differ from the new convention.
An occupied destination owned by another asset is an error.

Cross-references *within a source* use the same UUIDs recorded in the source
metadata and output headers. A scene's `MeshComponent` holds the mesh's UUID,
which remains unchanged when its `.gasset` file moves.

The content tree is **committed to git**. It is regenerated by re-importing in
the editor, not by a hidden build step, so a stale tree is a visible diff. It
holds no derived data: what is computed from other content assets is built on
load and kept outside the tree (see [Derived data](#derived-data)).
`.gasset` files are stored in Git LFS, set by `.gitattributes`. A game's build
reads every header, so it fails with `missing the GRDY magic prefix` on a
checkout that holds LFS pointers; `git lfs pull` fetches the real files.

## Persistent source metadata

An import creates a sidecar by appending `.import.toml` to the full source
filename, for example `assets/hero.glb.import.toml`:

```toml
version = 1
source_id = "753c9647-1ad6-40e5-8e72-905cb0ad4f17"
importer = "gltf"

[outputs."mesh/0"]
asset_id = "d6e76e64-a888-49cd-bc16-61f0566f36ab"

[outputs.scene]
asset_id = "abfdaec1-d105-4c4c-b37c-138a1231e4ac"
```

An optional `[settings]` table holds importer-specific settings. The importer
that owns the source reads it on every import; a sidecar without one uses the
importer's defaults, and a first import writes none. The `obj` and `image`
importers read no settings. The `gltf` importer reads only `events` and
`additive`, described under [Clip events](#clip-events) and
[Additive clips](#additive-clips), and an unknown key fails its import. The
`bvh` importer's are described under [BVH motion capture](#bvh-motion-capture).

A `bvh` source's settings:

```toml
[settings]
scale = 0.01
frame_range = [10, 250]
```

A `gltf` source whose `animation/Run` clip gets two footsteps:

```toml
[settings.events.Run]
footstep = [0.2, 0.55]
```

Settings are edited by hand. To change them: import the source once, edit the
`[settings]` table in its sidecar, then import the source again from the editor.
The re-import keeps every asset ID and replaces the content with the new
result. An unknown key or an invalid value fails that import with an error
naming the source; nothing is written.

The sidecar records ownership, the content header carries file identity, and
the registry records location. Commit the sidecar with its source and move
them together when renaming or reorganizing source files. A separate new
source needs its own identity; copying a sidecar does not create a new asset.

When importing older content without a sidecar, the importer uses matching
header provenance to recover existing output UUIDs. It rejects ambiguous
ownership instead of assigning existing assets to an unrelated source.
Existing example sidecars preserve all previously committed UUIDs and paths.

The importer validates content headers, metadata, and planned destinations
before writing outputs. An unreadable existing header is an error; it never
silently causes a replacement UUID. Individual files are replaced using
temporary files. The entire import is not a crash-safe multi-file transaction.

## BVH motion capture

`BvhImporter` reads a `.bvh` file's `HIERARCHY` and `MOTION` sections and emits:

- `skeleton/0`: one bone per joint, parents first, with each joint's `OFFSET` as
  its rest translation. Bone ids are derived from the joint's path from the
  root, so they stay the same across re-imports and across clips of the same
  rig.
- `animation/<file stem>`: one translation channel for each joint with position channels
  and one rotation channel for each joint with rotation channels, one key per
  frame at the file's `Frame Time`. Euler angles are in degrees, applied in the
  order the joint's `CHANNELS` line lists them. A quaternion that would flip
  sign against the previous frame is negated, so playback takes the short way
  round.
- `scene`: a node per joint with its rest transform, and a `SkeletonComponent`
  on the root pointing at `skeleton/0`.

Settings, in the sidecar's `[settings]` table:

| Key           | Default | Meaning |
|---------------|---------|---------|
| `scale`       | `0.01`  | Multiplier applied to every offset and translation. The default turns the centimetres most BVH files use into metres. Must be finite and positive. |
| `frame_range` | whole file | `[start, end)` frame indices to keep. Needs `start < end <= frame count` and at least two frames. Keys are timed from the first kept frame. |
| `events`      | none    | Named events on the clip, as described under [Clip events](#clip-events). The table is `[settings.events.<name>]`, where `<name>` is the clip's sanitised name, the `<name>` of its `animation/<name>` output: `Walk#Cycle.bvh` gives `Walk_Cycle`. Times count from the first kept frame. |
| `additive`    | none    | Makes the clip additive against a base clip, as described under [Additive clips](#additive-clips). The table is `[settings.additive.<name>]`, with `<name>` as for `events`. `range` counts from the first kept frame. |

A `frame_range` outside the file, an empty or one-frame range, a bad `scale`,
bad `events` or `additive`, or an unknown key fails the import. The file's own timing is
kept; there is no resampling setting.

## Clip events

The `gltf` and `bvh` importers put named events on the clips they emit, read
from `[settings.events.<clip>]` tables:

```toml
[settings.events.walk]
footstep_left = [0.12, 0.62]
footstep_right = [0.37, 0.87]
```

- `<clip>` is the `<name>` of the clip's `animation/<name>` output, after the
  importer has sanitised it and made it unique, not the raw glTF animation
  name or file stem.
- Each key is an event name, and its list holds clip times in seconds. Each
  time becomes an `AnimationEvent { name }` at that time; a time listed twice
  under one name adds one event.
- A table for a clip the source does not emit, or a time that is negative,
  not finite, or past the clip's duration, fails the import.

A clip's `AnimationEvent`s can also be added, moved, renamed and removed in the
editor's clip tab (see `crates/animation-editor/README.md`), and Save writes
them into the clip's `.gasset` with `resave_content_asset`. Other event types
show there read-only and are saved as they are. The save keeps the clip's
import provenance, so the clip stays owned by its source: the next re-import
of the source rebuilds the clip and its events from `[settings.events]`, and
the edits saved from the editor are lost. To keep an event across re-imports,
put it in `[settings.events]`. An authored clip, with no provenance, keeps its
saved events.

## Additive clips

The `gltf` and `bvh` importers make a clip additive against a base clip, its
matched neutral take, from a `[settings.additive.<clip>]` table:

```toml
[settings.additive.Cold_Walk]
base = "6f1c2a3e-9d1b-4c84-a1a7-0b6f3a1d2e44"
base_range = [0.0, 4.2]
range = { start = 0.1, end = 4.3 }
```

- `<clip>` is the `<name>` of the clip's `animation/<name>` output, as for
  [Clip events](#clip-events).
- `base` is the base clip's asset id, as written in the
  `[outputs."animation/<name>"]` table of the base source's sidecar. A value
  that isn't an asset id fails the import.
- `base_range` and `range` are optional spans in seconds of the base clip and of
  this clip, each written as a two-element array `[start, end]` or as a
  `{ start, end }` table. The whole clip is used when one is unset. An extra key
  in a `{ start, end }` table fails the import.
- `range` is checked against this clip: a `range` that is not finite, whose
  `start` is negative or not below its `end`, or whose `end` is past the clip's
  duration fails the import. `base_range` is checked when the clip loads, once
  the base is read.
- The clip's root bones, whose translation the delta leaves out, come from the
  skeletons the source emits: every bone with no parent in one of them. The
  sidecar doesn't set them.
- A table for a clip the source does not emit, or an unknown key, fails the
  import.

The import doesn't read the base or check that it exists, so the clip and its
base import in any order. The delta between the two is
[derived data](#derived-data), built when the clip loads: a missing base, a
`base_range` that doesn't fit the base, or any other failed derive fails the
clip's load, with an error naming the clip and the reason. A clip with no table
is plain, `Additive::None`; removing the table and importing again makes the
clip plain again. A clip added in code with `AssetServer::add` derives nothing,
and has no delta.

## The `.gasset` file format

```
magic "GRDY" (4 bytes)
u32   header length, little-endian
      bincode(u32 format version, ContentAssetHeader)
      payload — the asset's own bincode encoding, verbatim
      derived blob — `derived.len` bytes, only when the header has a `derived` section
```

A skeleton asset (`skeleton/N`) holds bone names, parent indices (each parent before its child), inverse bind poses, the rest pose as `JointPose` and the bone ids. `JointPose` lives in `concerto-mesh`; `concerto-animation` re-exports it. A skin whose joints are not ordered parents-first fails the import with an error naming the skin and joint; a skin with a joint outside every scene root is skipped with a warning. Neither emits a skeleton.

The format version is `CONTENT_FORMAT_VERSION`, currently `6`; it changes only when the framing or the header changes, never for a payload. Version 5 adds `kind_version` to the header, and version 6 adds `derived`, the description of a derived blob that trails the payload in a cooked file. The payload is everything between the header and that blob; `read_content_asset` returns it without the blob, and `read_content_asset_parts` returns both. `write_content_asset` writes no section and refuses a header that has one; `write_cooked_content_asset` writes the header, the payload and the blob of a `CookedSection`. Committed content never has a section. A version-4 file reads as kind version 1 and a version-5 file reads with no section; neither needs a re-import, and each moves to version 6 the next time the editor saves it or an import rewrites it. Any other version is rejected. glTF clips have no stored frame rate, so the clip editor shows a detected one; BVH clips store the rate from `Frame Time`.

`ContentAssetHeader` (see `concerto_foundation::assets::content`):

| Field            | Meaning                                                        |
|------------------|---------------------------------------------------------------|
| `asset_id`       | Persistent UUID, minted once and reused from source metadata; legacy UUIDs are preserved |
| `references`     | the `AssetId`s this asset points at, so a tree can be integrity-checked by scanning headers alone |
| `kind`           | the asset type tag (`Mesh`, `StandardMaterial`, `Scene`, …); must match the loading type |
| `kind_version`   | the `Asset::FORMAT_VERSION` of `kind` that the payload was written with; `1` for a version-4 file |
| `provenance`     | `Some(ImportProvenance { source, sub_asset })` for import output, kept when `resave_content_asset` writes over it; `None` for editor-authored assets and for an imported asset saved with `save_content_asset_into` |
| `derived`        | `Some(DerivedSection { derived_version, key, len })` for a cooked file: the type's `DERIVED_VERSION` and `derived_key` at cook time and the blob's length in bytes; `None` otherwise, and for a file older than version 6 |

The header can be read without touching the payload, which is what lets a
future tool index a whole content tree cheaply.

## Changing an asset type's encoding

Each asset type has its own payload version, `Asset::FORMAT_VERSION`, which
starts at 1 and is written to `kind_version` in every header. Version 1 of a
type is its encoding as committed at header version 4. Header scans
(`scan_content`, `asset_id!`, the build script, project open and import) never
check it; a file is checked only when it is decoded. Every decode goes through
`decode_content_asset::<A>`, or `utils::load_content_asset::<A>` for a file
under a `ContentAssetRoot`, which the `AssetServer` uses:

| `kind_version`            | Result                                                      |
|---------------------------|-------------------------------------------------------------|
| equal to `FORMAT_VERSION` | the payload is decoded with bincode                         |
| older                     | `A::migrate(version, payload)`; the default rejects it      |
| newer                     | rejected: the file was written by a newer build             |

A `kind` other than `A::name()` is rejected too, so a type that changes its
`name()` cannot read files written under the old one: they are re-imported, or
re-created if they were authored in the editor. bincode is not
self-describing, so a payload cannot absorb a new field with
`#[serde(default)]`: any change to a type's serialized fields, their order or
their types is a new encoding.

Each content type's encoding is pinned by a test. `check_encoding(&value, dir)`
(`concerto_foundation::assets::content`) serializes the value and compares the
bytes with `<dir>/<A::name()>.v<A::FORMAT_VERSION>.bin`. The checks sit in each
crate's serialization test, with the fixtures in its `tests/encoding/`:

| Test file | Types |
|---|---|
| `crates/mesh/tests/mesh_serialization.rs` | `Mesh`, `Skeleton` |
| `crates/render/tests/material_serialization.rs` | `Texture`, `StandardMaterial` |
| `crates/animation/tests/graph_serialization.rs` | `AnimationClip`, `AnimationGraph`, `BoneMask` |
| `crates/scene/tests/scene_serialization.rs` | `Scene` |
| `crates/motion-matching/tests/serialization.rs` | `MotionDatabase` |

A missing fixture fails the test with a hint to bless it. Bytes that differ
fail it with a message to raise the version. The value given to the check sets
every field to something other than its default, and uses no `HashMap` with
more than one entry, since bincode writes a map in iteration order. A type that
becomes content, because an importer emits it or the editor creates it, adds
its check in the same change.

`AnimationGraph` stores each node's Rust type name, so moving or renaming a
node type also fails its check. That is a change to the persisted form, but not
to the field layout: old graphs need the old name to resolve, not a version
bump.

Adding a reflected field to a node type is not a version bump either. A node is
stored as its reflected fields by name, so the graph's bincode layout does not
change, and a stored node without the new field loads with the field's value
from the type's `Default` and no warning. The bytes of a newly encoded node do
change, so the fixture is re-blessed at the same version, with the field set
to something other than its default in the checked graph.

A reflected node field that changes type is not a version bump either, for the
same reason. A stored node whose field holds a value of the old kind, such as a
`String` leaf where the field is now an `Option<Name>`, loads with that field
at its value from the type's `Default` and a warning that the stored data is a
different kind of value; the graph still loads. The fixture is re-blessed at
the same version.

When you change one:

1. Raise the version. A derived type writes `#[asset(version = N)]` next to
   `#[derive(Asset)]`; a hand-written `impl Asset` sets
   `const FORMAT_VERSION: u32 = N;`.
2. Choose between migration and rejection. To keep reading older files,
   implement `Asset` by hand and override `migrate`. Put frozen
   `<Type>V<n>` structs, which only derive `Deserialize`, and their
   conversions in a `legacy` module next to the type, in its own crate.
   `migrate` matches on the version, decodes the frozen struct with bincode and
   converts it; an older version converts through each newer one in turn.
   Without a migration, older files fail to load with an error that says to
   re-import their source or re-create them in the editor.
3. Bless the new fixture with
   `CONCERTO_BLESS_ENCODING=1 cargo test -p <crate> --test <file>`, run the
   test again without the variable, and commit the new `.bin` file. Keep the
   old version's fixture while its `migrate` arm exists, with a test that
   passes it through `decode_content_asset` and checks the converted value;
   delete it when you choose rejection.
4. Migration happens in memory: the file keeps its old version until it is
   saved, and a save writes the current `FORMAT_VERSION`. Re-import the
   affected sources, or re-save editor-authored assets, and commit the
   rewritten content.
5. A legacy struct and its `migrate` arm may be deleted once no committed
   `.gasset` under `examples/*` carries that version. Its fixture and test go
   with it, and the PR says so. After that, files at that version are
   rejected.

`AnimationClip` is at version 3. Version 2 adds the clip's events, and
version 3 adds `additive`, an `Additive` stored as the payload's first field:
`None` for a plain clip, or `LocalSpace` with the base clip's asset id, the
optional `base_range` and `range`, and the root bones. Its `migrate` reads a
version-2 payload through `legacy::AnimationClipV2`
(`crates/animation/src/clip.rs`) as a clip that isn't additive, and a version-1
payload through `legacy::AnimationClipV1`, which becomes an `AnimationClipV2`
with no events and then the current clip. `AnimationClip.v1.bin` and
`AnimationClip.v2.bin` stay, each with a test that decodes it. The clips committed
under `examples/tech-demo/content/UAL1/` and
`examples/tech-demo/content/lafan1/animations/` are still version 1: they
migrate in memory on every load, and the editor reports them as outdated until
they are re-imported. Like a graph's nodes, each event is stored as its Rust
type name and reflected fields. A clip makes its events live when it loads; an
event whose type is not registered with `register_clip_event`, or whose stored
value leaves a warning, is dropped with a warning naming the clip and the type.

The editor reports outdated files when the first scene of a project opens.
It compares each entry's `kind_version` with the `FORMAT_VERSION` that the
scene's `AssetServer` records for that kind. That server is built from the
game's recipe, so it knows every type the game registers. For each kind and
file version that differs, it logs a warning with the count, and the status
line reads `N content assets are outdated; see the log`. It reports once per
opened project. A kind the game's recipe does not register is not reported;
such a file still fails when it is loaded.

`CONTENT_FORMAT_VERSION` is a different number. It changes only when the
framing or `ContentAssetHeader` changes, never for a payload, and a new header
version keeps the previous ones readable where it can, as version 6 reads
versions 4 and 5. The version-6 framing is pinned by
`crates/foundation/tests/encoding/content_asset.v6.bin`, rewritten under
`CONCERTO_BLESS_ENCODING=1`.

## The asset registry

The registry lists every content asset's `AssetId` with its address and kind.
No registry file is committed or read at runtime. A game's build writes it as
TOML to `$OUT_DIR/concerto_registry.toml` and embeds it (see
[Loading at runtime](#loading-at-runtime)):

```toml
[assets]
9f3c…e21a = { address = "content/UAL1/scene.gasset", kind = "Scene" }
b75e…8457 = { address = "content/UAL1/mesh_0.gasset", kind = "Mesh" }
```

`AssetRegistry` (`concerto_foundation::assets::registry`) is the registry in
memory, and a resource in every `App` that has `AssetManagerPlugin`. Each
`AssetEntry` has the id, the address, the kind, and, when the registry was built
by scanning headers, the import provenance. List assets by type without loading
them, as with Unreal's `GetAssetsByClass`:

```rust
fn list_meshes(registry: Res<AssetRegistry>) {
    for entry in registry.of_type::<Mesh>() {
        println!("{} in {} ({:?})", entry.name(), entry.folder(), entry.id);
    }
}
```

Entries come in folder, then name order. `get(id)` and `id_for_address(address)`
look up one entry. React to catalogue changes with `registry.is_changed()`.

A scene's serialized `MeshComponent` carries a bare `AssetId` (no path); when
the spawner upgrades that `Weak` handle to `Strong`, `AssetServer` resolves the
id to an address through the registry. Code-authored references already contain
the UUID emitted by `asset_id!`. `AssetServer` holds a snapshot of the resource
that its load tasks resolve against, replaced whenever the resource changes. A
`load::<T>` whose id the registry lists under another kind fails before reading
the file.

The editor scans the content tree once, when a project opens. After that, an
import inserts the entries it returns, a New in the Asset Browser inserts the
created asset's entry, and a scene save upserts the scene's entry, all in
memory, with no rescan and no file written beyond the asset itself. Content that git or
a file explorer changes while the editor is open shows after the project is
reopened. An import still scans the tree itself before writing, to find
outputs that moved. Every scan, the build's included, fails on two files with
one id, naming both, and on an unreadable header, naming its path.

## How the runtime finds content assets

`ContentAssetRoot` is the directory (native) or URL base (wasm) that
addresses are resolved against:

- **native default:** `<exe-dir>/<exe-name>-content/` when that directory
  contains a `content/` directory, otherwise the directory containing the
  executable (`content_root_for_exe`). Address `content/hero/scene.gasset`
  therefore resolves within the executable-specific directory in Cargo
  workspaces and directly beside the executable in packaged layouts.
- **wasm default:** the page origin, fetched over HTTP.

Every workspace example ships its content tree with a `build.rs` that copies
`content/` into `<exe-dir>/<pkg-name>-content/`. The executable name matches
the package name, so native root discovery selects it automatically and the
content trees do not collide:

```rust
// build.rs — copy to <exe-dir>/<pkg-name>-content/content/
let output_path = out_dir.ancestors().nth(3).unwrap()
    .join(format!("{}-content", env::var("CARGO_PKG_NAME")?));
```

On wasm each example is served from its own Trunk origin, so there is
nothing to disambiguate and the default is correct. `index.html` ships the
files the game loads with Trunk `copy-dir` and `copy-file` links, each with a
`data-target-path` that keeps its address under `content/`. A file the game
loads that no link copies is a 404 on the web. `tech-demo` copies only what its
scene, character and motion database use, not the whole tree.

## Loading at runtime

Use `asset_id!` for references written in game code:

```rust
use concerto::{asset_id, foundation::assets::AssetId};

const HERO: AssetId = asset_id!("content/hero/scene.gasset");

let hero: AssetHandle<Scene> = asset_server.load::<Scene>(HERO);
```

The macro resolves paths against the **calling package's `CARGO_MANIFEST_DIR`**,
reads the `.gasset` header, and emits the UUID's 16 bytes as a constant. It does
not embed the file payload, import source assets, read the registry, or perform
a runtime path lookup. A missing file expands to `AssetId::nil()` with a
deprecation warning at the macro invocation, which a crate that denies warnings
sees as an error. Malformed headers and unsupported file versions are compile
errors there. It is also available as
`concerto_foundation::asset_id!` and `concerto_foundation::assets::asset_id!`.

Register the package's content directory for stable Rust build tracking, and
embed the registry the build writes:

```toml
# Cargo.toml (adjust the path for your package)
[build-dependencies]
concerto-asset-build = { path = "../../crates/asset-build" }
```

```rust
// build.rs — combine with your existing content-copying build script
fn main() -> std::io::Result<()> {
    concerto_asset_build::track_assets("content")
}
```

```rust
// src/main.rs
fn main() {
    concerto::run(my_game::game, concerto::content_registry!());
}
```

This helper tells Cargo to watch the directory recursively and configures the
macro's allowed input root. Changing, adding, or removing files causes Cargo to
recompile the package and re-read UUIDs, even when no Rust source changed.
Register one common directory containing all referenced content. The macro
rejects unregistered inputs so an omitted build dependency cannot silently
leave stale UUIDs in the executable. Existing examples include this setup.

`track_assets` also scans the `content` tree and reads `content.toml`, failing
the build when the file is invalid or sets a root other than `content`, and writes
the registry to `$OUT_DIR/concerto_registry.toml`. Two files with one id, or an
unreadable header, print the paths as a Cargo warning and fail the build.
It also records the package directory as `CONCERTO_PROJECT_ROOT`.
`concerto::content_registry!()` embeds the file and that directory; it needs a
`build.rs` that calls `track_assets`. `concerto::run` installs both, and a
native game keeps derived data in `<project>/derived-cache/`, as the editor
does; the web build has no cache. Cargo watches that tree and `content.toml` as well as the
registered directory, so a `.gasset` moved or added by git or a file explorer,
or a change to `content.toml`, is in the registry at the next build. Cargo
re-runs a build script on every build while a path it watches is missing, so a
`content.toml` or root that does not exist is not watched: one created later is
read the next time the build script runs for another reason, such as a change
to the registered directory or to `build.rs`.

Import assets before compiling the game, and ship the content tree alongside
it; the registry is inside the executable. Compilation uses files on the build
machine even for wasm targets; runtime loading still uses the configured
directory or HTTP base. Moving a file requires updating the macro's path for the
next compilation, and an existing binary finds the moved file only once it is
rebuilt.

`AssetServer` starts with an empty registry and resolves loads against the one
published to it. `concerto::run` and `concerto_terminal_renderer::run` take the
embedded registry text and install it after the recipe runs and before plugin
readiness: it becomes the `AssetRegistry` resource and is published to the
`AssetServer`, so startup systems can load by id. A malformed embedded registry
panics; only a corrupt build produces one. `concerto::headless` and tests build
an `AssetRegistry` in code, for example with `AssetRegistry::from_content_tree`,
and insert it as a resource or publish it with `AssetServer::publish_registry`.
Constructing a macro-generated UUID never requires runtime initialization.

```rust
// Deserialized component handles and macro-generated IDs use the same API.
let mesh: AssetHandle<Mesh> = asset_server.load::<Mesh>(id);
```

Loads return immediately with a handle; the bytes load on a background task
pool. A registry miss on `load` fails the load and logs the id — it
does not fall back to anything.

`AssetServer::is_loading` reports whether an asset requested with `load` or
`add` has yet to reach the world. A load counts until the frame that inserts
its asset, or handles its failure; cancelled loads do not count.

Stored assets can only be requested by UUID. Use `asset_id!` for references
written in code and `load` for both those IDs and serialized references.
There is no runtime path-loading API or path-to-UUID lookup. Paths are resolved
internally from the registry when reading asset bytes.

Configure the root before content loading starts; changing projects requires a
separate server. Procedural assets can still be inserted with `AssetServer::add`.

### The startup scene

A game declares the scene it starts in from its recipe, the
`pub fn game(app: &mut App)` that its `main` passes to `concerto::run`:

```rust
app.register_plugin(DefaultPlugins)
    .set_startup_scene(asset_id!("content/forest/scene.gasset"));
```

`ScenePlugin` (part of `DefaultPlugins`) spawns that scene from a `Startup`
system in the game's `App`. The editor builds its edit `App`s from the same
recipe and does not spawn the startup scene in them. Load any other scene with
`asset_server.load::<Scene>` as above.

`concerto::run` sets up logging before it builds the `App`: `env_logger`
natively (filtered by `RUST_LOG`), and the browser console with a panic hook on
wasm. A game does not call `env_logger::init()` itself.

## Derived data

An asset whose runtime part is computed from other content assets implements
`DerivedData` (`concerto_foundation::assets::derived`). Its `.gasset` file holds
only what a user authors; the computed part, a blob, is never saved into the
content tree. `MotionDatabase` is one: its file is the definition, and its
`BakedDatabase` is derived from the skeleton and clips it names.

| Item | Meaning |
|---|---|
| `DERIVED_VERSION` | Raised when `derive` gives a different blob for the same inputs |
| `is_derived()` | Whether this asset has a blob to derive; `true` unless the type overrides it |
| `file_is_derived(&header, payload)` | Whether a file of the type derives, from its header and payload; by default it decodes the payload, migrating, and asks `is_derived()`. A type overrides it when decoding all of it is slow |
| `derived_inputs()` | The content assets the blob is built from; none may itself derive |
| `derived_key(&inputs)` | A `u64` that is equal for inputs giving an equal blob |
| `derive(&inputs, progress)` | Builds the blob, calling `progress(finished, total)` |
| `apply_derived(&blob)` | Takes the blob into the asset's runtime part |

Whether an asset derives is decided per asset, not per type. One whose
`is_derived()` is false is a plain asset of a derived type: it loads as it is,
reading no inputs, computing no key and touching no cache, and a cook skips it.
A derived type whose assets all derive, such as `MotionDatabase`, keeps the
default.

`DerivedInputs::read` reads each input's file from the content root as a load
does. An input whose kind is a registered derived type is asked
`file_is_derived`, and refused when it answers true, with an error naming the
input's id, its address and its kind and saying it is itself derived. A plain
asset of a derived type is accepted, and a file of a kind that isn't derived is
never looked at. `bytes(id)` gives the whole file and `decode::<T>(id)` decodes
it, migrating as a load does but without `on_load`.

A type is registered with `app.register_derived_asset::<A>()`, which also
registers it as an asset. A load of it runs `on_load`. An asset that doesn't
derive is then loaded as it is, and a derived section on it, which only a
hand-made file can have, is ignored with a warning. One that derives applies
the blob of a cooked file's derived section when its `derived_version` is the type's
`DERIVED_VERSION`, reading no inputs and touching no cache. Without a section,
or when the section's version differs or `apply_derived` refuses it, with a
warning, the load reads the inputs, computes the key and looks the blob up in
the `AssetServer`'s `DerivedDataCache`. A hit is applied; a miss, or a cached blob that
`apply_derived` refuses, is derived, written to the cache and applied. The
handle reads as loaded only once the blob is applied, and a missing input, an
input that itself derives, or a failed derive fails the load. `AssetServer::add` derives nothing: code that adds a
derived asset in memory attaches the computed part itself.

The key covers the definition and the bytes of its inputs, not code. A change
to an engine `derive` raises `DERIVED_VERSION`. Code a game registers, such as
the `extract` of a game's own `FeatureChannel`, is in no key and no version, so
after changing it the cache still holds blobs derived by the old code and a
load applies them. Delete the project's `derived-cache/` to derive them again.

The cache is a directory of files at
`<cache>/<TypeName>/v<DERIVED_VERSION>/<key as 16 hex digits>.blob`, created
on the first write. A write goes to a uniquely named temporary file that is
renamed into place, so two processes writing one key do not collide. A read or
write error is logged and the load still succeeds. The editor, a game it hosts
and a native `concerto::run` all use `<project>/derived-cache/`, which the
project's `.gitignore` lists, so a blob derived in one is a hit in the others.
An `App` that sets no cache, such as a test, derives on every load. The web
build has no cache: an uncooked web build, such as `trunk serve`, derives in
the browser, on the main thread. A cooked build carries the blobs instead.

### Cooking

A cook writes the blob of each asset that derives into a copy of its `.gasset` file, as a
trailing derived section, so a shipped build applies it without deriving. It is
run through the project tool:

```sh
cargo concerto build <dir> --cook [--release]
cargo concerto build <dir> --cook [--release] --target wasm32-unknown-unknown [--public-url <url>] [--dist <dir>]
cargo concerto run <dir> --cook [--release] [-- <game args>]
```

`run` does what the native `build` does, then runs the game with the arguments
after `--`.

1. The game is built natively, in the requested profile.
2. It runs as `<game> --cook <target>/cooked/<pkg>`. Every `concerto::run`
   binary accepts `--cook <out>`, which calls `concerto::cook` instead of opening
   a window. That builds the recipe's `App` with no `Gpu`, `Window` or
   `DrawTarget`, runs each plugin's `build` and never its `finish`, and runs no
   frame. It points the `AssetServer` at the project's `content/` and its
   `derived-cache/`, and calls `AssetServer::cook`.
3. `AssetServer::cook` takes every registry entry whose kind is a registered
   derived type, in address order. It asks the type's `file_is_derived`
   first. An asset that doesn't derive is skipped: it isn't decoded, no file is
   written and the `CookReport` leaves its address out. One that derives is
   decoded and its `on_load` runs; then it derives or takes a cache hit as a
   load does, and
   writes `<out>/<address>`: the original header and payload bytes, unchanged,
   plus the section. A cached blob that `apply_derived` refuses is derived
   again, never cooked. The first failure stops the cook and fails the command,
   naming the asset.
4. The cooked files are *staged*: each is copied over the file at the same
   path in what the build ships, `<exe dir>/<exe stem>-content/` natively,
   or `<dist>/` after `trunk build` for the web. A cooked file whose path the
   build doesn't ship is skipped.

The committed `content/` never holds a section, and `build.rs` and
`index.html` don't change. A later `cargo build` that changes `content/` reruns
`build.rs`, which copies the raw files back over the cooked ones, and the game
derives at load again. A load applies a cooked blob without checking its key,
so a change to derive code that doesn't raise `DERIVED_VERSION` leaves the
staged blob derived by the old code, as it leaves a cached one: cook again
after such a change.

`cook` writes a `.concerto-cook` marker into `out` and refuses a non-empty
directory without one, so `--cook` never empties a directory it didn't write.
An `<out>` that starts with `-` is refused as a flag, so `--cook --forest` fails
instead of cooking into `./--forest`.
The cook needs no GPU: a plugin that needs the `Gpu` must reach it in `finish`,
not in `build`.

## Editor-authored assets

Some assets have no DCC source — a `Scene` assembled in an editor, for
instance. `save_content_asset_into` writes one directly:

```rust
use concerto_foundation::assets::content::save_content_asset_into;

let id = save_content_asset_into(&scene, project_root, "content/levels/intro.gasset", &mut registry)?;
```

It writes the `.gasset` file (with `provenance: None`), keeping the id already
in that file's header, and upserts the entry in `registry`, an `AssetRegistry`
in memory. No registry file is written. This API is separate from source
importing and does not create a source sidecar. The `project_root` is the
source tree under version control — deliberately *not* the exe-relative
runtime root, so a save is not overwritten by the next build.

A file written with `provenance: None` belongs to the editor: a re-import of
the source it came from leaves it untouched, and pruning keeps it. To save an
edit to an imported asset and leave it owned by its source,
`resave_content_asset` writes over the existing file instead:

```rust
use concerto_foundation::assets::content::resave_content_asset;

let id = resave_content_asset(&clip, project_root, "content/walk/animation_walk.gasset", &mut registry)?;
```

It fails when there is no file at the address or the file holds another kind,
keeps the header's id and provenance, writes the references and
`kind_version` of `value`, and upserts the entry in `registry` with the same
provenance. The next re-import of the source then overwrites the saved edit.

## Writing a new importer

An importer lives in whatever crate owns the asset types it produces and
implements `asset_import::Importer`:

```rust
use asset_import::{ImportContext, ImportError, Importer};

pub struct MyImporter;

impl Importer for MyImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["myext"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let doc = parse(source_path)?;

        // Emit each sub-asset by name. `emit` serializes the value, records
        // its resolved AssetId, and captures its `referenced_sub_assets()`.
        for (i, mesh) in doc.meshes().enumerate() {
            ctx.emit(&format!("mesh/{i}"), &mesh_to_engine_mesh(mesh))?;
        }

        // Build same-file cross-references with `sub_asset_id`, so a struct
        // you emit can hold a real `AssetHandle::weak(id)` that resolves to
        // another sub-asset's persistent UUID.
        let mesh_id = ctx.sub_asset_id("mesh/0");

        // Record external inputs (an external .bin, a sibling texture) so a
        // future incremental import can tell the source changed.
        ctx.track_dependency(external_path, asset_import::hash_file_contents(&external_path)?);

        Ok(())
    }
}
```

Then register it in `crates/import/src/lib.rs::registered_importers()`.

`ImportContext::emit<T: Asset>` requires the emitted type to implement
`Asset` (`Serialize + DeserializeOwned` + a `name()` tag). There is no
separate DTO trait — the on-disk form of an asset *is* the asset.

## Current limitations

- **glTF `extras` are not read.** The Blender add-on that wrote game components
  into `extras` is deprecated. Add game components to the imported scene in the
  editor and save it; a saved scene is left alone by later re-imports.
- **No incremental import.** Re-import regenerates the source's outputs
  while preserving their identities. `track_dependency` records inputs but
  nothing consumes them yet.
- **No cross-source references.** An importer that emits a reference to a
  sub-asset from a *different* source file logs a warning and leaves it
  unresolved (e.g. an OBJ material pointing at a standalone texture).
- **Quantised / morph-target animation panics.** `GltfImporter` hits
  `todo!()` on I8/U8/I16/U16 rotation channels and morph-target weight
  outputs — re-export uncompressed for now.
- **Sub-asset matching still uses importer keys** such as `mesh/0` and
  `animation/Idle_Loop`. Reordering objects in a DCC export can change what these keys
  mean. Matching across such changes is deferred.
- **Pruning skips editor-saved outputs.** Re-import deletes the `.gasset` and
  the metadata key of every output the importer stopped emitting, and removes
  it from the editor's registry. An output saved from the editor with
  `save_content_asset_into` (its header has no provenance) is kept with its
  key; one saved with `resave_content_asset` keeps its provenance and is
  pruned like any other output. Pruning an output that another asset
  still references logs a warning naming the output and the referrer.
- **A renamed output keeps its id only with a hint.** An importer that renames a
  sub-asset calls `ctx.renamed_from(new_name, old_name)` before it first calls
  `sub_asset_id` or `emit` for `new_name`; the new name then takes over the old
  name's id, and the file keeps its address. The glTF and BVH importers do this
  for clips renamed from `animation/{index}` to `animation/{name}`. Other
  importers get the mechanism only.
- **Moved content is seen at the next build.** A game finds a moved `.gasset`
  once it is rebuilt, and an open editor once the project is reopened; there
  is no runtime file watcher yet.
- **`ImageImporter` always imports as sRGB.** A per-entry colour-space
  hint would need to come from `content.toml` or a filename convention.
