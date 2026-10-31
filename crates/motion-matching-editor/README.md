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
not see a clip re-imported while it is open. Under `--mcp` the worker asks for
frames with `FrameDemand`, but `status` does not count it, so `busy: no` can come
before the rows settle.

The `bottom` host holds a `Timeline` of the selected entry, read from the same
`EntryFacts`. A drag on its track writes the entry's `TimeRange`, from
`time_of_frame` of the first and last frames, each capped at the clip's duration
because rounding can land above it and `bake` refuses that. A click clears the
range, so the entry uses the whole clip. A clip with no detected rate has no
frames, so its range cannot be selected.

The `inspector` host, above the normal inspector, shows the document's status on
a line of its own when it is not empty.

## Joint names

`JointNameEditor` makes a `JointName` a button naming the joint, on a field and
in a list, which opens a menu of the skeleton's bones. The bones reach the
preview as the `SkeletonBones` resource, replaced whenever the skeleton changes
and removed when there is none or it cannot be read. Without it the button is
disabled, and reads the stored name in the muted colour. A name the skeleton
lacks uses the warning colour, and `bake` reports it. An empty name reads `None`.
