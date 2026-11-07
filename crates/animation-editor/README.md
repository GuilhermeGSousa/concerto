# concerto-animation-editor

The asset editor of an `AnimationClip`, as an extension of `concerto-editor`.
Only editor crates depend on it. `cargo concerto new` writes this entry into a new
project. A game made without it adds it to its own `Cargo.toml`, and
`cargo concerto editor <dir> --update` writes the call into the game's generated
`editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-animation-editor", install = "concerto_animation_editor::install" }]
```

`install` registers `AnimationClipEditor` as an asset editor and forwards what
its preview reports. A game that doesn't list it has no clip tab: a clip stays
selected in the content browser and opens nothing. Listing
`concerto-motion-matching-editor` doesn't bring it, so a game that wants both
lists both.

`read_clip`, `read_shown_scene`, `ClipFacts`, and in `preview` the recipe
`clip_preview` and its signals and events, are public for other editors that
play a clip. `concerto-motion-matching-editor` uses them, with no need for this
crate's `install`.

## The tab

Opening an `AnimationClip` opens a read-only tab that plays it on a skeleton in
a preview. The editor never changes a clip: a clip tab is never `Unsaved`, and
Ctrl+S does nothing on it. The tab reads the clip when it opens, so re-importing
a clip whose tab is open does not refresh the tab; closing and reopening it
does.

The preview shows the `Scene` named `scene` in the clip's folder, or an empty
stage when there is none. The tab's `left` host, titled Preview, has:

- the scene picker, showing the shown scene's name or `None`. It lists the
  project's Scene assets, and picking one shows it for the rest of the tab's
  life, whichever clip is open in it. The pick is not saved;
- the Skeleton checkbox, which draws the skeleton's bones. A BVH scene has no
  mesh, so with it off the stage shows nothing;
- under them, the error of a scene that failed to load. The clip still plays,
  on an empty stage.

The `inspector` host, titled Clip, has:

- the transport: step back, play/pause, step forward, Loop. A step pauses, then
  moves one frame from the frame at the current time, and stops at either end
  of the clip. Play at the end of a clip that does not loop starts again from
  0. Without a frame rate the step buttons are disabled, and a clip with no
  duration disables the whole transport;
- the rate, 0.25×, 0.5×, 1× or 2×, with the active one selected;
- the Root motion checkbox, off by default. On, the root bone's movement moves
  the character and the orbit camera follows. Off, the character stays over one
  spot and keeps its height;
- the facts: Duration, Frames, Rate, Clip bones and Driven. Rate is detected from
  the keys, so it reads `30 fps, detected`. Driven reads `60 of 65`, 60 of the
  skeleton's 65 bones, and uses the warning colour when it differs from Clip
  bones: the clip then names bones this skeleton does not have;
- the document's status on a line of its own when it is not empty, which covers
  a clip that failed to load and `Preview failed: …`.

The `bottom` host holds a timeline. Scrubbing it pauses the clip, which stays
paused after release. Its track selects no frames. Opening another clip reuses
the document, so it resets the time and starts playing, and keeps the scene
pick, the rate, the toggles and the loop flag.

The widgets follow what the preview reports, `ClipPreviewProgress` and
`ClipPreviewBones`, so the head moves a frame after the pointer, and it does not
move at all when the preview failed to build. A preview behind another tab is
frozen, and the document keeps its last report.
