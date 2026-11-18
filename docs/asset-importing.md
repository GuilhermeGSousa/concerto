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
- **Import records** keep what an import needs to run again. Each output's
  header names its source and its name in it, and the import's anchor, its
  scene file or its one `main` asset, holds the importer, its settings and the
  hashes of the files it read. The content alone is enough to import a source
  again; [Import records](#import-records) describes them.
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

| Extensions           | Importer (name)           | Settings / asset settings       | Emits                                              |
|----------------------|---------------------------|---------------------------------|---------------------------------------------------|
| `.gltf` `.glb`       | `GltfImporter` (`gltf`)   | none / none                     | `mesh/*`, `material/*`, `texture/*`, `skeleton/*`, `animation/<clip name>`, `scene` |
| `.obj` (+ its `.mtl`)| `ObjImporter` (`obj`)     | none / none                     | `mesh/*`, one `material/<mtl stem>`, `scene`       |
| `.bvh`               | `BvhImporter` (`bvh`)     | `BvhSettings` / none            | `skeleton/0`, `animation/<file stem>`, `scene`     |
| `.png` `.jpg` `.jpeg`| `ImageImporter` (`image`) | none / `TextureSettings`        | one `main` texture                                |
| `.ogg` `.wav`        | `AudioImporter` (`audio`) | none / none                     | one `main` audio clip                             |

A texture's `.gasset` holds its **source**, the image file's own PNG or JPEG
bytes as imported, as the file's [uncooked section](#uncooked-data)
(`TextureSource`). Its payload holds a summary of that source (width, height
and hash), its **role** (`Color`, `Normal` or `Data`, set by the importer) and its
**settings**: compression (`Auto`, `Bc7`, `Bc1`, `Bc5`, `Bc4`, `Uncompressed`),
colour space (sRGB or linear, by default the role's: sRGB for `Color`, linear
otherwise), max size and mips. Every level the GPU samples is derived data:
when the texture loads, the levels are derived from the source into
`derived-cache/`, or read from there, and cooked builds carry them in the
file. A load reads the source only when the cache has no levels for it, and a
cooked build does not hold it. `Auto` is BC7 for a colour texture with alpha below 255, BC1 for an
opaque one, BC5 for a normal map and BC7 for data. A texture is first fitted
within its max size, then, when compressed, rounded to a multiple of 4 on each
side; its mips halve down to 1×1, averaging sRGB colour in linear space and
renormalising normals. BC4 and BC5 have no sRGB form, so a texture set to
either with an sRGB colour space fails to derive and asks for Linear; an
import whose settings for a texture would do that fails, naming the texture. The
image importer gives its texture the `Color` role; the glTF importer gives
base colour and emissive textures `Color`, normal maps `Normal`, and
metallic-roughness and occlusion textures `Data`. An image a glTF uses both as
a normal map and as data is `Data`, since BC5 would drop the blue channel the
data needs. The glTF importer keeps each image's bytes from its buffer view,
data URI or file, so a 16-bit PNG imports and its derived levels are 8-bit. The loaded
texture keeps its levels and the summary, never the source. On the web there is no encoder, so
an uncooked run derives uncompressed levels; a cooked build carries the
compressed ones. A device without BC texture compression (Safari, and some
GPU and driver combinations on the web) refuses BC textures: each one logs an
error naming the texture and the fix, and its material samples a neutral
stand-in by the texture's role: opaque white for a colour texture, a flat
normal for a normal map, and white for a data texture, so metallic-roughness
and occlusion keep their factors. Set such a texture's
compression to `Uncompressed` to show it there. The renderer samples the chain trilinearly with up to 16×
anisotropic filtering, as much as the adapter supports. Code builds a sampled
RGBA8 texture with `Texture::new`, which builds its mip chain from the pixels;
it never derives. A texture whose levels were derived keeps exactly those
levels when added, so `mips: false` stays one level.

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
temporary file. Nothing else writes into a host but an extraction, which takes
one asset out of it. A re-import keeps each output where the content holds it:
an embedded output stays in the scene file, and a loose one stays loose. A
scene file that has moved keeps its address.

### Extracting an embedded output

An embedded output is made a file of its own by extracting it:
`extract_embedded(id, project_root, config, &mut registry)` moves the
embedded asset `id` out of its scene file to the address a loose output of its
name would have, `content/hero/material_0.gasset` for
`content/hero/scene.gasset#material/0`, keeping its id, its provenance and its
uncooked section. It fails before writing when `id` is not embedded (`'<address>'
is a file of its own already`) or when that address is taken. It stages both
files, the loose record and the scene file without it, then writes the loose
file and then the scene file. If the scene file cannot be written, the loose
file is deleted again and the error returned; if that delete fails too, the
error names both files and says to delete the loose one, since two files with
one id stop the project from opening. On success `registry` holds the asset as
loose. A re-import keeps a loose output loose, so the extraction sticks; there
is no way to embed it again. Deleting the loose file and re-importing puts a
new output with a new id back in the scene file. In the editor, **Extract to
file** on an asset row and the `extract_asset` MCP tool run it;
[crates/editor/README.md](../crates/editor/README.md#importing) describes
both.

The import returns an `AssetEntry` for every asset, embedded ones with
`embedded_in`, and the editor's registry follows the import from those entries.

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

Cross-references *within a source* use the UUIDs recorded in the output
headers. A scene's `MeshComponent` holds the mesh's UUID,
which remains unchanged when its `.gasset` file moves.

The content tree is **committed to git**. It is regenerated by re-importing in
the editor, not by a hidden build step, so a stale tree is a visible diff. It
holds no derived data: what is computed from other content assets is built on
load and kept outside the tree (see [Derived data](#derived-data)).
`.gasset` files are stored in Git LFS, set by `.gitattributes`. A game's build
reads every header, so it fails with `missing the GRDY magic prefix` on a
checkout that holds LFS pointers; `git lfs pull` fetches the real files.

## Import records

An import's **anchor** is its `scene` output, or its `main` output when it
emits no `scene`. glTF, OBJ and BVH imports anchor on their `scene.gasset`,
and image and audio imports on their `main` asset. An import whose importer
emits neither fails with `the importer emitted neither 'scene' nor 'main'`.

The import writes an `ImportRecord` into the anchor's header:

| Field              | Holds |
|--------------------|-------|
| `importer`         | the importer's `name()` |
| `settings_version` | the importer's `Importer::SETTINGS_VERSION` when the record was written |
| `settings`         | the importer's settings, as bincode |
| `source`           | the source as a `SourceFile { path, hash }`: its path relative to the project root, with `/` separators, and the Fnv1a 64 hash of its bytes |
| `dependencies`     | each file the importer tracked that could be read, in tracking order, by path and hash; one outside the project has a path starting with `../` |

Nothing reads the hashes yet; showing stale imports is #880. A cooked file has
no record (see [Cooking](#cooking)). `resave_content_asset` keeps the record
along with the provenance, so saving the texture tab over an image's `main`
texture keeps it.

### Finding an import's outputs

Every output's header carries `provenance { source, sub_asset }`, with
`source` relative to the project root. An import of a source owns every
content record whose provenance names that source, and matches each output by
its sub-asset name, wherever the record has moved in the content tree. An
output keeps its id and its address; a new one gets a new id at the address
[The content tree](#the-content-tree) gives. An output the importer no longer
emits is deleted.

The import fails before writing anything when:

- two records claim the same output, two anchors included:
  `'<a>' and '<b>' are both '<sub_asset>' of '<source>'; delete one and
  import again`. A copied content folder fails earlier, on its duplicate ids.
- a file without provenance sits at the address an output would be written
  to, because it was saved from the editor: `'<address>' was saved from the
  editor and is not this import's '<sub_asset>'; move or rename it, then
  import again`. This covers the anchor too. Once moved, the output takes a
  new id at its usual address. See [Editor-authored assets](#editor-authored-assets).
- a record of another source sits at that address.

### Settings

An importer's `Settings` are kept in the record as bincode, at the importer's
`SETTINGS_VERSION`. The `gltf`, `obj`, `image` and `audio` importers have no
settings (`NoSettings`); the `bvh` importer's are described under
[BVH motion capture](#bvh-motion-capture). A clip's events and additive base
are not settings: the clip owns them, as [Clip events](#clip-events) and
[Additive clips](#additive-clips) describe.

`import_source_into`'s last argument, `ImportSettings`, may carry settings for
that import. What the import uses depends on the anchor's record:

| Record | Settings given (`ImportSettings::settings`) | No settings given |
|---|---|---|
| none, and no outputs found | used | the defaults (a first import) |
| none, but outputs found (the anchor was deleted) | used | the defaults, with the warning `'<source>' has outputs but no import record; importing with default settings` |
| written by another importer | the import fails: `'<anchor>' was imported by '<a>', not '<b>'` | the same failure |
| `settings_version` differs from the importer's | used | the defaults, with a warning naming the source and both versions |
| decodes | used | the record's |
| does not decode | used | the import fails, naming the anchor |

Whatever is used is written into the new record at the importer's current
version, so a re-import given no settings keeps the ones the last import used.
`ContentScan::new` scans a project's content once, and its `staged_settings`
returns what an import of a source would use when given none: the record's
settings or the defaults, and the main asset's current asset settings. One
scan stages a whole batch of sources; it neither creates nor writes anything.
A record that cannot be used fails it as it fails the import. Settings are
set in the editor's Import dialog, or in the `settings` argument of the
`import_source` MCP tool; nothing is edited by hand.

A texture's settings are not in the record but in its own `.gasset`, and a
re-import keeps them: it refreshes the texture's source bytes and role, and
takes the settings the previous import, or the editor, wrote there, reading
only the payload and not the source section. A texture
the import creates takes the defaults. An image source's texture is its main
asset, so its settings are the `image` importer's **asset settings**, a
`TextureSettings`: given in `ImportSettings::asset_settings`, they replace the
texture's own; otherwise the texture keeps its own, or takes the defaults when
new. Only the `image` importer has asset settings; giving them to another
fails the import, naming the type it expected. A glTF's textures start at the
defaults and are changed in the texture tab. A texture whose file does not
decode at this build's `Texture` version takes the defaults as if it were
new.

The importer validates content headers, the record and planned destinations
before writing outputs. An unreadable existing header is an error; it never
silently causes a replacement UUID. Individual files are replaced using
temporary files. The entire import is not a crash-safe multi-file transaction.

### Re-importing from the content

A re-import starts from the content, not from the source: the anchor's record
says where the source is and how it was imported, so the `assets/` tree holds
nothing but source bytes. `reimport(asset, source, project_root, config,
settings)` imports again the import that `asset` belongs to, given as any of
its outputs. Without `source`, it reads the file at the provenance's `source`
path. With `source`, that file is imported instead and becomes the import's
source: a file inside the project is used where it is, and one outside it is
copied, with the files it refers to, under its own name into the directory of
the recorded source, whose extension it must have. Every output keeps its id
and its address, and its provenance is rewritten to name the new source. A
source that was moved inside the project is therefore re-imported by giving
its new path. `settings` follow the table above, so a re-import given none
keeps the record's.

`reimport` fails before writing anything when:

- `asset` has no provenance: `'<address>' was not imported; it has no
  source`.
- no `source` is given and the recorded one is missing: `'<path>', the source
  of '<anchor>', is missing; re-import it with a source`.
- the new source is already another import's source: `'<path>' is already the
  source of '<address>'; re-import that one, or pick another file`.

`import_source_into` stays the entry point for a file the user picks: when the
placed path is an existing import's source, it is a re-import of that import.
In the editor, **Re-import…** on an asset row and the `reimport` MCP tool run
`reimport`; [crates/editor/README.md](../crates/editor/README.md#importing)
describes both.

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

Settings, `BvhSettings`, kept in the import record:

| Field         | Default | Meaning |
|---------------|---------|---------|
| `scale`       | `0.01`  | Multiplier applied to every offset and translation. The default turns the centimetres most BVH files use into metres. Must be finite and positive. |
| `frame_range` | `None`, the whole file | `Some(FrameRange { start, end })`: the frames to keep, from `start` up to but not including `end`. Needs `start < end <= frame count` and at least two frames. Keys are timed from the first kept frame. |

A `frame_range` outside the file, an empty or one-frame range, or a bad
`scale` fails the import. The file's own timing is kept; there is no
resampling setting.

## Clip events

A clip owns its events. An imported clip starts with none, and a re-import of
its source keeps the events of the clip it replaces, read from the clip's
current `.gasset`: every `ClipEventKey`, `AnimationEvent`s and other event types
alike, at its saved time. An event past the re-imported clip's end, for
example after a shorter BVH `frame_range`, moves to the end, and the import
logs `'animation/<name>': moved <n> events past the clip's new end (<d> s) to
its end`. The clip is found by its output name, `animation/<name>`, after the
importer has sanitised the name and made it unique. An importer that emits
clips does this with `ImportContext::previous` and
`AnimationClip::keep_authored`.

A clip's `AnimationEvent`s are added, moved, renamed and removed in the
editor's clip tab (see `crates/animation-editor/README.md`), and Save writes
them into the clip's `.gasset` with `resave_content_asset`. Other event types
show there read-only and are saved as they are. Setting events over MCP is
#607. In code, `AnimationClip::add_event` or `replace_events` followed by
`resave_content_asset` does the same.

## Additive clips

A clip owns its additive base too: `Additive::LocalSpace` with the base clip,
its matched neutral take, the optional `base_range` and `range`, and the root
bones. An imported clip starts plain, `Additive::None`. A re-import keeps the
base of the clip it replaces, as for events:

- the base clip and `base_range` are kept as they are;
- `range` is clamped to the re-imported clip's duration, with the warning
  `'animation/<name>': clamped the additive range to <d> s`, and dropped, so
  the whole clip is used, when nothing of it is left, with the warning
  `'animation/<name>': dropped the additive range, which no longer overlaps
  the clip`;
- the root bones, whose translation the delta leaves out, are taken from the
  skeletons the source emits: every bone with no parent in one of them.

`base_range` and `range` are spans in seconds of the base clip and of this
clip; the whole clip is used when one is unset. Until #607 lands, the base is
set only in code: `AnimationClip::set_additive` with an `AdditiveBase`,
then `resave_content_asset`. Setting `Additive::None` the same way makes the
clip plain again.

The import doesn't read the base or check that it exists, so the clip and its
base import in any order. The delta between the two is
[derived data](#derived-data), built when the clip loads: a missing base, a
`range` or `base_range` that doesn't fit its clip, or any other failed derive
fails the clip's load, with an error naming the clip and the reason. A clip
added in code with `AssetServer::add` derives nothing, and has no delta.

## The `.gasset` file format

```
magic "GRDY" (4 bytes)
u32   header length, little-endian
      bincode(u32 format version, ContentAssetHeader)
      payload — the asset's own bincode encoding, verbatim
      derived blob — `derived.len` bytes, only when the header has a `derived` section
      embedded asset 0 — its payload, then its derived blob if it has one
      embedded asset 1 — …
      uncooked sections — the host's, then each embedded asset's in directory order, for those whose header has an `uncooked` section
```

A skeleton asset (`skeleton/N`) holds bone names, parent indices (each parent before its child), inverse bind poses, the rest pose as `JointPose` and the bone ids. `JointPose` lives in `concerto-mesh`; `concerto-animation` re-exports it. A skin whose joints are not ordered parents-first fails the import with an error naming the skin and joint; a skin with a joint outside every scene root is skipped with a warning. Neither emits a skeleton.

The format version is `CONTENT_FORMAT_VERSION`, currently `9`; it changes only when the framing or the header changes, never for a payload. The payload is everything between the header and the blob or the section; `read_content_asset` returns it alone, and `read_content_asset_parts` returns all three. `write_content_asset` writes neither a section nor a blob and refuses a header that describes one; `write_content_asset_with_uncooked` writes the header, the payload and the uncooked bytes it is given as a section, also when they are empty; `write_cooked_content_asset` writes the header, the payload and the blob of a `CookedSection`, and never an uncooked section. Committed content never has a derived section. Version 9 is the only version any reader accepts, `build.rs` and `asset_id!` included. A file at another version fails with `'<path>' is content format <n>; this build reads only 9. Import its source again, or re-create it if it was authored in the editor.`, or with `the file is …` where the reader has bytes and no path. glTF clips have no stored frame rate, so the clip editor shows a detected one; BVH clips store the rate from `Frame Time`.

`ContentAssetHeader` (see `concerto_foundation::assets::content`):

| Field            | Meaning                                                        |
|------------------|---------------------------------------------------------------|
| `asset_id`       | Persistent UUID, minted once and kept by every re-import, which finds the output by its provenance |
| `references`     | the `AssetId`s this asset points at, so a tree can be integrity-checked by scanning headers alone |
| `kind`           | the asset type tag (`Mesh`, `StandardMaterial`, `Scene`, …); must match the loading type |
| `kind_version`   | the `Asset::FORMAT_VERSION` of `kind` that the payload was written with |
| `provenance`     | `Some(ImportProvenance { source, sub_asset })` for import output, kept when `resave_content_asset` writes over it; `None` for editor-authored assets and for an imported asset saved with `save_content_asset_into` |
| `import`         | `Some(ImportRecord { importer, settings_version, settings, source, dependencies })` on an import's anchor: the importer's name, its settings as bincode at `settings_version`, and the source and each dependency it read as a `SourceFile { path, hash }`, the path relative to the project root (`../` for a dependency outside it) and the hash Fnv1a 64 of its bytes. Kept when `resave_content_asset` writes over the file; `None` on every other record, after `save_content_asset_into`, and in a cooked file |
| `uncooked`    | `Some(UncookedSection { len, version })` when the record has uncooked bytes at the end of the file: data a cooked build never needs, `len` bytes long, written at the `UncookedData::VERSION` of its type; `None` otherwise, and for a cooked file |
| `derived`        | `Some(DerivedSection { derived_version, key, len })` for a cooked file: the type's `DERIVED_VERSION` and `derived_key` at cook time and the blob's length in bytes; `None` otherwise |
| `embedded`       | the directory of the assets stored in this file after the host's payload and blob, one `EmbeddedAsset` each, in file order; empty for a loose file |

The header can be read without touching the payload, which is what lets a
future tool index a whole content tree cheaply.

One file can hold several assets. The asset the file is for is the
**host**; the others are **embedded** in it, and an asset alone in its file is
**loose**. An `EmbeddedAsset` holds what a loose file's header would hold for
that asset (`asset_id`, `references`, `kind`, `kind_version`, `provenance`,
`uncooked`, `derived`), but no `import`, since an embedded asset is never an
import's anchor, and the byte range of its payload: `offset`, counted
from the end of the header, and `len`. Its derived blob, of `derived.len`
bytes, follows its payload. Its uncooked section, of `uncooked.len` bytes, is
not in that range: every section of a file sits after the last embedded
asset, the host's first and then each embedded asset's in directory order, so
everything a load reads is one run of bytes from the start of the file. Its `provenance` is not optional, because `provenance.sub_asset` is
its name inside the file (`mesh/0`), which `EmbeddedAsset::name()` returns.
Embedded assets are stored in directory order, so the host's payload ends
where the first one starts, less the host's blob. An embedded asset lists no
embedded assets of its own.

`write_content_asset_with_embedded` writes a host from its header, with an
empty directory, its payload and one `EmbeddedPayload` per embedded asset, and
fills the directory. Each asset's uncooked section is the bytes in
`EmbeddedPayload::uncooked`, described by its header's `uncooked`,
which gives their version; `None` writes none. It refuses a
host header with a derived or an uncooked section, and an embedded asset
that has no provenance, lists embedded assets, has a blob its header does not
describe, has section bytes its header does not describe or describes with
another length, or repeats an id or a name.
`write_content_asset_with_uncooked(header, payload, version, bytes)` writes
a loose file with a section at `version`. `write_content_asset`,
`write_content_asset_with_uncooked` and `write_cooked_content_asset` refuse
a header that lists embedded assets.
`read_content_asset` and `read_content_asset_parts` of a host return the
host's payload, section and blob, with the directory in the header.
`asset_record` returns one asset of a file as a standalone record, the bytes a
loose file of it would have, its uncooked section included: the host for
`None`, an embedded asset by name. `ContentAssetHeader::record_header` gives
that record's header without reading a payload. Every reader rejects a
directory whose ranges overlap, are out of order or run past the file, or that
repeats an id or a name, with an error that names the asset, a file too
short for the sections its header describes, and a file with embedded assets
whose sections do not start where its last asset ends.

Three readers deal with uncooked sections alone.
`ContentAssetHeader::has_uncooked` says whether the host or any embedded
asset has one. `without_uncooked` returns a file's bytes with every section
removed: the file cut where the sections start, under a header that describes
none; a file with no section comes back byte for byte.
`read_content_without_uncooked` reads a file from disk as those same bytes
without reading the sections, by stopping where they start; a file it cannot
understand is read whole, so the parser that follows reports what is wrong
with it. `read_uncooked_section` reads the header and then only the section
of one record, the host for `None` or an embedded asset by name, as
`UncookedBytes { kind, version, bytes }`, the record's kind and the
section's own version; it returns `None` for a record
with no section and an error for a name the file does not hold.

## Uncooked data

An asset type can keep data in its file that a cooked build never needs, such
as the source an importer derived the asset from. It is the record's uncooked
section, described in the file format above: the editor and the importer read
it, an uncooked run reads it when a derive misses its cache, and a cook drops
it. The data is a type of its own
that implements `UncookedData` and names the asset type whose files carry it;
the asset type declares nothing:

```rust
#[derive(Asset, Serialize, Deserialize)]
#[asset(version = 2)]
struct Gear {
    teeth: u32,
}

#[derive(Serialize, Deserialize)]
struct GearDrawing {
    strokes: u32,
}

impl UncookedData for GearDrawing {
    type Asset = Gear;
}
```

`UncookedData::VERSION`, `1` by default, is the version of the data's own
encoding, stored with each section, so it changes apart from the asset's
`FORMAT_VERSION`. A file has one section per record; nothing stops two types
from naming the same asset, so keep to one: a reader that asks for another
type than the one written fails or decodes nonsense.

A section is optional for each record, so readers return an `Option`. A
record has one exactly when a writer was given one, also when the value
serializes to zero bytes.

The section is read on its own, through
`concerto_foundation::assets::content`:

- `read_uncooked::<E>(project_root, entry)` reads and decodes the section
  of one registry entry, a loose file or an embedded asset, without reading
  its payload. It returns `None` for a record with no section.
- `read_asset_record_without_uncooked(project_root, entry)` returns the
  same standalone record as `read_asset_record` with no section, and does not
  read the section from disk. `read_asset_record` returns the record with its
  section.
- `decode_uncooked::<E>(kind, version, bytes)` decodes a section's bytes,
  as `read_uncooked_section` returns them, and rejects a record whose
  `kind` is not `E::Asset`'s.

It is written by
`save_content_asset_with_uncooked(&value, &uncooked, project_root, address, &mut registry)`,
which saves as `save_content_asset_into` does and writes `uncooked` as the
file's section. The two save functions of
[Editor-authored assets](#editor-authored-assets) take no section and keep
the one of the file being replaced, copying its bytes and its version as they
are, so a save that does not hold the data cannot drop it. A section left by
a file of another kind, which only `save_content_asset_into` can replace, is
dropped. A section that cannot be read fails the save and leaves the file as
it was.

An importer writes a section with
`ImportContext::emit_with(name, &value, &uncooked)`, which takes a value
of an `UncookedData` type after its asset; `emit` writes none. The import writes
the section with the output, loose or embedded in the scene file. The `scene`
output cannot carry one, and emitting it with one fails the import.
`ImportContext::previous::<T>` reads the previous output without its section,
so a re-import never reads one from disk.

A section has a version of its own; see
[Changing an asset type's encoding](#changing-an-asset-types-encoding). A load
never hands a section to the asset; see
[Loading at runtime](#loading-at-runtime). A derive reads its asset's own
section on a cache miss; see [Derived data](#derived-data). A cook writes
every file without its sections; see [Cooking](#cooking).
`AssetServer::load_uncooked::<E>(id).await` reads and decodes the section
of the asset `id` through the server's root and registry, for code that has a
server and no project root: `None` when the record has none, and an error for
an id the registry does not hold.

## Changing an asset type's encoding

Each asset type has its own payload version, `Asset::FORMAT_VERSION`, which
starts at 1 and is written to `kind_version` in every header. Every type is
at version 1, its encoding as of content format 9, and none has a migration:
the versions restarted when format 9 stopped reading older files. Header scans
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

An asset's [uncooked data](#uncooked-data) is versioned apart from its
payload: a section is written under its type's `UncookedData::VERSION`, and
`decode_uncooked::<E>` decodes it by the same table, with
`E::migrate(version, bytes)` for an older one, which also rejects by default.
A change to the payload's encoding leaves sections alone, and a change to the
uncooked type's encoding raises only its own `VERSION`.

Each content type's encoding is pinned by a test. `check_encoding(&value, dir)`
(`concerto_foundation::assets::content`) serializes the value and compares the
bytes with `<dir>/<A::name()>.v<A::FORMAT_VERSION>.bin`. The checks sit in each
crate's serialization test, with the fixtures in its `tests/encoding/`. An
uncooked type is pinned the same way, with
`check_uncooked_encoding(&value, dir)`, whose fixture is
`<dir>/<E::Asset::name()>.uncooked.v<E::VERSION>.bin`:

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
   re-import their source or re-create them in the editor. An uncooked
   type whose encoding changes raises its `UncookedData::VERSION` and overrides
   `UncookedData::migrate` the same way; without it an older section is refused
   wherever it is decoded, while saves still keep it as it is.
3. Bless the new fixture with
   `CONCERTO_BLESS_ENCODING=1 cargo test -p <crate> --test <dir>`, where
   `<dir>` is the directory under the crate's `tests/` that holds its
   `main.rs`, run the test again without the variable, and commit the new
   `.bin` file, or the new `.uncooked` one for an uncooked type. Keep the old version's fixture while its `migrate` arm
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

`AnimationClip` stores `additive`, an `Additive`, as the payload's first field,
so `file_is_derived` reads it without decoding the clip: `None` for a plain
clip, or `LocalSpace` with the base clip's asset id, the optional `base_range`
and `range`, and the root bones. Like a graph's nodes, each event is stored as
its Rust type name and reflected fields. A clip makes its events live when it
loads; an event whose type is not registered with `register_clip_event`, or
whose stored value leaves a warning, is dropped with a warning naming the clip
and the type.

The editor reports outdated files when the first scene of a project opens.
It compares each entry's `kind_version` with the `FORMAT_VERSION` that the
scene's `AssetServer` records for that kind. That server is built from the
game's recipe, so it knows every type the game registers. For each kind and
file version that differs, it logs a warning with the count, and the status
line reads `N content assets are outdated; see the log`. It reports once per
opened project. A kind the game's recipe does not register is not reported;
such a file still fails when it is loaded.

`CONTENT_FORMAT_VERSION` is a different number. It changes only when the
framing or `ContentAssetHeader` changes, never for a payload. Version 9 reads
no older version. The version-9 framing is pinned by
`crates/foundation/tests/encoding/content_asset.v9.bin`, rewritten under
`CONCERTO_BLESS_ENCODING=1`.

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

A native load reads a file around its
[uncooked sections](#uncooked-data): it reads the header, then the file up
to where the sections start, so their bytes never leave the disk, and the load sees the
file as if it had none. An uncooked web build fetches each file whole,
sections included, and ignores them unless a derive needs one. A cooked build
has none to skip.

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
| `derive(&inputs, progress)` | Builds the blob, calling `progress(finished, total)`; it may read the asset's own uncooked data with `inputs.own_uncooked::<E>()` |
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

A type with [uncooked data](#uncooked-data) derives from it:
`inputs.own_uncooked::<E>()` decodes the asset's own section as `E` and
returns it. The section is read only after a cache miss, or after a
cached or cooked blob was refused, and only when the file holds one: a native
load reads the file's header and the section then, alone, and an uncooked
web load uses the bytes its fetch already holds. A cache hit and a cooked
blob read no section. `derived_key` is always called without it, so the key
comes from the payload alone, which therefore keeps whatever identifies the
section, such as a hash of it. `own_uncooked` fails when the file holds no
section, and in `derived_key`, with an error naming the type and saying that
a cooked file never holds uncooked data; so a cooked asset whose blob is
refused fails to load instead of deriving. A native load reads the file and
the section at two moments, so a type whose section must match its payload
checks that in `derive`. Code that derives in memory, with no file, hands the
value over itself:
`DerivedInputs::default().with_own_uncooked(&value)?`.

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
trailing derived section, so a shipped build applies it without deriving, and
writes every file that has [uncooked sections](#uncooked-data) or an import
record without them, so a shipped build holds none. It is
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
3. `AssetServer::cook` takes every file of the registry, in address order,
   and reads its header from disk. A file with no uncooked section, no import
   record and no asset whose kind is a registered derived type is skipped
   without being read. Any other is read as a load reads it, without its sections. For a
   loose or host asset of a derived type the cook asks the type's
   `file_is_derived` first; one that derives is
   decoded and its `on_load` runs; then it derives or takes a cache hit as a
   load does, reading its own uncooked section on a miss, and
   writes `<out>/<address>`: the original header without its import record,
   the payload bytes plus the derived section, and no uncooked section. A cached blob that `apply_derived` refuses is derived
   again, never cooked. An embedded asset that doesn't derive is left as it is;
   one that derives fails the cook, with an error giving its `<address>#<name>`
   and saying embedded assets cannot be cooked yet. No importer writes one.
   A file whose asset doesn't derive, or is of any other kind, is written to
   `<out>/<address>` when it has an uncooked section, on the host or on an
   embedded asset, or an import record, as the same file without its sections
   and with `import: None`; its provenance stays. With neither, no file is
   written. The `CookReport` lists the address of every file written, the
   command prints each as `cooked <address>` and then `cooked <n> files`, and
   the cook needs a content root that is a directory. The
   first failure stops the cook and fails the command, naming the file.
4. The cooked files are *staged*: each is copied over the file at the same
   path in what the build ships, `<exe dir>/<exe stem>-content/` natively,
   or `<dist>/` after `trunk build` for the web. A cooked file whose path the
   build doesn't ship is skipped.

The committed `content/` never holds a derived section, and `build.rs` and
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
in memory. It keeps the [uncooked section](#uncooked-data) of the file
being replaced when that file holds the same kind;
`save_content_asset_with_uncooked` writes one. No registry file is written. This API is separate from source
importing and writes no import record. The `project_root` is the
source tree under version control — deliberately *not* the exe-relative
runtime root, so a save is not overwritten by the next build.

A file written with `provenance: None` belongs to the editor, and no import
owns it: a re-import of the source it came from leaves it untouched and does
not prune it. When it sits at the address one of that import's outputs is
written to, the import fails before writing anything, naming the file, and
asks to move or rename it; see
[Finding an import's outputs](#finding-an-imports-outputs). To save an edit to
an imported asset and leave it owned by its source, `resave_content_asset`
writes over the existing file instead:

```rust
use concerto_foundation::assets::content::resave_content_asset;

let id = resave_content_asset(&material, project_root, "content/hero/material_0.gasset", &mut registry)?;
```

It fails when there is no file at the address or the file holds another kind,
keeps the header's id, provenance and import record and the file's uncooked section,
writes the references and
`kind_version` of `value`, and upserts the entry in `registry` with the same
provenance. The next re-import of the source then rebuilds the asset from the
source and overwrites the saved edit, except a clip's events and additive base,
which a re-import keeps (see [Clip events](#clip-events) and
[Additive clips](#additive-clips)).

Both refuse an address whose file holds embedded assets, and leave the file as
it was. Such a file belongs to an import, which writes it whole; nothing
rewrites one asset inside it. The error gives the address, says the file is
read-only, and says to extract the asset first with Extract to file or the
`extract_asset` MCP tool (see
[Extracting an embedded output](#extracting-an-embedded-output)), after which
`resave_content_asset` writes the loose file.

`read_asset_record(project_root, &entry)` reads one asset out of the tree
whatever way it is stored: the standalone record of a registry entry, loose,
host or embedded.

## Writing a new importer

An importer lives in whatever crate owns the asset types it produces and
implements `asset_import::Importer`:

```rust
use asset_import::{ImportContext, ImportError, Importer, NoSettings};

pub struct MyImporter;

impl Importer for MyImporter {
    type Settings = MySettings;
    type AssetSettings = NoSettings;

    fn name(&self) -> &'static str {
        "myimporter"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["myext"]
    }

    fn import(
        &self,
        source_path: &Path,
        settings: &MySettings,
        _asset_settings: Option<&NoSettings>,
        ctx: &mut ImportContext,
    ) -> Result<(), ImportError> {
        let doc = parse(source_path)?;

        // Emit each sub-asset by name. `emit` serializes the value, records
        // its resolved AssetId, and captures its `referenced_sub_assets()`.
        for (i, mesh) in doc.meshes().enumerate() {
            ctx.emit(&format!("mesh/{i}"), &mesh_to_engine_mesh(mesh))?;
        }

        // `emit_with` also writes the type's uncooked data beside the
        // value, such as the source the asset was derived from.
        ctx.emit_with("gear", &gear, &drawing)?;

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

`name` is what import records keep as `importer`, so it never changes once
content uses it. The importer must emit a `scene` or a `main` output, its
anchor. `Settings` are the importer's own, kept as bincode in the anchor's
[import record](#import-records); `AssetSettings` belong to the one asset a
source makes, such as an image's texture, and are kept in that asset, with
`current_asset_settings` reading them back from `ctx.previous`. Both derive
`Reflect`, `Default`, `Clone`, `Serialize` and `Deserialize`; an importer
without either uses `NoSettings`. Settings must be bincode-safe, as asset
payloads are: no `skip_serializing_if`, `flatten`, `untagged` or other serde
attribute that needs a self-describing format. Every field is reflected, with
no `#[reflect(skip)]`, so the Import dialog and the `import_source` tool reach
it. Changing how `Settings` encode raises `Importer::SETTINGS_VERSION`, which
starts at 1; a record written at another version is imported with the
defaults, with a warning.

Then add it to `IMPORTERS` in `crates/import/src/lib.rs`. `importers()` and
`importer_for(extension)` read that list, as `ErasedImporter`s, for the
import itself, the record's settings and the editor's file picker.

`ImportContext::emit<T: Asset>` requires the emitted type to implement
`Asset` (`Serialize + DeserializeOwned` + a `name()` tag). There is no
separate DTO trait — the on-disk form of an asset *is* the asset.
`ImportContext::emit_with<E: UncookedData>` takes an `&E::Asset` and an `&E`
and writes the second as the output's uncooked section; see
[Uncooked data](#uncooked-data).

## Current limitations

- **glTF `extras` are not read.** The Blender add-on that wrote game components
  into `extras` is deprecated. An import's scene is read-only, so place it in a
  scene of your own in the editor and add game components there.
- **A file saved from the editor at an output's address stops the import.**
  A file without provenance, saved with `save_content_asset_into`, at the
  address an output is written to, the scene included, fails the import before
  it writes anything, naming the file. Move or rename it and import again: the
  output takes a new id at its usual address and the saved file keeps its id.
- **No incremental import.** Re-import regenerates the source's outputs
  while preserving their identities. The import record holds the hash of the
  source and of every file `track_dependency` recorded, but nothing reads them
  yet to tell whether an import is stale (#880).
- **No cross-source references.** An importer that emits a reference to a
  sub-asset from a *different* source file logs a warning and leaves it
  unresolved (e.g. an OBJ material pointing at a standalone texture).
- **Quantised / morph-target animation panics.** `GltfImporter` hits
  `todo!()` on I8/U8/I16/U16 rotation channels and morph-target weight
  outputs — re-export uncompressed for now.
- **Sub-asset matching still uses importer keys** such as `mesh/0` and
  `animation/Idle_Loop`. Reordering objects in a DCC export can change what these keys
  mean. Matching across such changes is deferred.
- **Pruning skips editor-saved outputs.** Re-import deletes the `.gasset` of
  every output the importer stopped emitting, and removes it from the editor's
  registry. An embedded output is pruned by leaving it out of the rewritten
  scene file. An output saved from the editor with `save_content_asset_into`
  (its header has no provenance) is no longer the import's and is left
  untouched; one saved with `resave_content_asset` keeps its provenance and is
  pruned like any other output. Pruning an output that another asset still
  references logs a warning naming the output and the referrer.
- **An extracted output cannot be embedded again.** Extracting an embedded
  output makes it a loose file for good: a re-import keeps it loose. Deleting
  the file and re-importing embeds a new output with a new id.
- **A renamed output gets a new id.** Outputs are matched by sub-asset name
  alone, so an output whose name changes, such as a clip renamed in the
  source, is a new output with a new id, and the old one is pruned.
- **Moved content is seen at the next build.** A game finds a moved `.gasset`
  once it is rebuilt, and an open editor once the project is reopened; there
  is no runtime file watcher yet.
- **`ImageImporter` always imports as sRGB.** A per-entry colour-space
  hint would need to come from `content.toml` or a filename convention.
