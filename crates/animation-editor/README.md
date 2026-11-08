# concerto-animation-editor

The asset editors of an `AnimationClip` and of a `RetargetRig`, and the clip
transport widget they play clips with, as an extension of `concerto-editor`.
Only editor crates depend on it. `cargo concerto new` writes this entry into a new
project. A game made without it adds it to its own `Cargo.toml`, and
`cargo concerto editor <dir> --update` writes the call into the game's generated
`editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-animation-editor", install = "concerto_animation_editor::install" }]
```

`install` adds three plugins: `TransportPlugin`, the transport widget below;
`AnimationClipEditorPlugin`, which registers `AnimationClipEditor` as an asset
editor and forwards what its preview reports; and `RetargetRigEditorPlugin`,
which registers `RetargetRigEditor` as a creatable asset editor and registers
`RetargetRig` and `AnimationClip` in the editor's `AssetServer`, so a rig tab can
pick both. A game that doesn't list it has no clip or rig tab: a clip stays
selected in the content browser and opens nothing, and a rig opens in the
generic inspector. Listing
`concerto-motion-matching-editor` doesn't bring it, so a game that wants both
lists both.

`read_clip`, `read_shown_scene`, `ClipFacts`, and in `preview` the recipes
`clip_preview` and `clip_playback` and their signals and events, are public for
other editors that play a clip. `clip_playback` holds the clip, the time, the
rate, playing and looping, advances the time, writes `ClipPreviewProgress`, and
listens for `ShowClip`, `ClearClip`, `SetClipTransport` and `SeekClip`; it poses
nothing. `clip_preview` calls it and adds the posing: every rooted skeleton,
root motion, the scene, the skeleton gizmo and `ClipPreviewBones`.
`concerto-motion-matching-editor` uses them, with no need for this crate's
`install`. `events` holds `TabEvent`, the event as the tab edits it, with
`read_events` and `write_events` to convert a clip's `ClipEventKey`s.

## The transport widget

`transport` holds the transport row and timeline the clip tab uses, public for
any tab whose preview plays a clip with `clip_playback`:

- `ClipTransport`, a component on the document: `duration`, `rate` (the frame
  rate, if any), `time`, `playing`, `looping` and `speed`, the rate multiplier.
  Its owner sets `duration` and `rate`; the widget keeps the rest;
- `spawn_transport(parent, document, theme)` spawns step back, play/pause,
  step forward, Loop, and the speeds 0.25×, 0.5×, 1× and 2×, and returns the
  `TransportRow` of their entities. A step pauses, then moves one frame from the
  frame at the current time, and stops at either end. Play at the end of a clip
  that does not loop starts again from 0. The buttons send `SetClipTransport`
  and `SeekClip` to the document's preview;
- `TransportTimeline { document }` on a `Timeline` has the widget write the
  timeline's duration, rate and time from `ClipTransport`, and
  `scrub_transport(document)` is its scrub listener, which pauses and seeks. The
  owner keeps the timeline's markers and tracks;
- `TransportDisabled` on the document disables every button;
- `TransportPlugin` copies each preview's `ClipPreviewProgress` into its
  document's `ClipTransport`, and keeps the buttons in step with it: the play
  glyph, Loop, and the speed button equal to `speed` selected, or none when
  `speed` is another value; the steps disabled without a rate, and every button
  disabled with no duration or under `TransportDisabled`.

## The tab

Opening an `AnimationClip` opens a tab that plays it on a skeleton in a
preview, shows the bones it drives, and edits its `AnimationEvent`s. Any clip
can be edited, imported or authored. The tab reads the clip when it opens, so
re-importing a clip whose tab is open does not refresh the tab; closing and
reopening it does.

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
- the Events section, described below;
- the document's status on a line of its own when it is not empty, which covers
  a clip that failed to load, a refused edit, `Saved …`, `Save failed: …` and
  `Preview failed: …`.

None of the transport, rate, toggle, scene or scrub controls marks the tab
`Unsaved`; only an event edit does.

### The timeline

The `bottom` host holds a timeline. Scrubbing it pauses the clip, which stays
paused after release. It has no range selection: a drag on its track selects
no frames. Opening another clip reuses the document, so it resets the time and
starts playing, and keeps the scene pick, the rate, the toggles, the loop flag
and the shown skeleton.

Under the track, the marker lane shows one marker per event at its time: an
`AnimationEvent` by its name, any other event by the last `::` segment of its
type name. A typed event's marker is locked, in the muted colour, and does not
drag. Pressing a marker selects its event; clicking the track or the lane's
empty space clears the selection. Dragging an unlocked marker moves its event,
snapped to a frame when the clip has a rate. A refused move leaves the marker
where it was and puts the reason in the status.

Under the lane, one read-only row per bone the clip drives shows a tick at each
key, and keys closer than a few pixels join into a bar. A row's keys are the
clip's key times for that bone over all its channels, translation, rotation and
scale together. The rows are named from the skeleton the preview shows, the
first rooted skeleton of the shown scene: first each bone of that skeleton the
clip has channels for, in the skeleton's order, then each bone id the clip
targets that the skeleton lacks, in id order, labelled `?` and the id's first 8
hex digits, in the warning colour. With no skeleton shown, every row is an id.
The rows follow the shown skeleton: picking another scene renames them. The
rows scroll with the wheel when they outgrow the host.

### Events

The Events section edits the clip's `AnimationEvent`s:

- **Add event** adds an event named `event` at the head's frame and selects
  it. It is disabled while the clip has no duration.
- With an `AnimationEvent` selected, a **Name** field, a **Time** field and a
  **Remove** button. Time reads in frames when the clip has a rate, and in
  seconds with three decimals otherwise; it takes a whole frame, or seconds
  without a rate. Enter applies a field. Escape, a refused value, or leaving
  the field without Enter puts back the event's value.
- With a typed event selected, its full type name, read only, and Remove
  disabled. Typed events, any event that isn't an `AnimationEvent`, are
  read-only in the editor and a save keeps them as they are.
- With nothing selected, only Add event.

Delete removes the selected `AnimationEvent` while the clip tab is active, no
text field has focus and the unsaved-changes prompt is closed, even with the
pointer over the preview. The crate binds
it to `RemoveClipEvent` in its `ClipEventContext`.

An edit is refused, with the reason in the status, when it targets a typed
event, leaves an empty name, puts the time outside the clip, or puts two
`AnimationEvent`s with the same name on one frame (`` `footstep` is already at
frame 12 ``), since both would fire on every crossing. A time is snapped to a
frame and clamped to the clip's duration, so the last frame of a clip that ends
between frames is accepted.

### Save and Discard

An event edit marks the tab `Unsaved`. Save (Ctrl+S) reads the clip from its
`.gasset` again, checks every event's time against that clip's duration, and
writes the events, sorted by time, over it with `resave_content_asset`, which
keeps the asset's id and its import provenance. The tab keeps its own order of
the events. Re-reading means a re-import that landed while the tab was open
keeps its channels, with the tab's events on top. On success the status reads
`Saved <address>` and the preview plays the saved clip from the same time. A
save that fails, such as an event past the end of a clip that got shorter,
reads `Save failed: …` and leaves the tab `Unsaved`.

Discard puts back the events of the clip as last opened or saved, and clears
the selection. Opening another clip in a tab with unsaved edits raises the
editor's unsaved-changes prompt, as for any `Unsaved` document.

An imported clip stays owned by its source after a save: the next re-import of
that source rebuilds the clip and its events from the import settings'
`[settings.events]`, and the edits saved from the tab are lost. See Clip events
in `docs/asset-importing.md`.

The preview plays the clip's keys and never fires its events.

### What the widgets follow

The widgets follow what the preview reports, `ClipPreviewProgress` and
`ClipPreviewBones`, so the head moves a frame after the pointer, and it does not
move at all when the preview failed to build. `ClipPreviewBones` carries the
shown skeleton's bones, `ShownBone` ids and names in bone order, which name the
bone rows. A preview behind another tab is frozen, and the document keeps its
last report.

## The rig tab

Opening a `RetargetRig` opens a tab that keeps the generic inspector over the
rig, with its save, `Unsaved` state, Discard and bone picker, and adds a bone
tree, a second rig and a clip beside the open one, and a preview of both
characters side by side. The open rig is the target; `Preview.open_rig_is`
set to `Source` swaps the sides.

### The panels

The `left` host, titled Bones, holds a tree of the open rig's skeleton, nested
by parent and expanded. Each row shows the bone's name and its tags, each in
its colour: `root`, `motion root`, and `<chain> start` or `<chain> end` at a
chain's ends, in the chain's colour in the preview. With no skeleton it reads
`No skeleton`; a skeleton that can't be read says why. Right-clicking a row
opens a menu listing Root, Motion root, then `<chain> start` and
`<chain> end` for each chain. A pick writes the bone's name at `root`,
`chains.<i>.start` or `chains.<i>.end`, or `{"Some": {"0": "<bone>"}}` at
`motion_root`, through `WriteValue`, the same code as MCP `set_property` on
that path. It marks the tab `Unsaved` and the inspector refreshes. Selecting a
row highlights it and does nothing else.

The `inspector` host has, above the inspector's `RetargetRig` and `Preview`
cards:

- the title, Retarget;
- the transport, as in the clip tab. It is disabled in Reference mode or with
  no clip;
- the facts: Scale, the retarget's scale from source to target (`0.93×`);
  Chains, the target chains with a source match (`5 of 6 shared`); and Driven,
  the source bones the clip drives (`60 of 65`). A fact that can't be worked
  out reads `None`;
- the status on a line of its own when it is not empty.

The `bottom` host holds a timeline that follows the transport and pauses and
seeks when scrubbed. It has no markers and no rows.

### The preview

`Preview` is a second value in the inspector, never saved, so editing it
leaves the tab clean: `other`, the rig beside the open one; `clip`;
`open_rig_is`; `mode`, `Reference` to stand both characters in their reference
poses or `Clip` to play the clip on the source and retarget it onto the target;
`root_motion`; `bones`; `looping`; `speed`, any finite value above 0; and
`paused_at`, `Some(time)` while paused. The transport and `Preview` stay in
step: a pause from the transport sets `paused_at`, and setting it pauses and
seeks. Opening another rig in the tab keeps `Preview` and goes back to the
start of the clip.

Each side shows the `Scene` named `scene` in its skeleton's folder, or its
bones alone when there is none or the scene holds another skeleton. The sides
stand apart by their widths. The bones are drawn in their chain's colour, the
same on both sides for chains of one name, with the root and the motion root in
colours of their own. With root motion off each pelvis stays over its spot;
on, the source moves with the clip and the target by the retarget's scale. The
preview follows unsaved edits to the rig.

### The status

The status lists what stops the preview, joined with ` · `: `Source rig: …`
or `Target rig: …` for a rig that doesn't resolve, `Retarget: …` for two rigs
that can't be retargeted, `Pick the other rig`, `Pick a clip` in Clip mode,
and `The clip drives none of the source skeleton's bones`. A line written by
something else, such as `Saved …`, stays until the next edit to the rig or to
`Preview`.

### MCP

The tab needs no tool of its own. `open_asset`, `inspect`, `set_property`,
`list_push`, `list_remove` and `save` work on the `RetargetRig` value as on any
inspected asset, and `set_property` on `Preview` drives everything the tab's
controls do; `inspect` reads both back. `status` shows the status line and
`screenshot` shows the preview.
