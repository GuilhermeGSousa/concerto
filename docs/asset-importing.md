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
the editor, not by a hidden build step, so a stale tree is a visible diff.
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
importer's defaults, and a first import writes none. The `gltf`, `obj` and
`image` importers read no settings. The `bvh` importer's are described under
[BVH motion capture](#bvh-motion-capture).

```toml
[settings]
scale = 0.01
frame_range = [10, 250]
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

A `frame_range` outside the file, an empty or one-frame range, a bad `scale`,
or an unknown key fails the import. The file's own timing is kept; there is no
resampling setting.

## The `.gasset` file format

```
magic "GRDY" (4 bytes)
u32   header length, little-endian
      bincode(ContentAssetHeader)
      payload — the asset's own bincode encoding, verbatim
```

A skeleton asset (`skeleton/N`) holds bone names, parent indices (each parent before its child), inverse bind poses, the rest pose as `JointPose` and the bone ids. `JointPose` lives in `concerto-mesh`; `concerto-animation` re-exports it. A skin whose joints are not ordered parents-first fails the import with an error naming the skin and joint; a skin with a joint outside every scene root is skipped with a warning. Neither emits a skeleton.

Format version 4 rejects every `.gasset` written at version 3. After upgrading, re-import every source through the editor over a scratch copy of the project and commit the result. glTF clips have no stored frame rate, so the clip editor shows a detected one; BVH clips store the rate from `Frame Time`.

`ContentAssetHeader` (see `concerto_foundation::assets::content`):

| Field            | Meaning                                                        |
|------------------|---------------------------------------------------------------|
| `format_version` | `CONTENT_FORMAT_VERSION` (currently `4`); a mismatch is rejected on read; there is no decoder for older files |
| `asset_id`       | Persistent UUID, minted once and reused from source metadata; legacy UUIDs are preserved |
| `references`     | the `AssetId`s this asset points at, so a tree can be integrity-checked by scanning headers alone |
| `kind`           | the asset type tag (`Mesh`, `StandardMaterial`, `Scene`, …); must match the loading type |
| `provenance`     | `Some(ImportProvenance { source, sub_asset })` for import output, `None` for editor-authored assets |

The header can be read without touching the payload, which is what lets a
future tool index a whole content tree cheaply.

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
nothing to disambiguate and the default is correct. Trunk's
`<link data-trunk rel="copy-dir" href="content" …>`
in `index.html` ships the tree.

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
`concerto::content_registry!()` embeds the file; it needs a `build.rs` that
calls `track_assets`. Cargo watches that tree and `content.toml` as well as the
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
- **Obsolete outputs are retained.** Re-import does not delete sub-assets
  that the source stopped emitting. Renaming clips therefore leaves the old
  `animation_N` outputs and their metadata keys behind next to the new ones;
  cleaning them up is tracked in issue #269.
- **Moved content is seen at the next build.** A game finds a moved `.gasset`
  once it is rebuilt, and an open editor once the project is reopened; there
  is no runtime file watcher yet.
- **`ImageImporter` always imports as sRGB.** A per-entry colour-space
  hint would need to come from `content.toml` or a filename convention.
