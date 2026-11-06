# concerto-motion-matching-editor

The asset editor of a `MotionDatabase`, as an extension of
`concerto-editor`. Only editor crates depend on it. A game lists it in its own
`Cargo.toml`, and `cargo concerto editor <dir> --update` writes the call into
the game's generated `editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-motion-matching-editor", install = "concerto_motion_matching_editor::install" }]
```

`install` registers `MotionDatabaseEditor` as a creatable asset editor, so
**New ▸ New MotionDatabase** in the content browser makes a definition and opens
it, and it registers `JointNameEditor` for the inspector. It also registers
`AnimationClip` as a derived asset in the editor's `App`, so a derive refuses an
entry naming an additive clip even before any game `App` has added
`AnimationPlugin`.

Listing this crate doesn't bring the clip tab. A definition's preview still plays
clips, through `concerto-animation-editor`'s `clip_preview`, but a game that wants
to open a clip in a tab lists that crate as well; see its
[README](../animation-editor/README.md).

## The document

`DefinitionDocument`, in the editor's world, holds the whole definition: the
skeleton, the entries, a `DatabaseSettings` with the rate, the speed, the
simulation bone, the trajectory and the `mirror_axis`, `X` or `Z`, that
mirrored entries are mirrored across, and one `HeldChannel` per channel, a live
`FeatureChannel` under the type name a save writes back. It waits for the
preview, because a game's channel types are only in the preview's type registry.
The preview `App` holds only the stage, and nothing edits in it. A second
definition reuses the tab.

Every value is published in `InspectedValues`: `Settings`, then one per channel,
named by its type's short name and numbered from the second of a type
(`BoneVelocities`, `BoneVelocities 2`), so MCP reaches each by its `target`. A
channel's value finds its channel by a `ChannelId`, so a commit queued against a
removed channel changes nothing, and can't land on the one that took its place.
The inspector shows the one value `ShownValue` names, `Settings` at first. A
document opens with an empty `InspectedValues`, so a refused open or a failed
preview keeps the inspector under the `inspector` host, with no card.

Opening a definition is refused with a status when it holds a channel whose type
is not registered in the game, that is not a `FeatureChannel`, or whose stored
value does not fit its type, so that a save cannot drop or change it.

An edit through a card comes back as `Hosted<PropertyCommitted>`, and an `Ok`
marks the document `Unsaved`. `save_definitions` saves any document with
`SaveRequested`, in front or not, writing each channel under its stored type name,
so a save with no edit writes the same definition.
Discarding unsaved edits puts the document back as it was last read or saved.

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
  a clip that cannot be read `Can't read`. A **Mirrored** checkbox ends the line:
  it sets the entry's `mirrored`, so the derived database and its playback take
  the clip's poses through the mirror across `mirror_axis`, and it is disabled when the
  document is read-only. A click selects the entry, **+** opens
  the asset picker over every clip and appends the pick as a whole clip, and **−**
  removes the entry. The same clip can be listed twice, once mirrored, to give
  the database both sides of the motion.
- **Settings**: a row that picks `Settings`.
- **Channels**: a row per channel with its value's name. A click picks it, **−**
  removes it, and **+** opens a menu of the types in the game's type registry
  that are a `FeatureChannel`. A new channel goes last, at the value its type's
  `Default` gives it. Adding or removing a channel replaces `InspectedValues` and
  `ShownValue`, and removing the shown one goes back to `Settings`.

`ShownValue` is the only record of which row is picked. An edit in the inspector,
or any change in `left`, marks the document `Unsaved`. Picking an entry does not
touch the inspector. With a failed preview there are no live channels: the
inspector shows no card, `left` lists what the file holds, its controls are
disabled, only an entry can still be selected, and Save is refused with a status.

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

The `inspector` host, above the normal inspector, holds the stats of the derived
database, in a list that scrolls past 160 px, and the document's status on a line
of its own when it is not empty. The next edit clears the status.

## Deriving

The database a definition plays is derived data: nothing is baked into the
project, and there is no Build button. The document derives it on open, after
each save, and when an `AssetsWritten` names its skeleton or one of its clips,
as an import of that clip sends. Discarding unsaved edits derives again, which
finds the cached blob, so a database an edit stopped plays again. It derives what
was last opened or saved, so an
unsaved edit derives nothing and the preview plays the last derived database.
Nothing is derived while the preview is not running, with no skeleton or with no
entry.

A worker thread reads the inputs with `DerivedInputs::read` from the project's
content and the editor's `AssetRegistry`, and computes the key. The editor's
`AssetServer` holds the project's `derived-cache/`, the one a hosted game and a
native run of the game use, so a blob found under that key is applied as it is,
and a weight change, which the key leaves out, finds the blob it had. Otherwise
the worker derives the blob, writes it to the cache and applies it, while the
status reads `Deriving 2 of 6`: the entry being worked on, out of how many. A
cache hit leaves the status alone, except a `Deriving …` line left by a derive
it replaced, which it clears. An error shows as `Derive failed: …` and
stops the database, and so does a channel weight that is not finite and
positive, on a cache hit as on a miss. A derive writes its lines only over an
empty status, the line it started over, or its own last line: a line written
while it runs, such as `Save failed: …` or the error of a refused open, stays.

The document holds `BackgroundWork` while the worker runs, so under `--mcp`
`status` reads `busy: yes` until it ends. A result is dropped when the tab has
opened another definition since, or when another derive was asked for while it
ran; closing the tab or opening another definition lets the thread run to its
end, and its blob still reaches the cache. The stats come from the derived
database: the frames in all and per entry, the memory, and each channel's mean
and standard deviation. An entry shorter than the trajectory looks ahead gets a
warning.

## The preview

`database_preview` adds `clip_preview` to the game's recipe, so a definition's
preview has the same stage as a clip's. It shows the scene named `scene` in the
skeleton's folder, read again whenever the skeleton changes, with a
`SkeletonGizmo` on each rooted skeleton. A folder with no such scene leaves the
stage empty, and a scene that cannot be read says why under the skeleton picker.

The derive's worker also decodes what the database needs to play from the inputs
it read: the skeleton and the clips of the definition as last saved, which may no
longer be the ones an unsaved edit shows. They reach the preview as `Arc`s in a
`ShowDatabase` signal, and the editor sends `ClearClip` with it, so only one system
writes the bones. Then
`play_database` steps a frame index at the database's `rate`, looping the entry
last sought, or every frame in order until one is, and writes
`database.pose(frame, ..)` to every bone with a match by id in the skeleton's `bone_ids`. That pose holds each
parentless bone relative to the simulation bone, so it is placed by
`database.simulation(frame)` first: the character goes where the clip takes it,
the camera follows it, and the simulation bone, a yellow ring with an arrow along
its facing, is drawn under it with `DebugGizmos`. After an unsaved edit the last
derived database plays until the next save derives again.

Every channel also draws the frame's features, through its `preview` and
`preview_features`, so a game's channel can add its own shapes. The channels are
the derived database's, so after an unsaved edit they draw what was last saved.
Channels take colours in turn from a palette without the simulation bone's yellow.
Each is dimmed unless it is the one picked under **Channels**, which draws at full
colour, so with `Settings` picked they are all dimmed. `ShowChannel` tells the
preview which one, by its index among the derived channels, and only when the
definition's channel there has the type the derived one has, so an unsaved edit
never highlights the wrong channel.

With no derived database to play, because nothing was derived or the derive
failed, the preview plays the selected entry's whole clip through `clip_preview`,
read on a worker. Picking another skeleton stops a database at once. No clip plays
while a derive is asked for or running, so a definition that derives never
flashes a clip when it opens. That clip plays unmirrored, even for a mirrored
entry; once the database is derived, it plays the mirrored frames.

The `Timeline` is always the selected entry's. With a clip its head is the
preview's time and a scrub sends `SeekClip`. With a database, selecting an entry
moves the playback to the entry's first frame, a scrub sends `SeekDatabase` to the
nearest derived frame of that entry, and `DatabaseProgress` moves the head, so it
returns to the entry's first frame each time the entry loops. An entry whose clip
is not the one derived at its place, or whose Mirrored differs from the derived
entry's, is not sought, so the head stays where it is and
the playback goes on looping the entry it was in. The tab has no transport:
the preview always plays, and a scrub moves it without pausing.

## Joint names

`JointNameEditor` makes a `JointName` a button naming the joint, on a field and
in a list, which opens a menu of the skeleton's bones. The bones are on the
document, replaced whenever the skeleton changes and `None` when there is no
skeleton or it cannot be read, so the row reads the front document and needs no
edit target.
Without them the button is disabled, and reads the stored name in the muted
colour. A name the skeleton lacks uses the warning colour, and a derive fails naming it.
An empty name reads `None`. The menu closes when another tab comes to the front.
