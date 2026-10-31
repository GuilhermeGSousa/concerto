# concerto-motion-matching-editor

The asset editor of a `MotionDatabaseDefinition`, as an extension of
`concerto-editor`. Only editor crates depend on it. A game lists it in its own
`Cargo.toml`, and `cargo concerto editor <dir> --update` writes the call into
the game's generated `editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-motion-matching-editor", install = "concerto_motion_matching_editor::install" }]
```

`install` registers `MotionDatabaseEditor` as a creatable asset editor, so
**New ▸ New MotionDatabaseDefinition** in the content browser makes a definition
and opens it, and it registers `JointNameEditor` for the inspector.

## The document

`DefinitionDocument`, in the editor's world, holds the skeleton and the entries.
The rate, the speed, the simulation bone, the trajectory and the channels live
in the preview's world, on one `DatabaseSettings` entity and one entity per
channel, which carries the channel's own type as a component with `ChannelOrder`
and `ChannelKind`. The preview `App` is the edit target, so the normal inspector
edits them. Nothing else may add, remove or rename those entities: the preview
holds `FixedEntities`, and the channel list below sends `AddChannel` and
`RemoveChannel` to it. A second definition reuses the tab.

Opening a definition is refused with a status when it holds a channel whose type
is not registered in the game, or that is not a component and a `FeatureChannel`,
so that a save cannot drop it.

## The panels

The `left` host scrolls and holds:

- **Skeleton**: the asset picker, listing `Skeleton` assets, reading `None` with
  no skeleton and `Missing` for one gone from the project. A skeleton that cannot
  be read says why under the picker.
- **Entries**: a row per entry with its clip's name, its range in seconds or
  `Whole clip`, and Driven, `60 of 65`: 60 of the skeleton's 65 bones have a
  channel in the clip. It uses the warning colour when that differs from the
  clip's bones, so the clip names bones this skeleton does not have. Before the
  clip has been read it reads `Reading…`, with no skeleton `No skeleton`, and for
  a clip that cannot be read `Can't read`. A click selects the entry, **+** opens
  the asset picker over every clip and appends the pick as a whole clip, and **−**
  removes the entry. The same clip can be listed twice.
- **Settings**: a row that selects the `DatabaseSettings` entity.
- **Channels**: a row per channel with its type's short name. A click selects
  the entity, **−** removes it, and **+** opens a menu of the types in the game's
  type registry that are both a component and a `FeatureChannel`. A new channel
  goes last, at the value its type's `Default` gives it. The type does not
  declare `ReflectDefault`, so channels stay out of every scene's Add Component
  menu.

`Selection`, in the preview, is the only record of which row is selected, and
the inspector shows that entity. An edit there, or any change in `left`, marks
the document `Unsaved`. With a failed preview there is no world to edit: `left`
lists what the file holds, its controls are disabled, only an entry can still be
selected, and Save is refused with a status.

Clips are read off the main thread: a worker reads each clip that has no
`EntryFacts` against the current skeleton, on open, when an entry is added and
when the skeleton changes. Opening a definition reads them again, so a tab does
not see a clip re-imported while it is open. The document holds `BackgroundWork`
while any of its workers runs, so under `--mcp` frames keep running and `status`
reads `busy: yes` until the rows settle.

The `bottom` host holds a `Timeline` of the selected entry, read from the same
`EntryFacts`. Its head follows the preview, as described below. A drag on its
track writes the entry's `TimeRange`, from `time_of_frame` of the first and last
frames, each capped at the clip's duration because rounding can land above it and
`bake` refuses that. A click clears the range, so the entry uses the whole clip.
A clip with no detected rate has no frames, so its range cannot be selected.

The `inspector` host, above the normal inspector, holds **Build**, the line under
it, the baked state, the stats of the baked file, in a list that scrolls past
160 px, and the document's status on a line of its own when it is not empty. The
next edit clears the status.

## Build

Build is off, with the reason on the line below it, while the preview is not
running, with no skeleton, and with no entry. While it runs it is off and that
line reads `Baking 2 of 6`: the entry being worked on, out of how many.

Pressing it waits until no property or component edit is queued, then saves if
the document, or the preview's `SceneDirty`, says anything changed, so a weight
typed and not yet committed is in the build. A failed save stops it there. A
worker thread then reads the skeleton and clips, using the preview's
`AppTypeRegistry`, so a channel the game registers bakes, and hashes them with
the definition. When `<name>_baked.gasset` next to the definition holds that hash,
only the weights changed: the worker loads the file and calls `set_weights`, and
the status reads `Updated the weights in …`. Otherwise it bakes in full, and the
status reads `Built …`. A `BakeError` shows as `Build failed: …`.

The main thread writes the file, its id derived from its path, and adds it to the
registry, only if the document still shows the definition the build started for.
Closing the tab or opening another definition drops the result, while the thread
runs to its end. The baked file is an ordinary `.gasset`; the project decides
whether to commit it. Build refuses when that address holds another kind of
asset, or a file whose header it can't read, which includes a baked file from an
older format. It leaves that file alone, and it has to be removed by hand.

The baked state compares that file with the definition as last saved. A worker
works it out on open, after each save and after each build, so a changed weight
reads `Out of date` once saved even though the hash leaves it out. A definition
with no baked file, or one in an older layout, reads `Not built`. A skeleton or
clip that cannot be read reads `Can't read <asset>`. With a failed preview the
channels cannot be built, and the line stays empty. The stats come from the baked
file: the frames in all and per entry, the memory, and each channel's mean and
standard deviation. An entry shorter than the trajectory looks ahead gets a
warning.

## The preview

`database_preview` adds `clip_preview` to the game's recipe, so a definition's
preview has the same stage as a clip's. It shows the scene named `scene` in the
skeleton's folder, read again whenever the skeleton changes, with a
`SkeletonGizmo` on each rooted skeleton. A folder with no such scene leaves the
stage empty, and a scene that cannot be read says why under the skeleton picker.

The worker that works out the baked state also loads what the baked file needs to
play: the database, its skeleton and the clips it names, which may no longer be
the definition's. They reach the preview as `Arc`s in a `ShowDatabase` signal, and
the editor sends `ClearClip` with it, so only one system writes the bones. Then
`play_database` steps a frame index at the database's `rate`, through all the
frames in order and back to the first, and writes `database.pose(frame, ..)` to
every bone with a match by id in the skeleton's `bone_ids`. That pose holds each
parentless bone relative to the simulation bone, so it is placed by
`database.simulation(frame)` first: the character goes where the clip takes it,
the camera follows it, and the simulation bone, a yellow ring with an arrow along
its facing, is drawn under it with `DebugGizmos`. An out-of-date file plays until
the next build, and the baked state says so.

With no baked data to play, the preview plays the selected entry's whole clip
through `clip_preview`, read on a worker, and that is also the fallback for a
baked file made for another skeleton, one whose clips cannot be read, or one the
skeleton no longer fits. Picking another skeleton stops a database at once. Nothing
plays until the baked state is known, so a definition with baked data never
flashes a clip when it opens.

The `Timeline` is always the selected entry's. With a clip its head is the
preview's time and a scrub sends `SeekClip`. With a database, selecting an entry
moves the playback to the entry's first frame, a scrub sends `SeekDatabase` to the
nearest baked frame of that entry, and `DatabaseProgress` moves the head while the
playback is inside the entry; once it moves on, the head waits at the entry's last
frame until the playback comes round. An entry whose clip is not the one baked at
its place is not sought, so the head stays where it is. The tab has no transport:
the preview always plays, and a scrub moves it without pausing.

## Joint names

`JointNameEditor` makes a `JointName` a button naming the joint, on a field and
in a list, which opens a menu of the skeleton's bones. The bones reach the
preview as the `SkeletonBones` resource, replaced whenever the skeleton changes
and removed when there is none or it cannot be read. Without it the button is
disabled, and reads the stored name in the muted colour. A name the skeleton
lacks uses the warning colour, and `bake` reports it. An empty name reads `None`.
