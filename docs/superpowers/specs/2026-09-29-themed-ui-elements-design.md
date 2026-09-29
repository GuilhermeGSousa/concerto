# Themed UI Elements — Design

**Status:** approved design, not yet implemented
**Touches:** `crates/ui` (new `elements` module, `theme`, `node`, `widgets`, `text`,
`checkbox`), `crates/editor` (every panel that spawns UI), `examples/ui-showcase`,
`crates/editor/examples/custom_property.rs`

## Problem

`UITheme` is a bag of tokens. Every call site that wants a themed element
re-assembles it by hand from `UINode`, `UIMaterial`, `UIInteractionStyle`,
`UIText` and friends, reading tokens one at a time:

```rust
UIMaterial {
    corner_radius: theme.radius_sm,
    ..UIMaterial::flat(TRANSPARENT)
},
Interactable,
UIInteractionStyle {
    normal: TRANSPARENT,
    hovered: theme.surface_hovered,
    pressed: selection_tint(&theme),
    disabled: TRANSPARENT,
},
```

A survey of every `UITheme` consumer (ui crate, 13 editor files, ui-showcase)
found the same handful of elements rebuilt over and over:

| Use case | Where | Today's recipe |
|---|---|---|
| Body text | content, hierarchy, inspector, shell — four identical private `text()` helpers; `UIText::from_theme` | `font_size_md` + `line_height(md)`. Never sets `color`, so text silently uses `UIText::default()`'s cream, not `theme.text` |
| Text variants | everywhere | muted, small (`font_size_sm`), mono, caption (sm + muted + medium weight), single-line (`wrap: false, ellipsis: true`, ~12 sites); add_component re-invents `sized`/`muted`/`mono` |
| Surfaces | dock, inspector cards, shell, workspace, menus | `UIMaterial { corner_radius: theme.radius_X, ..flat(theme.surface_Y) }` or `with_border(fill, theme.border, 1.0)` |
| Solid buttons | `widgets::button` (showcase destructures its 5-tuple to change the width), showcase menu rows | raised / hovered / accent / disabled |
| Ghost rows | content rows, add-component row + entries, window chrome | transparent / hovered / selection tint / transparent (chrome: `error` on Close) |
| Tabs, chips | tabs, content kind tags | active/inactive colour pairs, restyled at runtime |
| Text fields | hierarchy filter, content filter, numeric fields, add-component search, showcase | node + single-line `UIText` + `UITextInput` + `Interactable` + `UIFocusable` + field material |
| Anchored popups | showcase dropdowns, submenu, context menu; add-component menu | popup surface + `UIAnchoredPanel::new(..).with_owner(..).toggled_by_owner()` |
| Divider | add_component | 1px `theme.border` |
| Row/column stacks | everywhere | `with_flex_direction(Row).with_align_items(Center).with_gap(Vec2::new(theme.spacing_sm, 0.0))` |
| Derived colours | `marks::selection_tint`, `marks::TRANSPARENT` | accent at 20% alpha lives outside the theme; `TRANSPARENT` duplicates `Color::TRANSPARENT` |

Tweaking a pre-built element is also painful: `widgets::button` returns a
5-tuple, so changing its width means destructuring and re-spawning every part.

## Goals

- One-line creation of every element above, following the theme by default.
- Every default overridable with chainable modifiers, in any order.
- Composable with arbitrary extra components (markers, actions) without an
  escape to raw tuples.
- Refresh systems that restyle on state change reuse exactly the colours the
  builders used.
- Migrate every existing call site; delete the duplicated helpers.

Non-goals: live theme switching, scroll/virtual-list scaffolding, the editor's
geometric marks, moving hard-coded pixel values onto the spacing scale.

## Constraints

- `ComponentBundle` (`crates/ecs/src/component/bundle.rs`) has a *static*
  component set (`get_component_ids()` takes no `self`). A builder therefore
  cannot carry optional components; each builder type has a fixed set.
  Tuples of bundles are bundles, so extras compose by nesting:
  `(theme.button("Close"), Control::Close)`.
- `EntityCommandQueue::add_child` / `spawn_child_queue` require
  `ComponentBundle + 'static`, so builders cannot borrow `&UITheme`.
- `concerto_ecs::component::bundle::{ComponentBundle, ComponentSink}` and
  `concerto_ecs::table::Table` are public, so `concerto_ui` can implement
  `ComponentBundle` for its own types.
- `apply_interaction_styles` writes `UIMaterial::color` from
  `UIInteractionStyle` every frame, so builders need only set the style, and
  refresh code that also writes `material.color` is redundant.

## Design

### Module layout

New module `concerto_ui::elements`:

```
crates/ui/src/elements/
  mod.rs       — Themed/Layout/Typography/Shape/Interactive traits, bundle! macro, prelude
  text.rs      — Text, Label
  surface.rs   — Surface, Stack, Divider
  button.rs    — ButtonVariant, Pressable, Button
  field.rs     — TextField
  chip.rs      — Chip, ChipColors
  popup.rs     — Popup
  controls.rs  — Checkbox, Slider
```

Builders are constructed from the theme via inherent methods on `UITheme`
defined in these files (`impl UITheme { pub fn button(&self, ..) -> Button }`).
`theme.rs` keeps the tokens plus colour presets.

`concerto_ui::elements::prelude` re-exports the four modifier traits, so call
sites write `use concerto_ui::elements::prelude::*;`.

### Mechanics

Each builder owns a `UITheme` clone (≈40 plain values, cloned once per builder)
plus its parts. Modifiers that only touch one component mutate it eagerly.
Colours that depend on several settings (interaction variant × selected ×
overrides, chip selection) are resolved when the builder is written into the
world, so `.pressed(theme.error).ghost()` equals `.ghost().pressed(theme.error)`.

A private macro implements `ComponentBundle` by delegating to the builder's
flat parts tuple:

```rust
macro_rules! bundle {
    ($builder:ty => $parts:ty) => {
        impl ComponentBundle for $builder {
            fn get_component_ids() -> Vec<ComponentId> {
                <$parts as ComponentBundle>::get_component_ids()
            }
            fn generate_empty_table() -> Table {
                <$parts as ComponentBundle>::generate_empty_table()
            }
            fn write_into<S: ComponentSink>(self, sink: &mut S, tick: u32) {
                self.into_parts().write_into(sink, tick)
            }
        }
    };
}
```

No `.build()` call is ever needed at a call site.

### Modifier traits

Each trait exposes one or two accessors; everything else is a provided
method. The accessors are public, so downstream crates can implement the
traits for their own builders and inherit every modifier.

```rust
pub trait Themed {
    fn theme(&self) -> &UITheme;
}

pub trait Layout: Sized {
    fn node_mut(&mut self) -> &mut UINode;
    // provided:
    fn width(self, UIValue) -> Self;          fn height(self, UIValue) -> Self;
    fn size(self, UIValue, UIValue) -> Self;
    fn min_width(self, UIValue) -> Self;      fn min_height(self, UIValue) -> Self;
    fn max_width(self, UIValue) -> Self;      fn max_height(self, UIValue) -> Self;
    fn row(self) -> Self;        // Row + align_items Center
    fn column(self) -> Self;     // Column
    fn gap(self, f32) -> Self;   // same gap on both axes
    fn padding(self, impl Into<UIRect>) -> Self;
    fn margin(self, impl Into<UIRect>) -> Self;
    fn grow(self) -> Self;       // flex_grow 1
    fn shrink(self, f32) -> Self;
    fn fixed(self) -> Self;      // flex_shrink 0
    fn align_items(self, AlignItems) -> Self;
    fn align_self(self, AlignItems) -> Self;
    fn justify(self, AlignContent) -> Self;
    fn clipped(self) -> Self;
    fn hidden(self) -> Self;
    fn z_index(self, i32) -> Self;
    fn node(self, impl FnOnce(UINode) -> UINode) -> Self;  // escape hatch
}

pub trait Typography: Themed + Sized {
    fn text_mut(&mut self) -> &mut UIText;
    // provided:
    fn muted(self) -> Self;              // theme.text_muted
    fn small(self) -> Self;              // font_size_sm + line_height
    fn large(self) -> Self;              // font_size_lg + line_height
    fn font_size(self, f32) -> Self;     // always pairs theme.line_height
    fn mono(self) -> Self;
    fn family(self, FontFamily) -> Self;
    fn weight(self, u16) -> Self;
    fn color(self, Color) -> Self;
    fn single_line(self) -> Self;        // wrap false, ellipsis true
    fn no_wrap(self) -> Self;            // wrap false
}

pub trait Shape: Themed + Sized {
    fn material_mut(&mut self) -> &mut UIMaterial;
    // provided:
    fn radius_sm(self) -> Self;  fn radius_md(self) -> Self;  fn radius_lg(self) -> Self;
    fn radius(self, f32) -> Self;
    fn bordered(self) -> Self;             // theme.border, 1px
    fn border(self, Color, f32) -> Self;
    fn rotation(self, f32) -> Self;
}

pub trait Interactive: Sized {
    fn interaction_mut(&mut self) -> &mut InteractionSpec;
    // provided:
    fn solid(self) -> Self;  fn ghost(self) -> Self;  fn tab(self) -> Self;
    fn selected(self, bool) -> Self;
    fn pressed(self, Color) -> Self;         // override the pressed colour
    fn disabled_color(self, Color) -> Self;  // override the disabled colour
}

pub struct InteractionSpec {
    pub variant: ButtonVariant,
    pub selected: bool,
    pub pressed: Option<Color>,
    pub disabled: Option<Color>,
}
```

`Typography::font_size` is deliberately not named `size`, and popup anchor
modifiers are prefixed `anchor_`, so no builder implementing several traits
has two methods with the same name.

`node.rs` gains `impl From<f32> for UIRect` (all sides), so `padding(8.0)`
works; `UIRect::axes` still covers the two-axis case.

### Theme presets

On `UITheme`, public, used by builders *and* by refresh systems:

```rust
pub fn selection(&self) -> Color;  // accent at 20% alpha
pub fn interaction(&self, variant: ButtonVariant, selected: bool) -> UIInteractionStyle;
pub fn chip_colors(&self, selected: bool) -> ChipColors;

pub enum ButtonVariant { Solid, Ghost, Tab }
pub struct ChipColors { pub fill: Color, pub border: Color, pub text: Color }
```

`interaction` resolves to:

| Variant | normal | normal when selected | hovered | pressed | disabled |
|---|---|---|---|---|---|
| Solid | `surface_raised` | `surface_hovered` | `surface_hovered` | `accent` | `srgba(0.09, 0.075, 0.11, 0.55)` (today's `widgets::button` value) |
| Ghost | `TRANSPARENT` | `selection()` | `surface_hovered` | `selection()` | `TRANSPARENT` |
| Tab | `surface` | `surface_raised` | `surface_hovered` | `accent` | `surface` |

`chip_colors(false)` = `{ TRANSPARENT, border, text_muted }`;
`chip_colors(true)` = `{ selection(), accent, text }`.

### Builder catalogue

Parts are listed in the order of the delegated tuple. "Defaults" lists only
what differs from the components' own defaults.

**Text**

| Constructor | Type | Parts | Traits | Defaults |
|---|---|---|---|---|
| `theme.text(v)` | `Text` | `UIText` | Typography | `font_size_md`, `line_height(md)`, colour `theme.text` |
| `theme.label(v)` | `Label` | `UINode`, `UIText` | Layout, Typography | as `text` |

`Text` is for text that sits on the same entity as another node (a button,
a text field, a mark). `Label` is a standalone text node.

**Surfaces and layout**

| Constructor | Type | Parts | Traits | Defaults |
|---|---|---|---|---|
| `theme.canvas()` | `Surface` | `UINode`, `UIMaterial` | Layout, Shape, `.fill(Color)` | fill `canvas`, no radius |
| `theme.panel()` | `Surface` | same | same | fill `surface`, `radius_lg` |
| `theme.card()` | `Surface` | same | same | fill `surface_raised`, `radius_md` |
| `theme.popup()` | `Surface` | same | same | fill `surface_raised`, bordered, `radius_md`, column, clipped |
| `theme.row()` | `Stack` | `UINode` | Layout | Row, align Center, gap `spacing_sm` |
| `theme.column()` | `Stack` | `UINode` | Layout | Column, gap `spacing_sm` |
| `theme.divider()` | `Divider` | `UINode`, `UIMaterial` | Layout, `.vertical()` | 1px tall, fixed, fill `border` |

**Interactive**

| Constructor | Type | Parts | Traits | Defaults |
|---|---|---|---|---|
| `theme.pressable()` | `Pressable` | `UINode`, `UIMaterial`, `UIInteractionStyle`, `Interactable` | Layout, Shape, Interactive | Ghost, `radius_sm`, fixed |
| `theme.button(v)` | `Button` | `UINode`, `UIMaterial`, `UIInteractionStyle`, `Interactable`, `UIButton`, `UIText` | Layout, Shape, Interactive, Typography | Solid, bordered, `radius_sm`, height `control_height`, fixed, padding `axes(spacing_sm, spacing_md)`, text single-line |
| `theme.text_field(placeholder)` | `TextField` | `UINode`, `UIMaterial`, `UIText`, `UITextInput`, `Interactable`, `UIFocusable` | Layout, Shape, Typography, `.fill(Color)`, `.bare()` | height `control_height`, fixed, padding `axes(spacing_xs, spacing_sm)`, fill `canvas`, bordered, `radius_md`, text no-wrap |
| `theme.chip(v)` | `Chip` | `UINode`, `UIMaterial`, `UIText`, `Interactable` | Layout, Typography, `.selected(bool)` | padding `axes(2, 7)`, fixed, `radius_sm`, bordered, `font_size_sm`, colours from `chip_colors(selected)` |
| `theme.checkbox(v, checked)` | `Checkbox` | `UINode`, `UIMaterial`, `UICheckbox`, `Interactable`, `UIText` | Layout, Shape, Typography | height `control_height`, bordered `surface_raised`; `checked_color: accent`, `unchecked_color: surface_raised` |
| `theme.slider(value, min, max)` | `Slider` | `UINode`, `UIMaterial`, `UISlider`, `Interactable` | Layout, Shape | height `control_height`, fill `surface_raised` |

`.bare()` makes a text field's fill transparent and removes its border (the
content filter sits inside a search pill that is already a surface).
`Pressable` has no text; rows with children (content rows, menu entries, tabs)
use it and add children. The slider's fill child keeps the colour the
`slider` module gives it; that is out of scope.

**Anchored popups**

| Constructor | Type | Parts | Traits | Defaults |
|---|---|---|---|---|
| `theme.dropdown(trigger)` | `Popup` | `UINode`, `UIMaterial`, `UIAnchoredPanel` | Layout, Shape, `.fill(Color)` | `popup()` surface; anchored to `trigger` node, owner `trigger`, toggled by owner, Below, Start, gap `spacing_xs` |
| `theme.context_menu()` | `Popup` | same | same | `popup()` surface; point target, no owner, not owner-toggled |

`Popup` adds `.anchor_side(UIAnchorSide)`, `.anchor_align(UIAnchorAlign)`,
`.anchor_gap(f32)`, `.anchor_target(UIAnchorTarget)`,
`.focus_on_open(Entity)` and `.open(bool)`. An anchored panel must be a UI
root, so a `Popup` is spawned with `cmd.spawn(..)`, never `add_child`; its
rustdoc says so. Visibility is projected from `open` by the crate, so callers
no longer set `with_visible(false)`.

### Examples

```rust
use concerto_ui::elements::prelude::*;

// Toolbar with a fixed-width button and a growing search field.
cmd.entity(toolbar)
    .add_child(theme.button("Save").width(UIValue::Px(112.0)))
    .add_child((theme.text_field("Search entities…").grow().single_line(), Filter))
    .add_child(theme.divider().vertical());

// Inspector card with a header row.
let mut card = stack.spawn_child_queue(theme.card().column().padding(UIRect::axes(6.0, 7.0)));
card.add_child_with(theme.row(), |header| {
    header.add_child(theme.label(name).single_line().grow());
});

// Window chrome control.
theme.button(glyph::X).ghost().pressed(theme.error).family(FontFamily::Name(PHOSPHOR.into())).muted();

// Dropdown opened by a trigger.
let menu = cmd.spawn(theme.dropdown(trigger).width(UIValue::Px(180.0))).entity();

// Refresh system restyling a tab on activation.
*style = theme.interaction(ButtonVariant::Tab, is_active);
```

## Migration

Every call site moves to the builders:

- **editor**: `content.rs`, `hierarchy.rs`, `inspector/{mod,sync,numeric,add_component}.rs`,
  `dock.rs`, `shell.rs`, `tabs.rs`, `window_chrome.rs`, `workspace.rs`,
  `diagnostics.rs`, `fonts.rs`, `marks.rs`.
- **examples**: `examples/ui-showcase/src/main.rs`,
  `crates/editor/examples/custom_property.rs`.
- **ui crate**: `scroll.rs` scrollbar track/thumb use `theme.canvas()`-style
  presets where it reads cleaner; otherwise untouched.

Specific rewrites:

- `fonts::icon(theme, glyph, size)` becomes a thin wrapper returning
  `theme.text(glyph).family(FontFamily::Name(PHOSPHOR.into())).font_size(size).no_wrap()`.
- Dock card titles become `theme.label(t).small().muted().weight(MEDIUM)`.
- `tabs::refresh` and `content::render_marks` reassign
  `theme.interaction(..)` and drop their `material.color` writes;
  `content::render_tags` reads `theme.chip_colors(active)`.
- add_component spawns its menu as `theme.popup().fill(MENU_FILL)` and keeps
  its small `menu_panel(row, search)` helper for the `UIAnchoredPanel` alone:
  the menu is re-anchored to each rebuilt row by re-inserting only that
  component, and it needs the search entity, which exists only after spawning.

Deleted: the four private `text()` helpers, add_component's
`sized`/`muted`/`mono`/`divider`, `widgets::button`, `UIText::from_theme`,
`marks::selection_tint` (→ `theme.selection()`), `marks::TRANSPARENT`
(→ `Color::TRANSPARENT`), and the showcase's `label`/`panel`/`menu_panel`
(`menu_row` survives as a two-line spawn-and-parent helper built on the builders).

Hard-coded per-site pixel values (e.g. `UIRect::axes(6.0, 8.0)`, row heights)
stay as explicit modifiers. Moving them onto the spacing scale shifts pixels
and is a separate decision.

### Expected visual changes

- Body text colour moves from `UIText::default()`'s cream
  `(0.945, 0.925, 0.885)` to `theme.text` `(0.914, 0.914, 0.929)` wherever it
  was built by a `text()` helper. This is a fix.
- Content kind tags go from 10.0 to `font_size_sm` (10.5).
- The showcase's buttons gain `radius_sm` corners; its checkbox uses the
  accent instead of the hard-coded blue.

Nothing else should move.

## Testing

TDD. New integration tests in `crates/ui/tests/elements.rs` spawn each
builder into a `World` and assert on the resulting components:

- defaults equal the documented theme tokens for every constructor;
- modifier order independence (`.pressed(c).ghost()` == `.ghost().pressed(c)`,
  `.small().muted()` == `.muted().small()`);
- `theme.interaction` matches the variant table for every variant × selected;
- `chip_colors` for both states, and `Chip::selected` uses them;
- `Popup` anchor defaults for `dropdown` and `context_menu`;
- nesting a builder in a tuple with extra marker components spawns all of them;
- `font_size` always pairs `theme.line_height`.

None of this needs `Res<Window>`, so it all runs headless.

After migration: the existing editor tests (`tests/tabs.rs`,
`tests/custom_property.rs`, `inspector/tests.rs`, the dock tests) and ui tests
pass unchanged, `cargo clippy --workspace` is clean, and the showcase and editor
are launched once for a visual check against the expected changes above.
