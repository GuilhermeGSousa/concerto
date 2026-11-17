# Asset importing

The engine does not load DCC files (`.gltf`, `.glb`, `.obj`, `.bvh`, `.png`, …)
directly at runtime. They are **imported** once, ahead of time, into a tree
of self-describing binary files that the game references by UUID. This is
the Unreal/Godot model: a source artist file is an input to a build step,
not something the shipped game reads.

```
assets/hero.glb   ──(import)──▶   content/hero/scene.gasset
                                    the scene, with the meshes, materials
                                    and skeletons embedded in it
                                  content/hero/texture_0.gasset
                                  content/hero/animation_Idle_Loop.gasset
assets/hero.glb.import.toml       (persistent source/output identities)
```

- **Source files** live wherever you like (a game's `assets/` by
  convention). They are never read by the running game. A texture's
  `.gasset` holds its image, so the file in `assets/` is needed
  only to import it again.
- **Content assets** (`*.gasset`) are the game-ready form, each a small
  header plus a `bincode` payload. A source's scene is one file that also
  holds its meshes, materials and skeletons, each still an asset with its own
  UUID; textures, animation clips and everything else are a file each. They
  are committed to version control alongside the code.
- **Source metadata** (`<source>.import.toml`) is committed beside each
  imported source. It records which UUID belongs to each sub-asset.
- **Addresses** are plain paths (`content/hero/scene.gasset`), and an
  embedded asset is named by its file's address, `#`, and its name inside the
  file (`content/hero/scene.gasset#mesh/0`). Code uses
  `asset_id!("content/hero/scene.gasset#mesh/0")` to resolve a UUID during
  compilation; the runtime uses UUIDs to locate assets through the registry.
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
shows, with every file it refers to. A file the importer does not read itself,
such as a texture an OBJ's `.mtl` names, is skipped with a warning naming it when
it cannot be read, and the import goes on without it. A missing `.mtl`, or a
glTF's missing `.bin` or external image, still fails the import. To
re-import a game's sources, run its editor over a scratch copy of the
project, never over the game's own directory:

```sh
cd ../tech-demo
cargo run -p tech-demo-editor -- --project <scratch copy of the tech demo>
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
| `.ogg` `.wav`        | `AudioImporter` | one `main` audio clip                             |

A texture's `.gasset` holds its **source**, the image as PNG or JPEG bytes,
its **role** (`Color`, `Normal` or `Data`, set by the importer) and its
**settings**: compression (`Auto`, `Bc7`, `Bc1`, `Bc5`, `Bc4`, `Uncompressed`),
colour space (sRGB or linear, by default the role's: sRGB for `Color`, linear
otherwise), max size and mips. Every level the GPU samples is derived data:
when the texture loads, the levels are derived from the source into
`derived-cache/`, or read from there, and cooked builds carry them in the
file. `Auto` is BC7 for a colour texture with alpha below 255, BC1 for an
opaque one, BC5 for a normal map and BC7 for data. A texture is first fitted
within its max size, then, when compressed, rounded to a multiple of 4 on each
side; its mips halve down to 1×1, averaging sRGB colour in linear space and
renormalising normals. BC4 and BC5 have no sRGB form, so a texture set to
either with an sRGB colour space fails to derive and asks for Linear. The
image importer gives its texture the `Color` role; the glTF importer gives
base colour and emissive textures `Color` and the others `Data`. The loaded
texture keeps its levels, not its source. On the web there is no encoder, so
an uncooked run derives uncompressed levels; a cooked build carries the
compressed ones. A device without BC texture compression (Safari, and some
GPU and driver combinations on the web) refuses BC textures: each one logs an
error naming the texture and the fix, and its material samples the dummy
texture, a transparent black texel, in its place. Set such a texture's
compression to `Uncompressed` to show it there. The renderer samples the chain trilinearly with up to 16×
anisotropic filtering, as much as the adapter supports. Code builds a sampled
RGBA8 texture with `Texture::new`; it builds its mip chain when it loads,
through `AssetServer::load` or `add`, and never derives.

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
default_scene = "content/sanatorium/level.gasset"
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

A source that emits a `scene` writes one host file for it, and its meshes,
materials and skeletons are embedded in that file instead of getting files of
their own. `concerto_import::embeds_by_default(kind)` is the rule: `Mesh`,
`StandardMaterial` and `Skeleton` embed. Textures, animation clips and every
other kind are loose files beside the scene, and so is every output of a source
that emits no scene, such as an image. An embedded asset keeps its own id and is
named `<scene address>#<sub-asset name>`:

```
assets/hero.glb  ──▶  content/hero/scene.gasset            the scene, the host
                      content/hero/scene.gasset#mesh/0      embedded in it
                      content/hero/scene.gasset#material/0
                      content/hero/scene.gasset#skeleton/0
                      content/hero/texture_0.gasset         loose
                      content/hero/animation_Idle.gasset    loose
```

A BVH import is `scene.gasset`, holding the scene and `skeleton/0`, plus the
clip; an OBJ import is `scene.gasset` alone.

An import builds the host file whole and replaces the old one through a
temporary file. Nothing else writes into a host. A re-import of a source whose
meshes, materials and skeletons are still loose files, as every import was
before embedded assets, keeps every id, writes them into the scene file and
deletes the loose files. A scene file that has moved keeps its address.

The import returns an `AssetEntry` for every asset, embedded ones with
`embedded_in`, and the editor's registry follows an output that changed between
loose and embedded from those entries.

### Converting a project without importing

`pack-imports` does that conversion for a whole project at once, without
reading a source or decoding a payload:

```sh
cargo run -p concerto-import --bin pack-imports -- <project directory>
```

It follows the project's `.import.toml` sidecars. For each source whose `scene`
output is a file the import owns, it writes that file at version 7 with the
source's loose meshes, materials and skeletons embedded, then deletes those
loose files and any folder left empty. An output with `file = true`, an output
saved from the editor, and every texture and clip stay loose and untouched.
Ids, kinds, kind versions, references, provenance and payload bytes are
carried over exactly, and sidecars are not changed.

It prints `<source>: embedded <n>` for each source it handled, and
`<source>: skipped, <reason>` for each it left alone: a source with no scene
output, one whose scene was saved from the editor or is missing, and content
whose source has no sidecar, which importing the source again converts. It
scans the whole tree first and stops before writing anything when a header is
unreadable, two assets share an id or a sidecar disagrees with the content.
Each scene file is replaced through a temporary file before its loose files
are deleted. A second run embeds nothing and changes no file.

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

An output's table takes `file = true` to keep that output out of the scene file:

```toml
[outputs."material/steel"]
asset_id = "d6e76e64-a888-49cd-bc16-61f0566f36ab"
file = true
```

The next import writes the output as a loose file at its usual address
(`content/hero/material_steel.gasset`) with the same id, and leaves it out of
the scene file. Removing the key and importing again embeds it and deletes the
loose file. If another asset already sits at the loose address, the import
fails before writing anything. `file = true` on an output that is loose anyway,
or on `scene`, changes nothing, and `file = false` reads as the key being
absent and is not written back. Like `[settings]`, it is edited by hand: import
once, edit, import again. This is how one imported material or mesh becomes a
file the editor can save on its own.

An optional `[settings]` table holds importer-specific settings. The importer
that owns the source reads it on every import; a sidecar without one uses the
importer's defaults, and a first import writes none. The `obj`, `image` and
`audio` importers read no settings. The `gltf` importer reads only `events` and
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
      embedded asset 0 — its payload, then its derived blob if it has one
      embedded asset 1 — …
```

A skeleton asset (`skeleton/N`) holds bone names, parent indices (each parent before its child), inverse bind poses, the rest pose as `JointPose` and the bone ids. `JointPose` lives in `concerto-mesh`; `concerto-animation` re-exports it. A skin whose joints are not ordered parents-first fails the import with an error naming the skin and joint; a skin with a joint outside every scene root is skipped with a warning. Neither emits a skeleton.

The format version is `CONTENT_FORMAT_VERSION`, currently `7`; it changes only when the framing or the header changes, never for a payload. Version 5 adds `kind_version` to the header, and version 6 adds `derived`, the description of a derived blob that trails the payload in a cooked file. The payload is everything between the header and that blob; `read_content_asset` returns it without the blob, and `read_content_asset_parts` returns both. `write_content_asset` writes no section and refuses a header that has one; `write_cooked_content_asset` writes the header, the payload and the blob of a `CookedSection`. Committed content never has a section. A version-4 file reads as kind version 1 and a version-5 file reads with no section; a version-6 file reads with no embedded assets. None needs a re-import, and each moves to version 7 the next time the editor saves it or an import rewrites it. Any other version is rejected. glTF clips have no stored frame rate, so the clip editor shows a detected one; BVH clips store the rate from `Frame Time`.

`ContentAssetHeader` (see `concerto_foundation::assets::content`):

| Field            | Meaning                                                        |
|------------------|---------------------------------------------------------------|
| `asset_id`       | Persistent UUID, minted once and reused from source metadata; legacy UUIDs are preserved |
| `references`     | the `AssetId`s this asset points at, so a tree can be integrity-checked by scanning headers alone |
| `kind`           | the asset type tag (`Mesh`, `StandardMaterial`, `Scene`, …); must match the loading type |
| `kind_version`   | the `Asset::FORMAT_VERSION` of `kind` that the payload was written with; `1` for a version-4 file |
| `provenance`     | `Some(ImportProvenance { source, sub_asset })` for import output, kept when `resave_content_asset` writes over it; `None` for editor-authored assets and for an imported asset saved with `save_content_asset_into` |
| `derived`        | `Some(DerivedSection { derived_version, key, len })` for a cooked file: the type's `DERIVED_VERSION` and `derived_key` at cook time and the blob's length in bytes; `None` otherwise, and for a file older than version 6 |

| `embedded`       | the directory of the assets stored in this file after the host's payload and blob, one `EmbeddedAsset` each, in file order; empty for a loose file, and for a file older than version 7 |

The header can be read without touching the payload, which is what lets a
future tool index a whole content tree cheaply.

Version 7 lets one file hold several assets. The asset the file is for is the
**host**; the others are **embedded** in it, and an asset alone in its file is
**loose**. An `EmbeddedAsset` holds what a loose file's header would hold for
that asset (`asset_id`, `references`, `kind`, `kind_version`, `provenance`,
`derived`) and the byte range of its payload: `offset`, counted from the end
of the header, and `len`. Its derived blob, of `derived.len` bytes, follows its
payload. Its `provenance` is not optional, because `provenance.sub_asset` is
its name inside the file (`mesh/0`), which `EmbeddedAsset::name()` returns.
Embedded assets are stored in directory order, so the host's payload ends
where the first one starts, less the host's blob. An embedded asset lists no
embedded assets of its own.

`write_content_asset_with_embedded` writes a host from its header, with an
empty directory, its payload and one `EmbeddedPayload` per embedded asset, and
fills the directory. It refuses a host header with a derived section, and an
embedded asset that has no provenance, lists embedded assets, has a blob its
header does not describe, or repeats an id or a name. `write_content_asset`
and `write_cooked_content_asset` refuse a header that lists embedded assets.
`read_content_asset` and `read_content_asset_parts` of a host return the
host's payload and blob, with the directory in the header. `asset_record`
returns one asset of a file as a standalone record, the bytes a loose
version-7 file of it would have: the host for `None`, an embedded asset by
name. `ContentAssetHeader::record_header` gives that record's header without
reading a payload. Every reader rejects a directory whose ranges overlap, are
out of order or run past the file, or that repeats an id or a name, with an
error that names the asset.

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
| `crates/mesh/tests/mesh/mesh_serialization.rs` | `Mesh`, `Skeleton` |
| `crates/render/tests/render/material_serialization.rs` | `Texture`, `StandardMaterial` |
| `crates/animation/tests/animation/graph_serialization.rs` | `AnimationClip`, `AnimationGraph`, `BoneMask`, `RetargetRig` |
| `crates/scene/tests/scene/scene_serialization.rs` | `Scene` |
| `crates/motion-matching/tests/motion-matching/serialization.rs` | `MotionDatabase` |
| `crates/navigation/tests/navigation/navmesh_asset.rs` | `NavMesh` |
| `crates/scene/tests/scene/glam_encoding.rs` | `GlamLeaves`, the reflected `Transform`, `Vec3`, `Quat` and `Mat4` leaves |

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
   `CONCERTO_BLESS_ENCODING=1 cargo test -p <crate> --test <dir>`, where
   `<dir>` is the directory under the crate's `tests/` that holds its
   `main.rs`, run the test again without the variable, and commit the new
   `.bin` file. Keep the old version's fixture while its `migrate` arm
   exists, with a test that passes it through `decode_content_asset` and
   checks the converted value; delete it when you choose rejection.
4. Migration happens in memory: the file keeps its old version until it is
   saved, and a save writes the current `FORMAT_VERSION`. Re-import the
   affected sources, or re-save editor-authored assets, and commit the
   rewritten content.
5. A legacy struct and its `migrate` arm may be deleted once no committed
   `.gasset` under `examples/*` or in the sibling game repositories
   (`../tech-demo`, `../one-more-night`) carries that version. Its fixture
   and test go with it, and the PR says so. After that, files at that version are
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
in the tech demo's repository under `content/UAL1/` and
`content/lafan1/animations/` are still version 1: they
migrate in memory on every load, and the editor reports them as outdated until
they are re-imported. Like a graph's nodes, each event is stored as its Rust
type name and reflected fields. A clip makes its events live when it loads; an
event whose type is not registered with `register_clip_event`, or whose stored
value leaves a warning, is dropped with a warning naming the clip and the type.

`Scene` is at version 2, which gives each node an `id` and adds `link`:
`None`, or the `AssetId` of a linked scene and the changes made to its nodes.
It has no migration: a Scene file before version 2 is refused, so an imported
scene must be re-imported from its source, and one authored in the editor
re-created.

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
version keeps the previous ones readable where it can, as version 7 reads
versions 4, 5 and 6. The version-7 framing is pinned by
`crates/foundation/tests/encoding/content_asset.v7.bin`, rewritten under
`CONCERTO_BLESS_ENCODING=1`; `content_asset.v6.bin` stays as it was written,
with a test that reads it.

## The asset registry

The registry lists every content asset's `AssetId` with its address and kind.
No registry file is committed or read at runtime. A game's build writes it as
TOML to `$OUT_DIR/concerto_registry.toml` and embeds it (see
[Loading at runtime](#loading-at-runtime)):

```toml
[assets]
9f3c…e21a = { address = "content/UAL1/scene.gasset", kind = "Scene" }
b75e…8457 = { address = "content/UAL1/animation_Idle_Loop.gasset", kind = "AnimationClip" }
41d0…77c2 = { address = "content/hero/scene.gasset", kind = "Mesh", embedded_as = "mesh/0" }
```

A file can hold several assets: its host, and the assets embedded in it. Each
has its own entry, all at the file's address, and an embedded one also has
`embedded_as`, its name inside the file. Written out, an embedded asset's
location is the address, `#`, and that name:
`content/hero/scene.gasset#mesh/0`. A line without `embedded_as` is a host or a
loose asset, so registry text written before embedded assets existed still
parses. A line with `embedded_as` needs the line of its host, the one at the
same address without it; registry text that has none fails to parse.

`AssetRegistry` (`concerto_foundation::assets::registry`) is the registry in
memory, and a resource in every `App` that has `AssetManagerPlugin`. Each
`AssetEntry` has the id, the address, the kind, `embedded_in` (for an embedded
asset, the id of its host and its name in the host's file), and, when the
registry was built by scanning headers, the import provenance. `file()` is the
id of the asset whose file holds the entry: the host of an embedded asset, the
asset itself otherwise. A file is identified by that id, not by its address. List assets by type without loading
them, as with Unreal's `GetAssetsByClass`:

```rust
fn list_meshes(registry: Res<AssetRegistry>) {
    for entry in registry.of_type::<Mesh>() {
        println!("{} in {} ({:?})", entry.name(), entry.folder(), entry.id);
    }
}
```

Entries come in folder, then name order. `get(id)` looks up one entry.
`id_for_address(address)` gives the host or loose asset of a file,
`id_for_location(location)` any asset by the text `AssetEntry::location` gives
(`content/hero/scene.gasset#mesh/0`), and `in_file(host)` every entry of the
file of the host or loose asset with that id, host first. `name()` is the file name without its extension, and for an
embedded asset the name its own file would have (`mesh_0` for `mesh/0`). Two
entries never share an id or a location; inserting one replaces whichever held
its id, its address when neither is embedded, or its name in the same host. React to catalogue changes with `registry.is_changed()`.

A scene's serialized `MeshComponent` carries a bare `AssetId` (no path); when
the spawner upgrades that `Weak` handle to `Strong`, `AssetServer` resolves the
id to an address through the registry. Code-authored references already contain
the UUID emitted by `asset_id!`. `AssetServer` holds a snapshot of the resource
that its load tasks resolve against, replaced whenever the resource changes. A
`load::<T>` whose id the registry lists under another kind fails before reading
the file.

The editor scans the content tree once, when a project opens. After that, an
import inserts the entries it returns, a New in the Asset Browser inserts the
created asset's entry (New lists the kinds with a creatable editor: `Scene`,
and with the animation editors `RetargetRig`, `AnimationGraph` and
`MotionDatabase`), and a scene save upserts the scene's entry, all in
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
content trees do not collide. `concerto_asset_build::profile_dir` finds
`<exe-dir>` from `OUT_DIR`, in Cargo's flat `build/<pkg>-<hash>/out` layout and
its nested `build/<pkg>/<hash>/out` one alike:

```rust
// build.rs — copy to <exe-dir>/<pkg-name>-content/content/
let output_path = concerto_asset_build::profile_dir(&out_dir)
    .ok_or_else(|| anyhow::anyhow!("OUT_DIR is not under a build directory"))?
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
errors there. An embedded asset is named by its location,
`asset_id!("content/hero/scene.gasset#skeleton/0")`: when the whole text is not
a file, the macro splits it at the first `#` that leaves a file before it and
looks the rest up among that file's embedded assets. A name the file does not
hold is a compile error that lists the names it does hold. It is also available as
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
the registry to `$OUT_DIR/concerto_registry.toml`, with one line for each asset
of a file, embedded ones included. Two assets with one id, or an unreadable
header, print their locations as a Cargo warning and fail the build.
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

An embedded asset loads by its id like any other, and the file is the unit of
loading. The first `load` of an asset whose file is not being loaded starts one
task for that file; a `load` of another asset of the file while the task runs
only adds its request to it. The task reads the file once, parses its header
once, and decodes each requested asset from its slice of those bytes, one after
another, skipping a request whose last handle was dropped before the task
reached it. When no request is left the task ends and the bytes are dropped; a
later `load` of an asset of the file starts a new task and reads it again. A
file that cannot be read or parsed fails every asset requested from it, each
logged error naming its asset as `file#name`, and an asset that fails to decode
fails alone.

`AssetServer::is_loading` reports whether an asset requested with `load` or
`add` has yet to reach the world. A load counts until the frame that inserts
its asset, or handles its failure; cancelled loads do not count.
`AssetServer::is_loading_or_loaded(id)` asks the same of one asset, and stays
true once it is in the world: it is false for an id never loaded or added, and
once its load fails, is cancelled, or it unloads.

Stored assets can only be requested by UUID. Use `asset_id!` for references
written in code and `load` for both those IDs and serialized references.
There is no runtime path-loading API or path-to-UUID lookup. Paths are resolved
internally from the registry when reading asset bytes.

Configure the root before content loading starts; changing projects requires a
separate server. Procedural assets can still be inserted with `AssetServer::add`.
`AssetServer::insert_loaded(id, asset)` puts a built asset under an id of the
caller's choosing instead of a fresh one: it runs `on_load`, drops a load of
that id still pending, and from then on `load(id)` returns the inserted asset
and reads no file. Inserting again under the same id replaces it. An editor
uses it to have a preview play a working copy in place of the saved file.
`AssetServer::reload(id)` reads the file again, whether the asset is loaded or
was inserted, and replaces it once the read lands, which is how that preview
goes back to the saved file.
`AssetServer::name_asset::<A>()` is for a server that edits references to a
kind it must not load, as the editor's does for `AnimationGraph`: a handle to
`A` set through it is weak, naming its asset and loading nothing, and
`names_asset::<A>()` says whether that is so.

### The startup scene

A game declares the scene it starts in from its recipe, the
`pub fn game(app: &mut App)` that its `main` passes to `concerto::run`:

```rust
app.register_plugin(DefaultPlugins)
    .set_startup_scene(asset_id!("content/sanatorium/level.gasset"));
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
| `further_inputs(&inputs)` | Further content assets named by the first inputs, such as the meshes a scene places; empty unless the type overrides it, and under the same rules as `derived_inputs()` |
| `derived_key(&inputs)` | A `u64` that is equal for inputs giving an equal blob |
| `derive(&inputs, progress)` | Builds the blob, calling `progress(finished, total)` |
| `apply_derived(&blob)` | Takes the blob into the asset's runtime part |

Whether an asset derives is decided per asset, not per type. One whose
`is_derived()` is false is a plain asset of a derived type: it loads as it is,
reading no inputs, computing no key and touching no cache, and a cook skips it.
A derived type whose assets all derive, such as `MotionDatabase`, keeps the
default.

`DerivedInputs::read` reads each input's file from the content root as a load
does, and keeps each file it reads for its own lifetime, so a host file is read
once for every input embedded in it. An input whose kind is a registered derived type is asked
`file_is_derived`, and refused when it answers true, with an error naming the
input's id, its address and its kind and saying it is itself derived. A plain
asset of a derived type is accepted, and a file of a kind that isn't derived is
never looked at. `read_more` reads further ids into the same inputs under the
same rules, skipping ids already read. `bytes(id)` gives the whole file of a
loose input, and for a host or an embedded one its standalone record: the bytes
a loose file of it would have, so a key over a host does not change when an
asset embedded in it does. `decode::<T>(id)` decodes either, migrating as a load
does but without `on_load`. A key over `bytes(id)` is therefore the same for an
asset embedded in a host file and for the same asset written as a loose file.

A type is registered with `app.register_derived_asset::<A>()`, which also
registers it as an asset. A load of it runs `on_load`. An asset that doesn't
derive is then loaded as it is, and a derived section on it, which only a
hand-made file can have, is ignored with a warning. One that derives applies
the blob of a cooked file's derived section when its `derived_version` is the type's
`DERIVED_VERSION`, reading no inputs and touching no cache. Without a section,
or when the section's version differs or `apply_derived` refuses it, with a
warning, the load reads the first inputs, asks `further_inputs` once with them
and reads the further ones too, under the same rules, then computes the key
over every input and looks the blob up in the `AssetServer`'s
`DerivedDataCache`. A hit is applied; a miss, or a cached blob that
`apply_derived` refuses, is derived, written to the cache and applied. The
handle reads as loaded only once the blob is applied, and a missing input, an
input that itself derives, first or further, or a failed derive fails the load. `AssetServer::add` derives nothing: code that adds a
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
   again, never cooked. An embedded asset that doesn't derive is skipped too;
   one that derives fails the cook, with an error giving its `<address>#<name>`
   and saying embedded assets cannot be cooked yet. No importer writes one. The
   first failure stops the cook and fails the command, naming the asset.
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
An `<out>` that starts with `-` is refused as a flag, so `--cook --sanatorium` fails
instead of cooking into `./--sanatorium`.
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

Both refuse an address whose file holds embedded assets, and leave the file as
it was. Such a file belongs to an import, which writes it whole; nothing
rewrites one asset inside it. The error gives the address, says the file is
read-only, and names the way on: set `file = true` for an output in the
source's `.import.toml` and import again, which makes that output a file of
its own.

`read_asset_record(project_root, &entry)` reads one asset out of the tree
whatever way it is stored: the standalone record of a registry entry, loose,
host or embedded.

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

        // Record external inputs (an external .bin, a sibling texture) so
        // importing from outside the project copies them with the source.
        ctx.track_dependency(external_path);

        Ok(())
    }
}
```

Then register it in `crates/import/src/lib.rs::registered_importers()`, give
its extensions an importer name in the match in `import_placed` there, accept
that name in the sidecar check in `crates/import/src/metadata.rs`, and add the
extensions to the editor's file picker, `SUPPORTED_EXTENSIONS` in
`crates/editor/src/import/mod.rs`.

`ImportContext::emit<T: Asset>` requires the emitted type to implement
`Asset` (`Serialize + DeserializeOwned` + a `name()` tag). There is no
separate DTO trait — the on-disk form of an asset *is* the asset.

## Current limitations

- **glTF `extras` are not read.** The Blender add-on that wrote game components
  into `extras` is deprecated. An import's scene is read-only, so place it in a
  scene of your own in the editor and add game components there.
- **A scene saved over an import's scene stops the import.** When the file that
  holds the sidecar's `scene` id has no provenance, because the editor saved it
  with `save_content_asset_into`, it cannot hold the import's meshes, materials
  and skeletons, and the import fails before writing anything, naming the file.
  Delete the `[outputs.scene]` table from the sidecar, move the file if it sits
  at the import's own `scene.gasset` address, and import again: the import
  writes a fresh scene under a new id and the saved scene keeps its id.
- **No incremental import.** Re-import regenerates the source's outputs
  while preserving their identities. `track_dependency` records the files a
  source refers to, which an import from outside the project copies beside it
when they can be read;
  nothing records their contents to tell whether they changed.
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
  it from the editor's registry. An embedded output is pruned by leaving it out
  of the rewritten scene file. An output saved from the editor with
  `save_content_asset_into` (its header has no provenance) is kept with its
  key, and one that would embed stays a loose file, untouched; one saved with
  `resave_content_asset` keeps its provenance and is pruned like any other
  output. Pruning an output that another asset
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
