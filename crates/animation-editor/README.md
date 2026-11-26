# concerto-animation-editor

The asset editors of an `AnimationClip`, of a `RetargetRig` and of an
`AnimationGraph`, and the clip transport widget the first two play clips with,
as an extension of `concerto-editor`.
Only editor crates depend on it. `cargo concerto new` writes this entry into a new
project. A game made without it adds it to its own `Cargo.toml`, and
`cargo concerto editor <dir> --update` writes the call into the game's generated
`editor/`:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-animation-editor", install = "concerto_animation_editor::install" }]
```

`install` adds four plugins: `TransportPlugin`, the transport widget below;
`AnimationClipEditorPlugin`, which registers `AnimationClipEditor` as an asset
editor and forwards what its preview reports; `RetargetRigEditorPlugin`,
which registers `RetargetRigEditor` as a creatable asset editor and registers
`RetargetRig`, `Skeleton` and `AnimationClip` in the editor's `AssetServer`, so a
rig tab can pick a rig and a clip, and a loaded rig can load its skeleton and
its reference clip; and `AnimationGraphEditorPlugin`, which registers
`AnimationGraphEditor` as a creatable asset editor, so New lists
`New AnimationGraph`, registers `BoneMask` in the editor's `AssetServer` and
names `AnimationGraph` there with `name_asset`, so a layer's mask and a state's
or a retarget node's graph can be picked while the editor never loads a graph
against its own type registry, names `Scene` the same way unless the editor
already registered it, so the `Preview` value's `scene` can be set, registers
the editor of a `Param<T>` field for
the engine's parameter types, and adds the graph tools when the editor serves
MCP. A
game that doesn't list it has no clip, rig or graph tab: a clip or a graph stays
selected in the content browser and opens nothing, and a rig opens in the
generic inspector. Listing
`concerto-motion-matching-editor` doesn't bring it, so a game that wants both
lists both.

`read_clip`, `read_shown_scene`, `ClipFacts`, and in `preview` the recipes
`clip_preview` and `clip_playback` and their signals and events, are public for
other editors that play a clip. `clip_playback` holds the clip, the time, the
rate, playing and looping, advances the time, writes `ClipPreviewProgress`, and
listens for `ShowClip`, `ClearClip`, `SetClipTransport` and `SeekClip`; it poses
nothing. `preview_stage` is the stage a preview plays on: the scene
`ShowClipScene` gives it, the bones, and the orbit camera following the first
rooted skeleton. It registers the `Skeleton` component gizmo with
`register_component_gizmo` and sets it to Always, so every skeleton's bones
show without a selection; `SetSkeletonGizmo(false)` sets it back to Selected,
which in a preview draws nothing. `clip_preview` calls both and adds
the posing: every rooted skeleton, root motion and `ClipPreviewBones`.
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
- the Skeleton checkbox, on by default, which draws the skeleton's bones: it
  sets the `Skeleton` gizmo to Always or back. A BVH scene has no mesh, so with
  it off the stage shows nothing;
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

## The graph tab

Opening an `AnimationGraph` opens a tab that shows the graph's nodes on a
[graph canvas](../editor/README.md#the-graph-canvas) and edits them: nodes are
added, wired, removed and inspected, and Save writes the graph back. A node
that plays another graph inside itself is drilled into in the same tab: a state
machine into a diagram of its states, a state or a retarget node into the graph
it plays. The tab's preview plays the graph on a character.

### The preview recipe

`graph::graph_preview` is the recipe of the tab's preview `App`: `preview_stage`,
with the `AnimationPlayer` the animation plugin puts on every rooted skeleton
playing a graph, and the root motion of the first rooted skeleton moving the
scene, so the character walks and the orbit camera follows. It listens for
three signals, sent with `PreviewSignals`, and writes one event:

- `ShowGraphs { graphs, play }`. Each of `graphs` is a `GraphBytes { id, bytes }`,
  a graph as a save writes it, made with `GraphBytes::of(id, &graph)`. The
  preview decodes each and stands it in for the asset `id` with
  `AssetServer::insert_loaded`, so a handle to that id, a state's or a retarget
  node's, plays the graph sent and not the file. A graph the preview was shown
  before and is not shown now goes back to its file, read again with
  `AssetServer::reload`. Every player then restarts on
  the graph `play`: the players are paused while the graphs are swapped, start
  again once nothing is loading, and a skeleton that appears later starts on
  the same graph.
- `SetGraphParam { name, type_name, value }` sets a parameter on every player's
  blackboard. `value` is the `ReflectedValue` of a value of the type
  `type_name`, which is built from the `Default` of the registered `Param<T>`
  that names that type; a type no registered `Param` names is logged and
  ignored. The preview keeps the parameters it was sent and gives them to a
  player that appears later.
- `SetGraphPlayback { playing, speed }` pauses the players or lets them play,
  and sets the preview's time scale. A graph has no duration, so there is no
  timeline and no step.
- `GraphPreviewState { nodes }` is the event, written on every frame the
  preview plays and on none while it is paused or its players are restarting.
  It is what `AnimationPlayer::for_each_node` visits on the first player that
  plays, the one with the lowest entity index: a `PreviewNode` per playing
  node with its `path` from the graph `ShowGraphs` named, its `weight` and its
  `report`, the clip time of a clip node and the current state and transition
  of a state machine. `nodes` is empty while no player plays the graph, as on
  a stage with no skeleton. The tab forwards it with `forward_hosted_event`,
  so `EventReader<Hosted<GraphPreviewState>>` reads it in the editor.

A node that reads the world, such as foot placement, a look-at target or motion
matching's trajectory, gets nothing from the preview's world and passes its
input through or holds. A `BlendNode` blends by node weights only game code
sets, so all its inputs play at 1.

### The preview

The preview plays the graph in view. While a state machine is in view it plays
the graph that holds the machine. Whenever a working copy changes, or what is in
view or the preview target does, the tab sends `ShowGraphs` with the working
copy of every graph it holds, the ones it has drilled into included, so an
unsaved edit is what plays and a state's or a retarget node's handle plays the
working copy of its graph; a nested graph the tab has not opened plays from its
file. Selecting a node sends nothing, so the players keep playing.

`Preview` is a value in the inspector, never saved, so editing it leaves the
tab clean, and it lasts while the tab is open:

- `scene`, the `Scene` the character comes from. Unset, it is the `Scene` named
  `scene` in the folder of the tab's graph, or an empty stage when there is
  none;
- `skeleton`, whether the skeleton's bones are drawn, on by default;
- `playing`, and `speed`, the preview's time scale, any finite value above 0;
- `from`, the node of the pose graph in view to preview from. Set, the tab
  builds a graph whose result node is fed by that node and everything that
  feeds it, sends it under an id of its own and plays that. A node that is not
  in the pose graph in view, or the result node, is refused: `from` goes back
  to none and the status says why. Going up or down a level, loading and
  Discard set it back to none.

The `left` host, titled Preview, holds the controls that write `Preview`: the
scene picker and the Skeleton checkbox the clip tab has, with the error of a
scene that failed to load under them, a Play checkbox, and the speeds 0.25×,
0.5×, 1× and 2×, the one equal to `speed` selected. Right-clicking a node of a
pose graph offers Preview from here, which sets `from`; the row over the canvas
then reads `Previewing node 3, a LayerNode` beside a Whole graph button that
sets it back.

### Live state

The tab keeps the last `GraphPreviewState` its preview wrote in `GraphLive`, on
the document, and shows the part of it that is in view on the canvas:

- a node of a pose graph has its weight as its badge, `1.00`, and a clip node
  whose clip the project has reads `Walk · 0.42s`, the clip's name and the
  time it plays at;
- in a state machine's diagram the current state is outlined with
  `CanvasEmphasis::Active`, and while a transition blends every arrow from its
  source state to its target state is drawn the same way.

A node is found in the state by the index it has in the graph last sent to the
preview, which `GraphCopy::build` and `build_from` answer beside the graph, so
a loose node, and with `from` set a node outside what plays, shows nothing.
The tab keeps those indices for what it has in view even when nothing is sent
again, as when `from` moves between two subtrees that build the same graph.
`LiveView` is that lookup: `LiveView::latest(live, sent)` and then
`node(id)`. Paused, the canvas keeps the last state. The state is dropped
whenever the tab sends `ShowGraphs`, since the nodes it names are those of the
graphs before, and the next frame the preview plays brings a new one. The
badges and subtitles are rewritten in place, so no box is rebuilt, and under
`--mcp` they do not keep the editor busy: the state arrives only on frames
that run for another reason.

### Parameters

A graph declares no parameters: the tab finds them. It walks the reflected
value of every node of the graph in view and of every graph those nodes play
inside themselves, through working copies where the tab holds one and through
the files otherwise, and collects each `Param<T>` that names something, inside
nested values, lists, options and enum variants, such as a transition's
trigger. A parameter is a name and the type its `Param<T>` reads it as.

`Parameters` is a value in the inspector with one field per parameter found,
each edited by the property editor its type has anywhere else, starting at the
type's `Default`. A value lasts while the tab is open, also while its parameter
is out of view, and is never saved. An edit is sent to the preview with
`SetGraphParam`, and every value is sent again after each `ShowGraphs`. A name
the graph reads as two types is two fields, `speed (f32)` and `speed (bool)`.
The players keep one value per name, so the one set last wins.

Under the controls, the `left` host's Parameters section lists each parameter
as `name · type · value`, a name read as several types in the warning colour
with `read as several types`, or `None` when the graph names none.

A `Param<T>` field of a node, such as a blend space's `input.param`, is edited
by `ParamNameEditor<T>`: a text field for the name and a button that lists the
names the tab has found for that `T`. Over MCP, `set_property` takes the name
as a JSON string. The plugin registers the editor for `bool`, `f32`, `u64`,
`Vec2`, `Vec3`, `IkTarget` and `IgnoredBody`; an editor crate registers it for a type of its
own with `app.register_param_editor::<T>()`, from `graph::ParamEditorAppExt`.
A `Param` of a type with no editor registered shows as a row with no editor
and is still written by `set_property`.

### The stack

The document keeps the chain of levels from the tab's own graph down to what is
in view, `GraphDocument::levels()`. A `GraphLevel` is a graph asset, shown as a
pose graph, or a state machine node of the graph above it, shown as a state
diagram. A diagram is not a graph of its own: it edits that node in the working
copy of the graph that holds it. The breadcrumb over the canvas shows the
chain, `Locomotion › States › Run (state)`: a graph by its asset's name, a
state machine as `States`, and a state's graph as `<state> (state)`. Each entry
above the last is a button that goes back up to it.

- Double-clicking a state machine node, or Enter with it selected, opens its
  diagram. Double-clicking any other node that plays a graph inside itself, a
  `RetargetNode`, opens that graph. Double-clicking a state opens its graph.
- A node or a state whose graph is not set is refused with
  `Node 2 names no graph; select it and set its graph`, as is one whose id is
  not an `AnimationGraph` of the project.
- A graph already in the stack is refused: a state that plays the graph that
  holds its machine cannot be opened inside itself.

Every graph the tab drills into gets a working copy, kept until the tab loads
another asset or its edits are discarded, so going up and back down shows the
same nodes under the same ids. An edit to a nested graph is an edit to that
asset: the tab is `Unsaved`, Save writes every graph that changed, and Discard
restores every one and goes back to the tab's own graph.

The editor keeps one tab per asset kind, so there is never a second graph tab:
opening another graph, one this tab has drilled into included, replaces what
the tab shows, after the unsaved-changes prompt when it has edits.

### The working copy

The tab never edits an `AnimationGraph` in place. Its document, `GraphDocument`,
holds a `GraphCopy`: every node of the graph under a `GraphNodeId`, each with its
inputs in slot order, loose nodes included. A node's id is the node's index in
the file when the graph is read and the next free number when it is added; ids
last while the graph stays open, are not saved, and are what the canvas, the
signals and the MCP tools name a node by.

The nodes are live, built with `AnimationGraph::make_live` from the types the
tab's preview `App` registers, which are the game's. So the tab waits for its
preview before it reads the file, and a game's own node types are listed,
built and inspected like the engine's. A node whose type the game does not
register, or whose stored value cannot be built, is a placeholder: it keeps its
stored value, a save writes it back as it was, the canvas outlines it in the
warning colour, and selecting it puts the reason in the status. If the preview
failed to build, the graph is shown read-only, with every node held as its
stored value, and every edit is refused with
`The preview is not running, so the graph is read-only`.

The copy tells a node when its inputs change: it calls the parent's
`EditorAnimationNode::input_added(index)` when it gives it an input and `input_removed(index)` when it
takes one away, so a `BlendSpace2DNode` keeps a point per input.

### The panels

The `bottom` host holds a row with the breadcrumb and, under it, the canvas, in
`Flow` mode for a pose graph. A node's title is the last segment of its type name. Its
subtitle is the name of the asset its first handle field names, when the
project has that asset. Its pins come from `input_slots()`: none, one per named
slot, or for a node that takes any number of inputs one per input and an empty
one after them. Every node but the result node has an output pin. When a graph
comes into view the canvas scrolls its result node into view, and when a state
machine does, its first state. A machine with no states is framed when it gets
its first.

- Pressing a node or a link selects it; a click on empty space clears the
  selection. A node or a state that becomes the selection, by a press,
  `graph_select` or being added, is scrolled into view.
- Dragging from a node's output pin to an input pin connects them.
- Delete removes the selected node or link.
- A right click on empty space lists the node types that can be added: every
  type registered with `#[reflect(AnimationNode)]` but `ResultNode` and
  `PlaceholderNode`, by short name, or by full path where two share one.
  Picking one adds a loose node.
- Dragging from a node's output pin and letting go on empty space lists the
  same types there; picking one adds that node and makes the dragged node its
  first input. A type that takes no inputs is refused and adds nothing.

For a state machine the canvas is in `Diagram` mode: a box per state, titled
with its name, its subtitle the name of the graph it plays or `no graph` in the
warning colour, and the initial state with the badge `initial`; an arrow per
transition, labelled with its trigger: `Instant`, `OnAnimationEnd`,
`<param> = <value>`, `<param> ≠ 0`, `on <message>` with the last segment of
the message's type path, `OnSucceeded` or `OnFailed`.

- A right click on empty space offers New state, which puts the keyboard in
  the name field beside the breadcrumb, shown only for a diagram. Enter there
  creates an `AnimationGraph` asset of that name in the folder of the graph
  that holds the machine, through `CreateAsset`, and adds a state of that name
  playing it. A name `CreateAsset` refuses, invalid or taken, or one the
  machine already has, adds no state and the status says why. The created file
  stays on disk if the tab's edits are later discarded.
- Dragging from one state to another adds a transition with an `Instant`
  trigger and no blend time, and selects it. The canvas sends nothing for a
  drag that ends on its own box, so a transition from a state to itself is made
  with Transition to self in the state's right-click menu.
- Delete removes the selected transition, or the selected state with every
  transition to or from it. The state's graph asset stays on disk.

The `inspector` host has the title, Graph, and the document's status on a line
of its own when it is not empty. Under it the inspector shows the selected
node as the document value `Node`, with the property editors its fields have
anywhere else. A selected state is the value `State { name, graph, initial }`
and a selected transition `Transition { trigger, blend_time }`. Neither is
stored: each is a `PropertyTarget` that reads the machine and writes an edit
into it with its edit methods before the edit is answered: a new `name` renames
the state, `graph` replaces the graph it plays, `initial` set to true makes it
the state the machine starts in, and a transition's fields replace its trigger
and blend time. An edit the machine refuses changes nothing and is refused with
the reason, in the status and in `set_property`'s reply, as
`Set State.name failed: the state machine already has a state named Idle`: a
name another state has, an empty name, and `initial` set to false, since a
machine always starts somewhere. A selected link, a placeholder, a state
machine node, or nothing selected shows no card for the selection: a state
machine's states and transitions are edited in its diagram, never as fields of
a `Node` value, and `graph_select` on one says to `graph_open` it. The `Preview` and `Parameters` cards follow
it.

### Editing

Every gesture triggers one signal, `EditGraph { document, edit }`, and the MCP
tools trigger the same one. `edit` is a `GraphEdit`:

- `AddNode { type_name, parent, slot }` adds a node of a type named by its
  short name or its full path, at its `Default` value, loose or connected as
  `Connect` connects, and selects it. A connection that is refused adds no
  node.
- `Connect { child, parent, slot }` makes `child` an input of `parent`. On a
  node with named slots, a `slot` that is taken replaces the input there, which
  becomes loose, and any other `slot`, or none, puts the child in the first
  empty slot; with every slot taken and no `slot` it is refused. On a node that
  takes any number of inputs, `slot` is the position to insert at, the end
  without one. A child that already has a parent is moved. It is refused when
  the child is the result node, when it would close a cycle, when the parent
  takes no inputs, and when `slot` is past a node's named slots.
- `RemoveNode(id)` removes a node and leaves its inputs loose; the result node
  is refused. `RemoveLink { child }` takes `child` out of its parent's inputs.
  Either way the later inputs of that parent move up a slot.
- `AddParent { type_name, child }` adds a node, makes `child` its first
  input, as dropping a link on empty space does, and selects the new node.
- `AddState { name, graph }` adds a state playing `graph`, or with none a graph
  asset it creates as New state does. `AddTransition { from, to }` adds an
  `Instant` transition with no blend time and selects it. `RemoveState(name)`
  and `RemoveTransition(transition)` remove one. A transition is a
  `TransitionRef { from, to, index }`, `index` counting the transitions between
  that pair in the order the machine tries them.
- `Open(target)` drills into `OpenTarget::Node(id)` or `OpenTarget::State(name)`
  and `Up(levels)` goes back up, stopping at the tab's own graph.
- `Select(selection)` selects a node, a link, a state, a transition or nothing.

Node and link edits are refused while a state machine is in view, and state and
transition edits while a pose graph is, each saying which tool gets there.

Each ends in a `GraphEdited { document, edit, result }` event: a `GraphChange`
naming the node added, the slot a connection took and the input it displaced,
the transition added or the graph a new state plays, or the reason for a
refusal, which is also the tab's status. An `AddState` that creates its graph
ends when the `AssetCreation` for it arrives. An edit that changes a graph marks
the tab `Unsaved`, as an edit of the `Node`, `State` or `Transition` value in
the inspector does. Selecting, moving through the stack, and editing `Preview`
or `Parameters` do not.

### Save and Discard

Save (Ctrl+S) builds a fresh `AnimationGraph` from each working copy that
changed, the result node first, then every node in id order, each connected to
its inputs in slot order, and writes it over its file, keeping the asset's id.
Loose nodes are saved; the runtime ignores what the result node does not reach.
The status then reads `Saved <address>, <address>`, naming each file written,
or `Save failed: …` with the tab left `Unsaved`. The save's `SceneSaved` lists
the same files as `written`, so `AssetsWritten` and the `save` tool's reply
name exactly what was written: a save after an edit to a nested graph alone
answers `Saved content/Walk.gasset.` and does not report the tab's own graph.

Discard drops every working copy and reads the tab's graph again, so the tab
shows it as last saved, with nothing selected, and a nested graph is read from
its file the next time it is opened. The preview is then shown the tab's graph
alone, so it plays every nested graph from its file too, as it does when the
tab opens another graph.

### MCP

All ten tools act on what the front graph tab has in view and answer
`refused`, saying to open a graph, on any other tab. A state is named by its
name, and a transition by `from`, `to` and `index`, its place among the
transitions between that pair, 0 without one.

| Tool | Does |
| --- | --- |
| `graph_inspect` | Read-only. The breadcrumb, then for a pose graph one line per node in id order, `id · type · place · subtitle`: `place` is `parent#slot`, `result` for the result node, or `loose`, and the subtitle is left out when the node has none. For a state machine, one line per state, `name · initial · graph address`, with `initial` only on the initial state and `no graph` or `missing graph` in place of an address, then one per transition, `from → to #index · trigger · blend`, such as `Idle → Run #0 · Instant · 0.25s`. When the graph names parameters, the line `parameters:` and one line per parameter follow, `name · type · value`, such as `moving · bool · false`, the type by the last segment of its name and the value as JSON, with ` · read as several types` after a name the graph reads as more than one. Once the preview has reported what it plays, the line `live:` follows: for a pose graph one line per playing node, `id · weight 1.00`, then ` · time 0.42s` on a clip node and ` · in Walk` on a state machine, with ` · Idle → Walk 0.25`, the transition and its progress, while one blends; for a state machine in view, that last part alone, `in Walk`. There is no `live:` block before a state has arrived or while no player plays the graph |
| `graph_add_node` | `type`, by short name or full path, and optionally `parent` and `slot`. Answers `Added node 4, a LayerNode, as input 0 of node 0.`, or `…, loose.` The new node is selected, so it is the value `Node`. With `child` in place of `parent` and `slot` it sends `AddParent`, as dropping that node's link on empty space does, and answers `Added node 5, a LayerNode, with node 3 as input 0.`; a type that takes no inputs answers `refused` and adds nothing, and `child` with `parent` or `slot` answers `invalid_arguments` |
| `graph_add_state` | `name`, and optionally `graph`, an existing `AnimationGraph` by address or id; without one a graph asset of that name is created beside the graph that holds the machine. Answers `Added state Idle, the initial state, playing content/Idle.gasset.` |
| `graph_connect` | In a pose graph `child`, `parent` and optionally `slot`; answers `Node 4 is input 0 of node 2; node 3, which was there, is loose.` In a state machine `from` and `to`, equal for a transition to itself; answers `Added transition Idle → Walk #0.` and selects it |
| `graph_remove` | One of `node`, `link` (the node the link makes an input), `state`, or a transition by `from`, `to` and `index`; several or none answers `invalid_arguments` |
| `graph_select` | `node`, `state`, or a transition by `from`, `to` and `index`, or nothing to clear the selection. What is selected is then the value `Node`, `State` or `Transition` of `status`'s `values:` line, which `inspect`, `set_property`, `list_push` and `list_remove` take as `target`. A state machine node is selected without a `Node` value, and the answer says to open its diagram with `graph_open` |
| `graph_open` | `node`, a state machine or a node that plays a graph, or `state`, a state of the machine in view. Answers `Showing Locomotion › States › Run (state).` |
| `graph_up` | Goes up one level and answers the same way; refused on the tab's own graph |
| `graph_set_param` | `name` and `value`, JSON of the parameter's type, and `type` when the graph reads the name as several types. Sets the parameter on the preview's players, as an edit of the `Parameters` value does, runs frames until the preview next reports what it plays, and answers `Set moving to true.` with the `live:` block of `graph_inspect` under it, so the reply shows the state the parameter led to; a transition that takes several frames to start shows on a later `graph_inspect`. While the preview is paused it answers at once, `Set moving to true. The preview is paused, so no live state follows; …`, and when no player plays the graph, as with no skeleton in the preview's scene, `Set moving to true. No preview is playing the graph, so no live state follows; …`. A preview that reports nothing within 30 s answers `timeout`. A name the graph does not read answers `refused` with `There is no parameter speed; the parameters are direction (Vec2), moving (bool).`, a name read as several types with no `type` answers `refused` with `moving is read as several types, bool and f32; give type.`, and a value that does not fit the type answers `refused` with the reason |
| `send_animation_message` | `message`, `{ "type": "<full type path>", "value": { ...fields } }`, and optionally `entity`, a character of the playing game by id as `scene_tree` shows it, such as `42v3`, which must carry the `AnimationPlayer`; the message then goes to that character through the game's `SendReflectedMessage` signal and the tool answers after two frames with `Sent Poke to the character. States: Poked.`; a game that is not playing answers `not_playing`, an id with no `AnimationPlayer` answers `no_player` saying to send to the entity that carries it. Without `entity` it sends the message to every character the preview plays, as the game's `SendAnimationMessage` does, and after the preview's second frame report answers `Sent Poke to the preview. States: Poked.` with one `node: <describe>` line for each node, other than a clip or a state machine, that describes itself, a slot's `playing count 0.50, …` among them. `graph_inspect`'s `live:` block prints the same line under such a node, indented and without the `node:` prefix. `type` is the type path as the refusal lists it, and the type must be registered with `register_animation_message` and reflect its `Default` (`#[reflect(Default)]`); an unknown or other type answers `refused` with the registered message types, a value that does not fit answers `refused` with the reason, and a paused preview answers `refused`. A trigger's `MessageType` path edited with `set_property` takes effect at once: every edit rebuilds the graph the preview plays, which resolves the type again |

The preview is driven through the value `Preview`: `set_property` with target
`Preview` and path `scene`, `skeleton`, `playing`, `speed` or `from`, the last
as `{"Some": {"0": 3}}` for node 3 and `"None"` for the whole graph. An
optional parameter of a node, such as a layer's `weight_param`, is set the same
way through the value `Node`, `{"Some": {"0": "fatigue"}}`, and cleared with
`"None"`; that needs the node's type to reflect its `Default`, with
`#[reflect(AnimationNode, Default)]`, as the engine's nodes with an optional
parameter do. The
parameters are also the value `Parameters`, one path per parameter. `inspect`
reads both back, and neither marks the tab unsaved.

An edit the tab refuses answers `refused` with the tab's status text.
`create_asset` with kind `AnimationGraph` makes an empty graph, `open_asset`
opens it, and `save` and `status` work as on any tab.
