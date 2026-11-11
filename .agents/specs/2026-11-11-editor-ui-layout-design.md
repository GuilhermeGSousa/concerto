# Editor UI: tiled layout, resizable panels, centre forms, one icon set

## Problem

The owner's four complaints about the editor's UI, and where each comes from:

1. **Panels cannot be resized.** `crates/editor/src/dock.rs:117-118` fixes the
   left column at 238 px (`RAIL`) and the right at 290 px (`SIDE`), and
   `crates/editor/src/workspace.rs:28` fixes an asset editor's bottom host at
   132 px. The rig tab and the scene tab put long names and wide values in
   those columns, and the inspector's label column is a fixed 72 px
   (`crates/editor/src/inspector/mod.rs:112`, issues #187 and #141).
2. **Panels cover the game.** `Region::Scene` is a full-window node and every
   other region is absolutely positioned over it (`dock.rs:153-209`). The
   viewport, and so the game while it plays, is drawn under the cards.
3. **Tabs without a preview waste the window.** The generic inspected-asset
   tab is "the inspector alone" (`crates/editor/README.md`, Inspected assets):
   its form sits in the 290 px right card and the centre shows an empty
   viewport.
4. **Icons are inconsistent and hard to read.** Two systems are in use.
   Buttons and tabs draw Phosphor glyphs (`crates/editor/src/fonts.rs`). The
   hierarchy and the Asset Browser draw 4 to 11 px geometric shapes
   (`crates/editor/src/marks.rs`), where a camera and a group differ by one
   pixel of size. Tabs pick their glyph in a third place, a `match` in
   `crates/editor/src/tabs.rs:229-232` that knows two kinds.

## Decisions taken with the owner

| Question | Answer |
|---|---|
| Layout model | Tiled, with splitters. Panels keep fixed places. No docking, no moving panels. |
| Panels while playing | They stay, beside the game. No maximise toggle. |
| Tabs without a preview | The form fills the centre. |
| Icons | Phosphor everywhere; the geometric marks go. |
| Remembering sizes | For the session only. No file is written. |
| Scope of sizes | One layout for the whole editor, not one per asset kind. |
| Asset Browser | A bottom strip across the full width. |
| MCP | `status` reports the sizes and a tool sets them. |

Issue #364 (open, close and arrange extra panels) stays open. This design
replaces the regions it describes, and does not answer its questions.

Bevy has no editor and `bevy_ui` has no splitter, so there is nothing to
compare with there. The layout below is the fixed-pane arrangement of Godot's
default editor, without its dock rearranging.

## Design

### 1. The tiled shell

```
+----------------------------------------------------------+
| mark  Concerto | tabs ...................| stats | – □ x |   top bar
+-----------+------------------------------+---------------+
|           |                              |               |
|   Left    |           Centre             |     Right     |
|           |                              |               |
+-----------+------------------------------+---------------+
|   Asset Browser            |   bottom host (if any)      |   bottom strip
+----------------------------------------------------------+
| status line (Chatter)                                    |
+----------------------------------------------------------+
```

`Region` in `dock.rs` becomes `Brand`, `Stats`, `Left`, `Centre`, `Right`,
`Bottom` and `Status`. `Scene`, `Rail`, `Side` and `Foot` go. The root is a
flex column: the top bar, a row of Left, Centre and Right, the bottom strip,
and the status line. Nothing is absolutely positioned over anything else, so
the Centre is exactly the space the panels leave.

| Panel | Today | After |
|---|---|---|
| Viewport | `Scene`, full window | `Centre` |
| Hierarchy | `Rail`, top | `Left` |
| Asset Browser | `Rail`, bottom | `Bottom` |
| Inspector | `Side` | `Right` |
| Chatter | `Foot`, between the cards | `Status`, full width |
| Brand and tabs, stats, window buttons | `Brand`, `Stats` | unchanged, in a top bar with a surface fill |

Panels are flat: no 18 px outer margin, no floating rounded cards. Each
region has the panel surface colour and a 1 px border to its neighbour. The
undecorated window's drag area stays the top bar, as today.

A region with no visible panel collapses to nothing, and so does the splitter
beside it. That is what gives a tab the whole width when it has nothing to
show on a side.

### 2. Splitters and the layout state

**`UISplitter`** is a new public widget in `concerto-ui`, with the slider and
the tree. It is a thin node between two siblings, 5 px of hit area drawing a
1 px line, that lights to the accent colour while hovered or dragged. It
carries its axis. A drag triggers `UISplitterDragged { delta }` on it, the
pointer's movement along the axis since the last event, in logical pixels.
The widget moves nothing itself; its owner decides what the drag means, as a
`Timeline`'s owner does.

The window crate has no cursor-icon API (`crates/window/src/window.rs` has
only `CursorMode`), so the pointer does not change to a resize arrow. That is
a follow-up issue, not part of this work.

**`EditorLayout`** is a resource in the editor's world:

```rust
pub struct EditorLayout {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub browser: f32,
}
```

`left` and `right` are widths, `bottom` is the strip's height, and `browser`
is the Asset Browser's width inside the strip while it shares it with a
bottom host. Defaults are 260, 320, 220 and 420. It lives for the session and
is the same on every tab.

**`ResizePanel { panel, size }`** is the one signal that changes it, with
`panel` one of `LayoutPanel::{Left, Right, Bottom, Browser}`. Its listener
clamps the size to the table below, for the window as it is now, and stores
the clamped size. The dock's four splitters translate their
`UISplitterDragged` into it; MCP triggers it directly.

`workspace.rs` keeps deciding what is visible, as `sync_workspace` does today,
and now also which regions are collapsed or filling. One system,
`apply_layout`, writes the sizes to the region nodes. It runs after
`sync_workspace` and does its work when any of these changed: `EditorLayout`,
the window's size, `ActiveEditor`, an `EditorHosts`, an `InspectedValues`.
`dock.rs` still names no document type.

`DockPlugin` owns `EditorLayout`, `ResizePanel`, the splitters and
`apply_layout`; the dock is the shell, and the editor does not run without
it.

The clamp:

| Panel | Minimum | Maximum |
|---|---|---|
| Left | 180 | what leaves the Centre 320 wide |
| Right | 220 | what leaves the Centre 320 wide |
| Bottom | 120 | what leaves the Centre 240 high |
| Browser | 200 | the strip's width less 240 |

When the window shrinks below what the stored sizes need, `apply_layout`
shows smaller sizes without changing the stored ones, which come back when
the window grows. Right gives way first, down to its minimum, then Left; the
bottom strip gives way alone on height. The minimums and the Centre's add up
to 720 px, so they always fit the window's own minimum, which stays
900 x 600.

What each panel is on each kind of tab:

| Tab | `left` | `right` | `bottom` | `browser` |
|---|---|---|---|---|
| Scene | sized | sized | sized | fills the strip |
| Preview with a bottom host (clip, rig) | sized | sized | sized | sized |
| Preview without one | sized | sized | sized | fills the strip |
| Form (section 5) | hidden | fills the row | sized | fills the strip |

A sized panel shows its stored size, and can be set. A hidden or filling
panel keeps its stored size untouched, cannot be set, and is reported as
`hidden` or `fill`.

### 3. Playing

Nothing new is built for play. The viewport is the Centre, the hosted game is
already told the viewport's size (`hosted::input::size_correction`), so it
renders at the Centre's size and no panel is over it. Dragging a splitter
while the game plays resizes the game as it goes. A press on a splitter is a
press off the picture, so it gives input to the editor, as any panel press
does.

### 4. Asset editor hosts

`EditorHosts` keeps its four fields. Where each lands:

| Host | After |
|---|---|
| `left` | `Left`, in place of the hierarchy |
| `inspector` | `Right`, above the inspector panel, as today |
| `bottom` | `Bottom`, to the right of the Asset Browser, with the `Browser` splitter between them |
| `canvas` | `Centre`, for an editor with no preview |

`BOTTOM_HOST_HEIGHT` goes: the bottom host is as high as the strip. The
Asset Browser takes the whole strip on a tab with no bottom host. The clip,
rig and motion-database tabs need no change beyond what their hosts are
parented to, and gain the splitters.

### 5. Forms in the centre

`AssetEditor` gains one method:

```rust
/// Whether this editor's tabs show their inspected values as the tab's main content.
fn form(&self) -> bool {
    false
}
```

`InspectedAssetEditor` returns `true`. On a tab whose editor says `form` and
has no preview (a preview wins):

- the document gets no `canvas` and no `left` host, only `inspector`, and
  `bottom` if it asks for one;
- the `Centre` is hidden, and the `Left` region collapses, since the
  hierarchy is hidden on every tab but a scene's and nothing else is there;
- the `Right` region grows to fill the row, and its splitter is hidden;
- the inspector's cards are laid out in a column at most 760 px wide, centred.

The inspector panel is not reparented, so its scroll position survives a tab
switch as it does today. `EditorLayout.right` is not
changed by any of this, and the scene tab finds its width as it left it.

The label column stops being 72 px: it becomes 38% of the card's width, no
less than 96 px and no more than 260 px. The inspector computes it once per
card from the card's laid-out width and writes it in pixels to that card's
labels, so every row's value starts at the same place.
`standard_row_node` keeps its signature. That holds on every tab, so a
widened right panel on the scene or rig tab also gives labels room. It
is meant to close #187, to be confirmed at the running editor, and gives #141 the space it asks for once the panel is dragged
wider.

A custom editor that calls `InspectedAssetEditor::build` from its own `build`
is its own `AssetEditor`, so it gets `false` and today's hosts unless it
returns `true` itself.

### 6. One icon set

`marks.rs` is deleted. Everything that drew a `Mark` draws a Phosphor glyph
through `fonts::icon`.

- **One grid.** A row icon is a 16 px glyph in a 20 px column. An icon button
  is a 16 px glyph in a square of `theme.control_height`. Tabs, tree rows,
  inspector buttons, the transport and the window buttons all use these two.
- **One colour rule.** Muted text colour at rest, text colour when its row is
  hovered or selected, the warning colour only for a warning. The secondary
  accent is no longer used to tell kinds apart; the glyph does that.
- **Asset kinds.** A new `AssetIcons` resource, inserted by `FontsPlugin`,
  maps an asset kind's `Asset::name()` to a glyph. The Asset Browser's rows and the tab strip both read it, so they
  cannot disagree, and the `match` in `tabs.rs` goes. The editor registers
  Scene (`stack`), Mesh (`cube`), Texture (`image`), StandardMaterial
  (`circles-three`) and Skeleton (`bone`); any other kind shows `file`. An
  extension crate registers its own with
  `app.register_asset_icon("AnimationClip", glyph::WAVEFORM)`, which
  `concerto-animation-editor` and `concerto-motion-matching-editor` do for
  their kinds. Folders show `folder`, and `folder-open` while expanded.
- **Entities.** The hierarchy draws glyphs in place of shapes, and the scene
  root, which shares the group's shape today, gets its own: the scene root `stack`, a camera `camera`, a light `lightbulb`, a
  mesh `cube`, an entity with children and none of those `folder-simple`, and
  anything else `dot`. Icons per component type, registered by a game or an
  extension, are not part of this work.
New glyph constants are added to `fonts::glyph` under their upstream names.
Step 5 starts by confirming the bundled `Phosphor.ttf` holds `bone`, `folder`,
`folder-open` and `folder-simple`; a missing one is replaced by the nearest
glyph the font has.

Tooltips on icon-only buttons are not part of this work. `UITooltip`
(`crates/ui/src/widgets.rs:30`) only shows a node its owner placed, holds no
text, and nothing in the editor uses it; a tooltip inside a region would be
clipped. That is a follow-up issue.

### 7. MCP

- `status` gains a line: `layout: left 260 · right 320 · bottom 220 ·
  browser fill`, the sizes shown, in logical pixels, with `hidden` and `fill`
  as the table in section 2 says. Its `viewport:` line already reports the
  Centre's picture, in physical pixels; under `--mcp` the scale is 1, so the
  two agree.
- `set_panel_size { panel, size }` triggers `ResizePanel`, with `panel` one of
  `left`, `right`, `bottom` and `browser` and `size` in logical pixels. It
  answers the new layout line and the `viewport:` line once `ViewportShown`
  holds the new layout, which is two frames later, or the layout line alone
  on a tab that shows no viewport. A size outside the clamp is clamped and
  the answer says the size that was set. A panel that is hidden or filling on
  the front tab answers `refused`, saying which tab sizes it.

The tool is registered by the editor's MCP plugin, beside `status`.

The tool and the splitter trigger the same signal, so they cannot drift.

## Out of scope

- Opening, closing, moving, tabbing or undocking panels (#364).
- A maximise-viewport toggle, and hiding panels on Play.
- Saving the layout to a file, and a layout per asset kind.
- A resize cursor on splitters; a double-click that resets a splitter.
- Tooltips on icon-only buttons.
- Icons registered per component type.
- New colours or type sizes in `UITheme`.

## Tests

Following `docs/testing.md`:

- **Layout scenario, over MCP** (`crates/editor/tests/editor/mcp.rs`): open a
  project, read `status`'s layout line, `set_panel_size` each panel, and
  assert the reply, the next `status`, and that the viewport's size changed by
  the same amount. Open an inspected asset and assert the line reads `left
  hidden` and `right fill`, and that `set_panel_size` for each is refused;
  return to the scene and assert its sizes are back.
- **Clamps**: one table-driven test of the listener's clamp, over window
  sizes and requested sizes.
- **`UISplitter`**: one widget test in `concerto-ui` that a drag sends
  `UISplitterDragged` with the movement along its axis only. Drag is one of
  the things widget tests are for.
- **Asset icons**: one table-driven test of the public `AssetIcons` lookup:
  a registered kind, an unregistered one, a kind registered twice. What the
  rows and tabs draw is the owner's visual check.
- Existing tests that assert on `Mark` shapes, on `Region::Rail`/`Side`/`Foot`
  or on the 132 px bottom host are removed or moved to the new seam.

Checks for the owner at the running editor, since a cloud session has no
display: each splitter drags and stops at its limits; the game's picture is
never under a panel while playing and resizes with a drag; an inspected asset
tab shows its form in the centre; the rig and clip tabs lay out with their
hosts and timeline; every icon is legible at 100% and 200% scale; long
inspector labels such as "Shadowmaps enabled" fit at the default width.

## Documentation

`crates/editor/README.md`: the layout paragraph of "How the crate is laid
out", the bottom host and hosts text in "Custom asset editors" and
"Previews", "Inspected assets" for `form`, a new short section for
`EditorLayout`, `ResizePanel` and `AssetIcons`, and `status` and
`set_panel_size` under Tools, and "Who has input", which speaks of panels
drawn over the viewport. `.agents/reference/gotchas.md`, whose input entry
says the same. The doc comment on `AssetEditor::bottom_host`.

## Order of work

Each step leaves the editor working and is one task of the plan.

1. `UISplitter` in `concerto-ui`.
2. The tiled shell: new regions, panels moved, `EditorLayout`, `ResizePanel`,
   splitters, hosts reparented. This step alone fixes complaints 1 and 2.
3. `status` layout line and `set_panel_size`.
4. `AssetEditor::form`, the centre form, and the proportional label column.
5. `AssetIcons`, glyphs in the hierarchy, Asset Browser and tabs, `marks.rs`
   removed.

Each step updates the documents it makes false, in its own commit.
