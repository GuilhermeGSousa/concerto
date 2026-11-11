# Editor UI layout: plan

Spec: `.agents/specs/2026-11-11-editor-ui-layout-design.md`. Branch:
`claude/editor-ui-improvements-oxgtqz`.

## For every task

- Read `AGENTS.md`, `.agents/reference/gotchas.md` (Editor), `docs/testing.md`
  and the spec before touching a file. The spec's section numbers are used
  below.
- Gate, for the crates the task touched: `cargo fmt --all -- --check`,
  `cargo clippy --locked -p <crate>... --all-targets -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings`,
  `cargo test --locked -p <crate>...`. The full done gate runs once, after
  task 5.
- This machine has no display and no GPU adapter. GPU tests skip. What needs
  eyes goes under `NOT VERIFIED:`.
- Existing tests that pin what the task removes are deleted or moved to the
  new seam in the same commit, with one line each in the report.
- Each task updates the documents it makes false, in its own commit.
- Commit on the branch. Never push.

## Tasks

### 1. `UISplitter` in `concerto-ui`

Spec section 2, first paragraph. A public widget in `crates/ui`, registered by
the UI plugin as the slider is: the component with its axis, the
`UISplitterDragged { delta }` signal sent to the splitter entity on `UIDrag`,
a theme builder for its node, and the accent highlight while hovered or
dragged. It moves nothing. One widget test: a drag sends the movement along
its axis only.

### 2. The tiled shell

Spec sections 1, 2, 3 and 4.

- `Region` becomes `Brand`, `Stats`, `Left`, `Centre`, `Right`, `Bottom`,
  `Status`; `build_dock` builds the flex shell; panels move as the spec's
  table says; flat panels, no outer margin.
- `EditorLayout`, `LayoutPanel`, `ResizePanel` and its clamping listener, the
  four splitters, and `apply_layout` with the shrink order and the triggers
  the spec lists. All owned by `DockPlugin`.
- `workspace.rs`: hosts are parented to the new regions, `BOTTOM_HOST_HEIGHT`
  goes, a region with nothing visible collapses with its splitter, the Asset
  Browser fills the strip without a bottom host.
- The extension crates (`concerto-animation-editor`,
  `concerto-motion-matching-editor`) and the two example editors still build
  and their tests pass.
- Test: the clamp table. The MCP scenario comes in task 3.
- Docs: README layout paragraph, bottom host and hosts text, "Who has input";
  the gotchas input entry; the `bottom_host` doc comment.

### 3. MCP: layout in `status`, `set_panel_size`

Spec section 7. The `layout:` line with `hidden` and `fill`; the tool, which
triggers `ResizePanel` and answers once `ViewportShown` holds the new layout.
Test: the layout scenario in `crates/editor/tests/editor/mcp.rs`, without its
form-tab half, which task 4 adds. README Tools and Errors.

### 4. Forms in the centre, and the label column

Spec section 5. `AssetEditor::form`, `InspectedAssetEditor` returning `true`,
no `canvas` or `left` host for a form document, the centre hidden, the right
region filling, cards capped at 760 px and centred; `left hidden` and
`right fill` in `status`, and `set_panel_size` refused for them. The label
column at 38% of the card, 96 to 260 px, written in pixels per card. Test: the
form-tab half of the layout scenario. README Inspected assets and Custom
asset editors.

### 5. One icon set

Spec section 6. Confirm the glyphs exist in `fonts/Phosphor/Phosphor.ttf`
first. `AssetIcons` and `register_asset_icon`, read by the Asset Browser and
the tab strip; glyphs in the hierarchy; the two icon sizes and the colour
rule; `marks.rs` deleted; the animation and motion-matching editors register
their kinds. Test: the `AssetIcons` lookup table. README section for
`AssetIcons`.

### 6. Full gate and branch review (session)

The done gate from `AGENTS.md`, then a `branch-reviewer` pass against the
spec.
