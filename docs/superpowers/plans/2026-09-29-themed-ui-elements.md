# Themed UI Elements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add theme-rooted builder structs to `concerto_ui` that spawn themed panels, buttons, text, fields, chips, controls and anchored popups in one line, then migrate every editor and showcase call site onto them.

**Architecture:** A new `concerto_ui::elements` module. Each builder holds a `UITheme` clone plus its component parts, exposes chainable modifiers through four shared traits (`Layout`, `Typography`, `Shape`, `Interactive`), and implements `ComponentBundle` by delegating to its parts tuple, so it spawns directly and nests in tuples with extra components. Theme presets (`selection`, `interaction`, `chip_colors`) live on `UITheme` so refresh systems reuse the builders' colours.

**Tech Stack:** Rust 2024, the in-repo `concerto_ecs` archetype ECS, `taffy` layout, `glyphon` text.

**Spec:** `docs/superpowers/specs/2026-09-29-themed-ui-elements-design.md`

## Global Constraints

- Named structs/fields for data types, never unnamed tuples (project convention). Bundle parts tuples are the only exception — that is how the ECS spells a bundle.
- Every builder is `ComponentBundle + 'static`; builders own a `UITheme` clone and never borrow.
- `Typography::font_size(size)` always sets `line_height = theme.line_height(size)`.
- Colour that depends on several settings (interaction variant × selected × overrides, chip selection) resolves when the builder is written into the world, so modifier order never matters.
- `ButtonVariant` table (normal / normal-when-selected / hovered / pressed / disabled):
  - Solid: `surface_raised` / `surface_hovered` / `surface_hovered` / `accent` / `Color::srgba(0.09, 0.075, 0.11, 0.55)`
  - Ghost: `Color::TRANSPARENT` / `selection()` / `surface_hovered` / `selection()` / `Color::TRANSPARENT`
  - Tab: `surface` / `surface_raised` / `surface_hovered` / `accent` / `surface`
- `selection()` = accent's sRGB channels at alpha 0.2 (`Color::srgba(a.r, a.g, a.b, 0.2)`).
- `chip_colors(false) = { fill: TRANSPARENT, border: border, text: text_muted }`, `chip_colors(true) = { fill: selection(), border: accent, text: text }`.
- A `Popup` is spawned with `cmd.spawn(..)`, never `add_child` — anchored panels must be UI roots.
- Migration must not move pixels except the spec's listed visual changes (body text colour → `theme.text`; content tags 10.0 → `font_size_sm`; showcase buttons get `radius_sm`, checkbox uses accent).
- Plain layout `UINode`s that read no theme token may stay as raw `UINode`s.
- `cargo clippy --workspace` introduces no new warnings; remove imports that become unused.

## Review Focus

- A builder nested in a tuple with extra marker components must spawn every component (the core composition promise) — pinned in Task 1 (`label_nests_with_extra_components`) and Task 3 (`button_nests_with_extra_components`).
- Chip text colour set via `Typography` is overridden by `chip_colors` at spawn — a reader expects `.muted()` to win; pinned in Task 4 (`chip_selection_overrides_text_colour`) so the documented behaviour is locked.
- Text fields must not ellipsise by default (numeric fields show their full value while editing) — pinned in Task 4 (`text_field_defaults`).
- Re-inserting a `Text` onto an existing entity must replace its `UIText` (add-component count label, via `cmd.insert`) — pinned in Task 1 (`text_reinserted_replaces_existing_text`).
- The tab refresh must keep setting `UIInteractionStyle::normal` to `surface_raised` for the active tab — already pinned by `crates/editor/tests/tabs.rs::activation_updates_style_and_closed_documents_remove_buttons`; Task 6 must keep it green.

---

## File Structure

Create:
- `crates/ui/src/elements/mod.rs` — traits `Themed`, `Layout`, `Typography`, `Shape`, `Interactive`; `InteractionSpec`; `bundle!` macro; `prelude`.
- `crates/ui/src/elements/text.rs` — `Text`, `Label`, `UITheme::{text, label, body_text}`.
- `crates/ui/src/elements/surface.rs` — `Surface`, `Stack`, `Divider`, `UITheme::{canvas, panel, card, popup, row, column, divider}`.
- `crates/ui/src/elements/button.rs` — `Pressable`, `Button`, `UITheme::{pressable, button}`.
- `crates/ui/src/elements/field.rs` — `TextField`, `UITheme::text_field`.
- `crates/ui/src/elements/chip.rs` — `Chip`, `UITheme::chip`.
- `crates/ui/src/elements/controls.rs` — `Checkbox`, `Slider`, `UITheme::{checkbox, slider}`.
- `crates/ui/src/elements/popup.rs` — `Popup`, `UITheme::{dropdown, context_menu}`.
- `crates/ui/tests/elements.rs` — builder tests.

Modify:
- `crates/ui/src/lib.rs` — `pub mod elements;`
- `crates/ui/src/node.rs` — `impl From<f32> for UIRect`.
- `crates/ui/src/theme.rs` — `ButtonVariant`, `ChipColors`, `selection`, `interaction`, `chip_colors`.
- `crates/ui/src/widgets.rs` — delete `button` (Task 9).
- `crates/ui/src/text/mod.rs` — delete `UIText::from_theme` (Task 9).
- Editor: `fonts.rs`, `dock.rs`, `shell.rs`, `tabs.rs`, `window_chrome.rs`, `workspace.rs`, `diagnostics.rs` (Task 6); `content.rs`, `hierarchy.rs` (Task 7); `inspector/{mod,sync,numeric,add_component}.rs`, `marks.rs`, `examples/custom_property.rs` (Task 8).
- `examples/ui-showcase/src/main.rs` (Task 9).

Commands used throughout:
- UI builder tests: `cargo test -p concerto-ui --test elements`
- All UI tests: `cargo test -p concerto-ui`
- Editor tests: `cargo test -p concerto-editor`
- Lints: `cargo clippy --workspace --all-targets`

---

### Task 1: Foundations — presets, traits, bundle macro, text builders

**Files:**
- Create: `crates/ui/src/elements/mod.rs`, `crates/ui/src/elements/text.rs`, `crates/ui/tests/elements.rs`
- Modify: `crates/ui/src/lib.rs`, `crates/ui/src/node.rs` (after `impl UIRect`, ~line 100), `crates/ui/src/theme.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `concerto_ui::theme::{ButtonVariant { Solid, Ghost, Tab }, ChipColors { fill, border, text }}`
  - `UITheme::selection(&self) -> Color`, `UITheme::interaction(&self, ButtonVariant, bool) -> UIInteractionStyle`, `UITheme::chip_colors(&self, bool) -> ChipColors`
  - `impl From<f32> for UIRect`
  - `concerto_ui::elements::{Themed, Layout, Typography, Shape, Interactive, InteractionSpec, Text, Label, prelude}`
  - `UITheme::text(&self, impl Into<String>) -> Text`, `UITheme::label(&self, impl Into<String>) -> Label`, `pub(crate) UITheme::body_text(&self, impl Into<String>) -> UIText`
  - `impl From<Text> for UIText`
  - Macro `bundle!(Builder => PartsType)` usable by later element files (defined at the top of `elements/mod.rs`, before the `mod` lines). Each builder defines a private `fn into_parts(self) -> PartsType`.

- [ ] **Step 1: Write the failing tests**

Create `crates/ui/tests/elements.rs`:

```rust
//! Covers the themed element builders: defaults follow the theme, modifiers compose in any order.
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, World,
    component::bundle::ComponentBundle,
};
use concerto_ui::{
    elements::prelude::*,
    interaction::UIInteractionStyle,
    material::UIMaterial,
    node::{AlignItems, FlexDirection, Overflow, UINode, UIRect},
    text::{FontFamily, UIText},
    theme::{ButtonVariant, ChipColors, UITheme},
    transform::UIValue,
};
use glam::Vec2;

struct Spawned {
    world: World,
    entity: Entity,
}

impl Spawned {
    fn get<T: Component>(&self) -> &T {
        self.world
            .get_component_for_entity::<T>(self.entity)
            .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<T>()))
    }

    fn has<T: Component>(&self) -> bool {
        self.world.get_component_for_entity::<T>(self.entity).is_some()
    }
}

fn spawn(bundle: impl ComponentBundle + 'static) -> Spawned {
    let mut world = World::default();
    let entity = world.spawn(bundle);
    Spawned { world, entity }
}

fn theme() -> UITheme {
    UITheme::nocturne()
}

fn colors(style: &UIInteractionStyle) -> [Color; 4] {
    [style.normal, style.hovered, style.pressed, style.disabled]
}

#[derive(Component)]
struct Marker;

#[test]
fn a_single_value_pads_every_side() {
    assert_eq!(UIRect::from(3.0), UIRect::all(3.0));
}

#[test]
fn selection_is_the_accent_at_a_fifth_alpha() {
    let theme = theme();
    let accent = theme.accent.to_srgba();
    assert_eq!(
        theme.selection(),
        Color::srgba(accent.r, accent.g, accent.b, 0.2)
    );
}

#[test]
fn interaction_follows_the_variant_table() {
    let t = theme();
    let disabled_solid = Color::srgba(0.09, 0.075, 0.11, 0.55);
    let cases = [
        (ButtonVariant::Solid, false, [t.surface_raised, t.surface_hovered, t.accent, disabled_solid]),
        (ButtonVariant::Solid, true, [t.surface_hovered, t.surface_hovered, t.accent, disabled_solid]),
        (ButtonVariant::Ghost, false, [Color::TRANSPARENT, t.surface_hovered, t.selection(), Color::TRANSPARENT]),
        (ButtonVariant::Ghost, true, [t.selection(), t.surface_hovered, t.selection(), Color::TRANSPARENT]),
        (ButtonVariant::Tab, false, [t.surface, t.surface_hovered, t.accent, t.surface]),
        (ButtonVariant::Tab, true, [t.surface_raised, t.surface_hovered, t.accent, t.surface]),
    ];
    for (variant, selected, expected) in cases {
        assert_eq!(
            colors(&t.interaction(variant, selected)),
            expected,
            "{variant:?} selected={selected}"
        );
    }
}

#[test]
fn chip_colours_mark_the_selected_chip() {
    let t = theme();
    assert_eq!(
        t.chip_colors(false),
        ChipColors { fill: Color::TRANSPARENT, border: t.border, text: t.text_muted }
    );
    assert_eq!(
        t.chip_colors(true),
        ChipColors { fill: t.selection(), border: t.accent, text: t.text }
    );
}

#[test]
fn text_defaults_follow_the_theme() {
    let t = theme();
    let spawned = spawn(t.text("Warren"));
    let text = spawned.get::<UIText>();
    assert_eq!(text.text, "Warren");
    assert_eq!(text.font_size, t.font_size_md);
    assert_eq!(text.line_height, t.line_height(t.font_size_md));
    assert_eq!(text.color, t.text, "body text uses the theme colour, not UIText's default");
    assert!(text.wrap);
    assert!(!text.ellipsis);
}

#[test]
fn a_label_is_a_node_with_text() {
    let spawned = spawn(theme().label("Curiosities"));
    assert_eq!(spawned.get::<UINode>(), &UINode::default());
    assert_eq!(spawned.get::<UIText>().text, "Curiosities");
}

#[test]
fn font_size_always_pairs_its_line_height() {
    let t = theme();
    let spawned = spawn(t.label("x").font_size(20.0));
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, 20.0);
    assert_eq!(text.line_height, t.line_height(20.0));
}

#[test]
fn typography_modifiers_compose_in_any_order() {
    let t = theme();
    let a = spawn(t.text("x").small().muted().mono().weight(500));
    let b = spawn(t.text("x").mono().weight(500).muted().small());
    for spawned in [&a, &b] {
        let text = spawned.get::<UIText>();
        assert_eq!(text.font_size, t.font_size_sm);
        assert_eq!(text.line_height, t.line_height(t.font_size_sm));
        assert_eq!(text.color, t.text_muted);
        assert!(matches!(text.font_family, FontFamily::Monospace));
        assert_eq!(text.font_weight, 500);
    }
}

#[test]
fn large_color_and_family_set_their_fields() {
    let t = theme();
    let spawned = spawn(
        t.text("x")
            .large()
            .color(t.accent)
            .family(FontFamily::Name("Inter".into())),
    );
    let text = spawned.get::<UIText>();
    assert_eq!(text.font_size, t.font_size_lg);
    assert_eq!(text.color, t.accent);
    assert!(matches!(&text.font_family, FontFamily::Name(name) if name == "Inter"));
}

#[test]
fn single_line_ellipsises_and_no_wrap_does_not() {
    let t = theme();
    let single = spawn(t.text("x").single_line());
    assert!(!single.get::<UIText>().wrap);
    assert!(single.get::<UIText>().ellipsis);
    let plain = spawn(t.text("x").no_wrap());
    assert!(!plain.get::<UIText>().wrap);
    assert!(!plain.get::<UIText>().ellipsis);
}

#[test]
fn layout_modifiers_shape_the_node() {
    let spawned = spawn(
        theme()
            .label("x")
            .width(UIValue::Px(10.0))
            .height(UIValue::Px(12.0))
            .min_width(UIValue::Px(1.0))
            .max_height(UIValue::Px(40.0))
            .row()
            .gap(3.0)
            .padding(2.0)
            .margin(UIRect::axes(1.0, 4.0))
            .grow()
            .fixed()
            .align_self(AlignItems::End)
            .clipped()
            .hidden()
            .z_index(5),
    );
    let node = spawned.get::<UINode>();
    assert_eq!(node.width, UIValue::Px(10.0));
    assert_eq!(node.height, UIValue::Px(12.0));
    assert_eq!(node.min_width, UIValue::Px(1.0));
    assert_eq!(node.max_height, UIValue::Px(40.0));
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(node.align_items, Some(AlignItems::Center), "row() centres its children");
    assert_eq!(node.gap, Vec2::splat(3.0));
    assert_eq!(node.padding, UIRect::all(2.0));
    assert_eq!(node.margin, UIRect::axes(1.0, 4.0));
    assert_eq!(node.flex_grow, 1.0);
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.align_self, Some(AlignItems::End));
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert!(!node.visible);
    assert_eq!(node.z_index, 5);
}

#[test]
fn the_node_escape_hatch_reaches_any_uinode_setting() {
    let spawned = spawn(theme().label("x").node(|node| node.with_overflow_y(Overflow::Scroll)));
    assert_eq!(spawned.get::<UINode>().overflow_y, Overflow::Scroll);
}

#[test]
fn label_nests_with_extra_components() {
    let spawned = spawn((theme().label("x").muted(), Marker));
    assert!(spawned.has::<Marker>());
    assert!(spawned.has::<UINode>());
    assert_eq!(spawned.get::<UIText>().color, theme().text_muted);
}

#[test]
fn text_reinserted_replaces_existing_text() {
    let t = theme();
    let mut spawned = spawn((UINode::default(), t.text("before")));
    spawned.world.insert(t.text("after").muted(), spawned.entity);
    assert_eq!(spawned.get::<UIText>().text, "after");
    assert_eq!(spawned.get::<UIText>().color, t.text_muted);
}

#[test]
fn text_converts_into_a_plain_uitext() {
    let t = theme();
    let text: UIText = t.text("x").small().into();
    assert_eq!(text.font_size, t.font_size_sm);
}

#[allow(dead_code)]
fn unused_material_import_guard(_: UIMaterial) {}
```

`World::insert(bundle, entity)` is used above; if the method has a different name, check `crates/ecs/src/world.rs` (search `pub fn insert`) and use it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p concerto-ui --test elements`
Expected: compile errors — `elements` module, `ButtonVariant`, `ChipColors`, `From<f32> for UIRect` do not exist.

- [ ] **Step 3: Add `From<f32> for UIRect`**

In `crates/ui/src/node.rs`, directly after the closing `}` of `impl UIRect`:

```rust
impl From<f32> for UIRect {
    /// The same value on all four sides.
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}
```

- [ ] **Step 4: Add the theme presets**

In `crates/ui/src/theme.rs`, add `use crate::interaction::UIInteractionStyle;` to the imports, then above `impl UITheme`:

```rust
/// The colour scheme an interactive element follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonVariant {
    /// A raised control: buttons, menu rows.
    Solid,
    /// No fill until hovered: list rows, icon controls.
    Ghost,
    /// A document tab: sits on the surface, raised when selected.
    Tab,
}

/// A chip's fill, outline and text colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipColors {
    pub fill: Color,
    pub border: Color,
    pub text: Color,
}

const DISABLED_SOLID: Color = Color::srgba(0.09, 0.075, 0.11, 0.55);
```

and inside `impl UITheme` (after `line_height`):

```rust
    /// The wash of accent behind a selected row.
    pub fn selection(&self) -> Color {
        let accent = self.accent.to_srgba();
        Color::srgba(accent.r, accent.g, accent.b, 0.2)
    }

    /// The interaction colours for a variant, raised to its selected fill when `selected`.
    pub fn interaction(&self, variant: ButtonVariant, selected: bool) -> UIInteractionStyle {
        match variant {
            ButtonVariant::Solid => UIInteractionStyle {
                normal: if selected { self.surface_hovered } else { self.surface_raised },
                hovered: self.surface_hovered,
                pressed: self.accent,
                disabled: DISABLED_SOLID,
            },
            ButtonVariant::Ghost => UIInteractionStyle {
                normal: if selected { self.selection() } else { Color::TRANSPARENT },
                hovered: self.surface_hovered,
                pressed: self.selection(),
                disabled: Color::TRANSPARENT,
            },
            ButtonVariant::Tab => UIInteractionStyle {
                normal: if selected { self.surface_raised } else { self.surface },
                hovered: self.surface_hovered,
                pressed: self.accent,
                disabled: self.surface,
            },
        }
    }

    /// A chip's colours, marked with the accent when `selected`.
    pub fn chip_colors(&self, selected: bool) -> ChipColors {
        if selected {
            ChipColors { fill: self.selection(), border: self.accent, text: self.text }
        } else {
            ChipColors { fill: Color::TRANSPARENT, border: self.border, text: self.text_muted }
        }
    }
```

- [ ] **Step 5: Create `crates/ui/src/elements/mod.rs`**

```rust
//! Themed element builders.
//!
//! Each builder is created from a [`UITheme`] (`theme.button("Save")`, `theme.card()`), follows
//! the theme by default, and is itself a [`ComponentBundle`](concerto_ecs::component::bundle::ComponentBundle):
//! spawn it directly, or nest it in a tuple with extra components.
//!
//! ```ignore
//! use concerto_ui::elements::prelude::*;
//! cmd.entity(toolbar)
//!     .add_child(theme.button("Save").width(UIValue::Px(112.0)))
//!     .add_child((theme.text_field("Search…").grow(), Filter));
//! ```
use concerto_color::Color;
use glam::Vec2;

use crate::{
    interaction::UIInteractionStyle,
    material::UIMaterial,
    node::{AlignContent, AlignItems, FlexDirection, UINode, UIRect},
    text::{FontFamily, UIText},
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};

/// Implements `ComponentBundle` for a builder by delegating to its `into_parts` tuple.
macro_rules! bundle {
    ($builder:ty => $parts:ty) => {
        impl concerto_ecs::component::bundle::ComponentBundle for $builder {
            fn get_component_ids() -> Vec<concerto_ecs::component::ComponentId> {
                <$parts as concerto_ecs::component::bundle::ComponentBundle>::get_component_ids()
            }

            fn generate_empty_table() -> concerto_ecs::table::Table {
                <$parts as concerto_ecs::component::bundle::ComponentBundle>::generate_empty_table()
            }

            fn write_into<S: concerto_ecs::component::bundle::ComponentSink>(
                self,
                sink: &mut S,
                current_tick: u32,
            ) {
                concerto_ecs::component::bundle::ComponentBundle::write_into(
                    self.into_parts(),
                    sink,
                    current_tick,
                )
            }
        }
    };
}

mod text;

pub use text::{Label, Text};

/// The modifier traits, for `use concerto_ui::elements::prelude::*;`.
pub mod prelude {
    pub use super::{Interactive, Layout, Shape, Themed, Typography};
}

/// A builder that carries the theme it was created from.
pub trait Themed {
    fn theme(&self) -> &UITheme;
}

/// Size, flex and spacing modifiers for any builder with a [`UINode`].
pub trait Layout: Sized {
    fn node_mut(&mut self) -> &mut UINode;

    /// Escape hatch for any [`UINode`] setting without a modifier of its own.
    fn node(mut self, edit: impl FnOnce(UINode) -> UINode) -> Self {
        let node = std::mem::take(self.node_mut());
        *self.node_mut() = edit(node);
        self
    }

    fn width(self, width: UIValue) -> Self {
        self.node(|node| node.with_width(width))
    }

    fn height(self, height: UIValue) -> Self {
        self.node(|node| node.with_height(height))
    }

    fn size(self, width: UIValue, height: UIValue) -> Self {
        self.node(|node| node.with_size(width, height))
    }

    fn min_width(self, min_width: UIValue) -> Self {
        self.node(|node| node.with_min_width(min_width))
    }

    fn min_height(self, min_height: UIValue) -> Self {
        self.node(|node| node.with_min_height(min_height))
    }

    fn max_width(self, max_width: UIValue) -> Self {
        self.node(|node| node.with_max_width(max_width))
    }

    fn max_height(self, max_height: UIValue) -> Self {
        self.node(|node| node.with_max_height(max_height))
    }

    /// Lays children out left to right, centred on the cross axis.
    fn row(self) -> Self {
        self.node(|node| {
            node.with_flex_direction(FlexDirection::Row)
                .with_align_items(AlignItems::Center)
        })
    }

    /// Lays children out top to bottom.
    fn column(self) -> Self {
        self.node(|node| node.with_flex_direction(FlexDirection::Column))
    }

    /// The same space between children on both axes.
    fn gap(self, gap: f32) -> Self {
        self.node(|node| node.with_gap(Vec2::splat(gap)))
    }

    fn padding(self, padding: impl Into<UIRect>) -> Self {
        let padding = padding.into();
        self.node(|node| node.with_padding(padding))
    }

    fn margin(self, margin: impl Into<UIRect>) -> Self {
        let margin = margin.into();
        self.node(|node| node.with_margin(margin))
    }

    /// Takes up the free space along the parent's main axis.
    fn grow(self) -> Self {
        self.node(|node| node.with_flex_grow(1.0))
    }

    fn shrink(self, shrink: f32) -> Self {
        self.node(|node| node.with_flex_shrink(shrink))
    }

    /// Never shrinks below its own size.
    fn fixed(self) -> Self {
        self.shrink(0.0)
    }

    fn align_items(self, align: AlignItems) -> Self {
        self.node(|node| node.with_align_items(align))
    }

    fn align_self(self, align: AlignItems) -> Self {
        self.node(|node| node.with_align_self(align))
    }

    fn justify(self, justify: AlignContent) -> Self {
        self.node(|node| node.with_justify_content(justify))
    }

    fn clipped(self) -> Self {
        self.node(UINode::clipped)
    }

    fn hidden(self) -> Self {
        self.node(|node| node.with_visible(false))
    }

    fn z_index(self, z_index: i32) -> Self {
        self.node(|node| node.with_z_index(z_index))
    }
}

/// Type-scale and colour modifiers for any builder with a [`UIText`].
pub trait Typography: Themed + Sized {
    fn text_mut(&mut self) -> &mut UIText;

    /// Sets the size and the theme's leading for it.
    fn font_size(mut self, size: f32) -> Self {
        let line_height = self.theme().line_height(size);
        let text = self.text_mut();
        text.font_size = size;
        text.line_height = line_height;
        self
    }

    fn small(self) -> Self {
        let size = self.theme().font_size_sm;
        self.font_size(size)
    }

    fn large(self) -> Self {
        let size = self.theme().font_size_lg;
        self.font_size(size)
    }

    fn muted(self) -> Self {
        let color = self.theme().text_muted;
        self.color(color)
    }

    fn color(mut self, color: Color) -> Self {
        self.text_mut().color = color;
        self
    }

    fn family(mut self, family: FontFamily) -> Self {
        self.text_mut().font_family = family;
        self
    }

    fn mono(self) -> Self {
        self.family(FontFamily::Monospace)
    }

    fn weight(mut self, weight: u16) -> Self {
        self.text_mut().font_weight = weight;
        self
    }

    /// One line, cut short with an ellipsis when it does not fit.
    fn single_line(mut self) -> Self {
        let text = self.text_mut();
        text.wrap = false;
        text.ellipsis = true;
        self
    }

    /// One line, never cut short.
    fn no_wrap(mut self) -> Self {
        self.text_mut().wrap = false;
        self
    }
}

/// Corner, border and rotation modifiers for any builder with a [`UIMaterial`].
pub trait Shape: Themed + Sized {
    fn material_mut(&mut self) -> &mut UIMaterial;

    fn radius(mut self, radius: f32) -> Self {
        self.material_mut().corner_radius = radius;
        self
    }

    fn radius_sm(self) -> Self {
        let radius = self.theme().radius_sm;
        self.radius(radius)
    }

    fn radius_md(self) -> Self {
        let radius = self.theme().radius_md;
        self.radius(radius)
    }

    fn radius_lg(self) -> Self {
        let radius = self.theme().radius_lg;
        self.radius(radius)
    }

    fn border(mut self, color: Color, width: f32) -> Self {
        let material = self.material_mut();
        material.border_color = color.to_linear();
        material.border_width = width;
        self
    }

    /// The theme's 1px border.
    fn bordered(self) -> Self {
        let color = self.theme().border;
        self.border(color, 1.0)
    }

    fn rotation(mut self, radians: f32) -> Self {
        self.material_mut().rotation = radians;
        self
    }
}

/// How an interactive element colours itself; resolved against the theme at spawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionSpec {
    pub variant: ButtonVariant,
    pub selected: bool,
    /// Replaces the variant's pressed colour.
    pub pressed: Option<Color>,
    /// Replaces the variant's disabled colour.
    pub disabled: Option<Color>,
}

impl InteractionSpec {
    pub fn new(variant: ButtonVariant) -> Self {
        Self {
            variant,
            selected: false,
            pressed: None,
            disabled: None,
        }
    }

    pub fn resolve(&self, theme: &UITheme) -> UIInteractionStyle {
        let mut style = theme.interaction(self.variant, self.selected);
        if let Some(pressed) = self.pressed {
            style.pressed = pressed;
        }
        if let Some(disabled) = self.disabled {
            style.disabled = disabled;
        }
        style
    }
}

/// Variant and state modifiers for any builder with an [`InteractionSpec`].
pub trait Interactive: Sized {
    fn interaction_mut(&mut self) -> &mut InteractionSpec;

    fn variant(mut self, variant: ButtonVariant) -> Self {
        self.interaction_mut().variant = variant;
        self
    }

    fn solid(self) -> Self {
        self.variant(ButtonVariant::Solid)
    }

    fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }

    fn tab(self) -> Self {
        self.variant(ButtonVariant::Tab)
    }

    fn selected(mut self, selected: bool) -> Self {
        self.interaction_mut().selected = selected;
        self
    }

    fn pressed(mut self, color: Color) -> Self {
        self.interaction_mut().pressed = Some(color);
        self
    }

    fn disabled_color(mut self, color: Color) -> Self {
        self.interaction_mut().disabled = Some(color);
        self
    }
}
```

`AlignContent`, `AlignItems`, `FlexDirection` are re-exported from `crate::node` (`pub use taffy::{AlignContent, AlignItems, FlexDirection, Overflow, Position};`).

- [ ] **Step 6: Create `crates/ui/src/elements/text.rs`**

```rust
//! Text builders.
use super::{Layout, Themed, Typography};
use crate::{node::UINode, text::UIText, theme::UITheme};

/// Text for an entity that already has a node: a button, a field, an icon slot.
pub struct Text {
    pub(crate) theme: UITheme,
    pub(crate) text: UIText,
}

/// A standalone text node.
pub struct Label {
    node: UINode,
    text: Text,
}

impl UITheme {
    /// Body text: the medium size in the theme's text colour.
    pub fn text(&self, value: impl Into<String>) -> Text {
        Text {
            theme: self.clone(),
            text: self.body_text(value),
        }
    }

    /// A node holding body text.
    pub fn label(&self, value: impl Into<String>) -> Label {
        Label {
            node: UINode::default(),
            text: self.text(value),
        }
    }

    pub(crate) fn body_text(&self, value: impl Into<String>) -> UIText {
        UIText {
            text: value.into(),
            font_size: self.font_size_md,
            line_height: self.line_height(self.font_size_md),
            color: self.text,
            ..Default::default()
        }
    }
}

impl Text {
    fn into_parts(self) -> UIText {
        self.text
    }
}

impl From<Text> for UIText {
    fn from(text: Text) -> Self {
        text.text
    }
}

impl Label {
    fn into_parts(self) -> (UINode, UIText) {
        (self.node, self.text.text)
    }
}

bundle!(Text => UIText);
bundle!(Label => (UINode, UIText));

impl Themed for Text {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Typography for Text {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}

impl Themed for Label {
    fn theme(&self) -> &UITheme {
        &self.text.theme
    }
}

impl Typography for Label {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text.text
    }
}

impl Layout for Label {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}
```

Note `UITheme` has both a field `text: Color` and the method `text(..)`; Rust keeps them in separate namespaces, and `self.text` inside `body_text` is the field.

- [ ] **Step 7: Register the module**

In `crates/ui/src/lib.rs`, add `pub mod elements;` in the alphabetical `pub mod` list (between `anchor`/`checkbox` … after `checkbox`: `pub mod elements;` before `pub mod focus;`).

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p concerto-ui --test elements`
Expected: all tests PASS. Then `cargo test -p concerto-ui` — everything PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/ui/src/elements crates/ui/src/lib.rs crates/ui/src/node.rs crates/ui/src/theme.rs crates/ui/tests/elements.rs
git commit -m "Add themed element traits, theme presets and text builders"
```

---

### Task 2: Surfaces, stacks and dividers

**Files:**
- Create: `crates/ui/src/elements/surface.rs`
- Modify: `crates/ui/src/elements/mod.rs` (add `mod surface;` + re-export), `crates/ui/tests/elements.rs`

**Interfaces:**
- Consumes: `Themed`, `Layout`, `Shape`, `bundle!` from Task 1.
- Produces:
  - `Surface` with `pub(crate)` fields `theme: UITheme`, `node: UINode`, `material: UIMaterial`; inherent `pub fn fill(self, Color) -> Self`; `pub(crate) fn into_parts(self) -> (UINode, UIMaterial)`; implements `Themed`, `Layout`, `Shape`.
  - `Stack` (`Layout`), `Divider` (`Layout`, inherent `pub fn vertical(self) -> Self`).
  - `UITheme::{canvas, panel, card, popup, row, column, divider}(&self)`.

- [ ] **Step 1: Write the failing tests** (append to `crates/ui/tests/elements.rs`)

```rust
fn material_of(spawned: &Spawned) -> &UIMaterial {
    spawned.get::<UIMaterial>()
}

#[test]
fn each_surface_takes_its_fill_and_radius_from_the_theme() {
    let t = theme();
    let cases = [
        (spawn(t.canvas()), t.canvas, 0.0),
        (spawn(t.panel()), t.surface, t.radius_lg),
        (spawn(t.card()), t.surface_raised, t.radius_md),
    ];
    for (spawned, fill, radius) in &cases {
        let material = material_of(spawned);
        assert_eq!(material.color, fill.to_linear());
        assert_eq!(material.corner_radius, *radius);
        assert_eq!(material.border_width, 0.0);
    }
}

#[test]
fn a_popup_is_a_bordered_clipped_column_card() {
    let t = theme();
    let spawned = spawn(t.popup());
    let material = material_of(&spawned);
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.corner_radius, t.radius_md);
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    let node = spawned.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.overflow_x, Overflow::Hidden);
    assert_eq!(node.overflow_y, Overflow::Hidden);
}

#[test]
fn shape_modifiers_restyle_a_surface() {
    let t = theme();
    let spawned = spawn(
        t.panel()
            .fill(t.accent)
            .radius_sm()
            .border(t.focus, 2.0)
            .rotation(0.5),
    );
    let material = material_of(&spawned);
    assert_eq!(material.color, t.accent.to_linear());
    assert_eq!(material.corner_radius, t.radius_sm);
    assert_eq!(material.border_color, t.focus.to_linear());
    assert_eq!(material.border_width, 2.0);
    assert_eq!(material.rotation, 0.5);
}

#[test]
fn stacks_space_their_children_by_the_theme() {
    let t = theme();
    let row = spawn(t.row());
    let node = row.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Row);
    assert_eq!(node.align_items, Some(AlignItems::Center));
    assert_eq!(node.gap, Vec2::splat(t.spacing_sm));

    let column = spawn(t.column().gap(0.0));
    let node = column.get::<UINode>();
    assert_eq!(node.flex_direction, FlexDirection::Column);
    assert_eq!(node.gap, Vec2::ZERO);
}

#[test]
fn dividers_are_one_border_coloured_pixel() {
    let t = theme();
    let horizontal = spawn(t.divider());
    assert_eq!(horizontal.get::<UINode>().height, UIValue::Px(1.0));
    assert_eq!(horizontal.get::<UINode>().flex_shrink, 0.0);
    assert_eq!(horizontal.get::<UINode>().align_self, Some(AlignItems::Stretch));
    assert_eq!(material_of(&horizontal).color, t.border.to_linear());

    let vertical = spawn(t.divider().vertical());
    assert_eq!(vertical.get::<UINode>().width, UIValue::Px(1.0));
    assert_eq!(vertical.get::<UINode>().height, UIValue::Auto);
}
```

Also delete the `unused_material_import_guard` function from Task 1 now that `UIMaterial` is used.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p concerto-ui --test elements`
Expected: compile errors — no method `canvas`/`panel`/`card`/`popup`/`row`/`column`/`divider` on `UITheme`.

- [ ] **Step 3: Create `crates/ui/src/elements/surface.rs`**

```rust
//! Filled surfaces, bare stacks and dividers.
use concerto_color::Color;

use super::{Layout, Shape, Themed};
use crate::{
    material::UIMaterial,
    node::{AlignItems, UINode},
    theme::UITheme,
    transform::UIValue,
};

/// A filled, optionally rounded and bordered node.
pub struct Surface {
    pub(crate) theme: UITheme,
    pub(crate) node: UINode,
    pub(crate) material: UIMaterial,
}

/// A node that only arranges its children.
pub struct Stack {
    node: UINode,
}

/// A one-pixel rule in the theme's border colour.
pub struct Divider {
    node: UINode,
    material: UIMaterial,
}

impl UITheme {
    /// The window's backdrop: canvas fill, square corners.
    pub fn canvas(&self) -> Surface {
        self.surface_of(self.canvas, 0.0)
    }

    /// A docked panel: surface fill, large corners.
    pub fn panel(&self) -> Surface {
        self.surface_of(self.surface, self.radius_lg)
    }

    /// A card inside a panel: raised fill, medium corners.
    pub fn card(&self) -> Surface {
        self.surface_of(self.surface_raised, self.radius_md)
    }

    /// A floating menu body: a bordered, clipped column card.
    pub fn popup(&self) -> Surface {
        self.card().bordered().column().clipped()
    }

    /// Children left to right, centred, `spacing_sm` apart.
    pub fn row(&self) -> Stack {
        Stack {
            node: UINode::default(),
        }
        .row()
        .gap(self.spacing_sm)
    }

    /// Children top to bottom, `spacing_sm` apart.
    pub fn column(&self) -> Stack {
        Stack {
            node: UINode::default(),
        }
        .column()
        .gap(self.spacing_sm)
    }

    /// A horizontal rule; `.vertical()` turns it on its side.
    pub fn divider(&self) -> Divider {
        Divider {
            node: UINode::default()
                .with_height(UIValue::Px(1.0))
                .with_flex_shrink(0.0)
                .with_align_self(AlignItems::Stretch),
            material: UIMaterial::flat(self.border),
        }
    }

    fn surface_of(&self, fill: Color, corner_radius: f32) -> Surface {
        Surface {
            theme: self.clone(),
            node: UINode::default(),
            material: UIMaterial {
                corner_radius,
                ..UIMaterial::flat(fill)
            },
        }
    }
}

impl Surface {
    pub fn fill(mut self, color: Color) -> Self {
        self.material.color = color.to_linear();
        self
    }

    pub(crate) fn into_parts(self) -> (UINode, UIMaterial) {
        (self.node, self.material)
    }
}

impl Stack {
    fn into_parts(self) -> UINode {
        self.node
    }
}

impl Divider {
    pub fn vertical(mut self) -> Self {
        self.node.width = UIValue::Px(1.0);
        self.node.height = UIValue::Auto;
        self
    }

    fn into_parts(self) -> (UINode, UIMaterial) {
        (self.node, self.material)
    }
}

bundle!(Surface => (UINode, UIMaterial));
bundle!(Stack => UINode);
bundle!(Divider => (UINode, UIMaterial));

impl Themed for Surface {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Surface {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Surface {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Layout for Stack {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Layout for Divider {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}
```

- [ ] **Step 4: Register it** — in `elements/mod.rs` add `mod surface;` after `mod text;` and `pub use surface::{Divider, Stack, Surface};`.

- [ ] **Step 5: Run the tests** — `cargo test -p concerto-ui --test elements` → PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/elements crates/ui/tests/elements.rs
git commit -m "Add themed surface, stack and divider builders"
```

---

### Task 3: Pressables and buttons

**Files:**
- Create: `crates/ui/src/elements/button.rs`
- Modify: `crates/ui/src/elements/mod.rs`, `crates/ui/tests/elements.rs`

**Interfaces:**
- Consumes: Task 1 traits, `InteractionSpec`, `UITheme::body_text`; `crate::widgets::UIButton`, `crate::interaction::{Interactable, UIInteractionStyle}`.
- Produces:
  - `Pressable` — parts `(UINode, UIMaterial, UIInteractionStyle, Interactable)`; `Themed`, `Layout`, `Shape`, `Interactive`. Initial `UIMaterial::color` = resolved `style.normal`.
  - `Button` — parts `(UINode, UIMaterial, UIInteractionStyle, Interactable, UIButton, UIText)`; `Themed`, `Layout`, `Shape`, `Interactive`, `Typography`.
  - `UITheme::pressable(&self) -> Pressable`, `UITheme::button(&self, impl Into<String>) -> Button`.

- [ ] **Step 1: Write the failing tests** (append; add `interaction::Interactable` and `widgets::UIButton` to the `concerto_ui` imports at the top)

```rust
#[test]
fn a_pressable_is_a_ghost_row_by_default() {
    let t = theme();
    let spawned = spawn(t.pressable());
    assert!(spawned.has::<Interactable>());
    assert_eq!(
        colors(spawned.get::<UIInteractionStyle>()),
        colors(&t.interaction(ButtonVariant::Ghost, false))
    );
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.corner_radius, t.radius_sm);
    assert_eq!(
        material.color,
        Color::TRANSPARENT.to_linear(),
        "the first frame already shows the normal colour"
    );
    assert_eq!(spawned.get::<UINode>().flex_shrink, 0.0);
}

#[test]
fn a_button_is_a_solid_bordered_control() {
    let t = theme();
    let spawned = spawn(t.button("Save"));
    assert!(spawned.has::<UIButton>());
    assert!(spawned.has::<Interactable>());
    assert_eq!(
        colors(spawned.get::<UIInteractionStyle>()),
        colors(&t.interaction(ButtonVariant::Solid, false))
    );
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    assert_eq!(material.corner_radius, t.radius_sm);
    let node = spawned.get::<UINode>();
    assert_eq!(node.height, UIValue::Px(t.control_height));
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.padding, UIRect::axes(t.spacing_sm, t.spacing_md));
    let text = spawned.get::<UIText>();
    assert_eq!(text.text, "Save");
    assert_eq!(text.color, t.text);
    assert!(!text.wrap && text.ellipsis);
}

#[test]
fn interaction_modifiers_compose_in_any_order() {
    let t = theme();
    let a = spawn(t.button("x").pressed(t.error).ghost().selected(true));
    let b = spawn(t.button("x").selected(true).ghost().pressed(t.error));
    let mut expected = t.interaction(ButtonVariant::Ghost, true);
    expected.pressed = t.error;
    assert_eq!(colors(a.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(colors(b.get::<UIInteractionStyle>()), colors(&expected));
    assert_eq!(a.get::<UIMaterial>().color, t.selection().to_linear());
}

#[test]
fn a_selected_tab_is_raised_and_disabled_colour_overrides() {
    let t = theme();
    let spawned = spawn(t.pressable().tab().selected(true).disabled_color(t.error));
    let style = spawned.get::<UIInteractionStyle>();
    assert_eq!(style.normal, t.surface_raised);
    assert_eq!(style.disabled, t.error);
}

#[test]
fn button_nests_with_extra_components() {
    let spawned = spawn((theme().button("Close").ghost().large(), Marker));
    assert!(spawned.has::<Marker>());
    assert!(spawned.has::<UIButton>());
    assert_eq!(spawned.get::<UIText>().font_size, theme().font_size_lg);
}

#[test]
fn a_char_labels_a_button() {
    assert_eq!(spawn(theme().button('x')).get::<UIText>().text, "x");
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p concerto-ui --test elements` → compile error, no `pressable`/`button` on `UITheme`.

- [ ] **Step 3: Create `crates/ui/src/elements/button.rs`**

```rust
//! Pressable rows and text buttons.
use concerto_color::Color;

use super::{InteractionSpec, Interactive, Layout, Shape, Themed, Typography};
use crate::{
    interaction::{Interactable, UIInteractionStyle},
    material::UIMaterial,
    node::{UINode, UIRect},
    text::UIText,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
    widgets::UIButton,
};

/// A node that reacts to the pointer; add children for its content.
pub struct Pressable {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    interaction: InteractionSpec,
}

/// A pressable with a text label on the same entity.
pub struct Button {
    pressable: Pressable,
    text: UIText,
}

impl UITheme {
    /// A ghost row: no fill until hovered, small corners, never shrinks.
    pub fn pressable(&self) -> Pressable {
        Pressable {
            theme: self.clone(),
            node: UINode::default().with_flex_shrink(0.0),
            material: UIMaterial {
                corner_radius: self.radius_sm,
                ..UIMaterial::flat(Color::TRANSPARENT)
            },
            interaction: InteractionSpec::new(ButtonVariant::Ghost),
        }
    }

    /// A solid, bordered, control-height button with a one-line label.
    pub fn button(&self, label: impl Into<String>) -> Button {
        Button {
            pressable: self
                .pressable()
                .solid()
                .bordered()
                .height(UIValue::Px(self.control_height))
                .padding(UIRect::axes(self.spacing_sm, self.spacing_md)),
            text: UIText {
                wrap: false,
                ellipsis: true,
                ..self.body_text(label)
            },
        }
    }
}

impl Pressable {
    fn into_parts(self) -> (UINode, UIMaterial, UIInteractionStyle, Interactable) {
        let style = self.interaction.resolve(&self.theme);
        let material = UIMaterial {
            color: style.normal.to_linear(),
            ..self.material
        };
        (self.node, material, style, Interactable)
    }
}

impl Button {
    fn into_parts(
        self,
    ) -> (
        UINode,
        UIMaterial,
        UIInteractionStyle,
        Interactable,
        UIButton,
        UIText,
    ) {
        let (node, material, style, interactable) = self.pressable.into_parts();
        (node, material, style, interactable, UIButton, self.text)
    }
}

bundle!(Pressable => (UINode, UIMaterial, UIInteractionStyle, Interactable));
bundle!(Button => (UINode, UIMaterial, UIInteractionStyle, Interactable, UIButton, UIText));

impl Themed for Pressable {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Pressable {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Pressable {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Interactive for Pressable {
    fn interaction_mut(&mut self) -> &mut InteractionSpec {
        &mut self.interaction
    }
}

impl Themed for Button {
    fn theme(&self) -> &UITheme {
        &self.pressable.theme
    }
}

impl Layout for Button {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.pressable.node
    }
}

impl Shape for Button {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.pressable.material
    }
}

impl Interactive for Button {
    fn interaction_mut(&mut self) -> &mut InteractionSpec {
        &mut self.pressable.interaction
    }
}

impl Typography for Button {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}
```

If `UIMaterial` struct-update (`..self.material`) is rejected because `UIMaterial` is not `Clone`/has non-`Copy` fields, that is fine — struct update moves the remaining fields; `self.material` is owned here.

- [ ] **Step 4: Register it** — `mod button;` and `pub use button::{Button, Pressable};` in `elements/mod.rs`.

- [ ] **Step 5: Run the tests** — `cargo test -p concerto-ui --test elements` → PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/elements crates/ui/tests/elements.rs
git commit -m "Add themed pressable and button builders"
```

---

### Task 4: Text fields, chips, checkboxes and sliders

**Files:**
- Create: `crates/ui/src/elements/field.rs`, `crates/ui/src/elements/chip.rs`, `crates/ui/src/elements/controls.rs`
- Modify: `crates/ui/src/elements/mod.rs`, `crates/ui/tests/elements.rs`

**Interfaces:**
- Consumes: Task 1 traits, `UITheme::{body_text, chip_colors}`.
- Produces:
  - `TextField` — parts `(UINode, UIMaterial, UIText, UITextInput, Interactable, UIFocusable)`; `Themed`, `Layout`, `Shape`, `Typography`; inherent `fill(Color)`, `bare()`. `UITheme::text_field(&self, impl Into<String>) -> TextField`.
  - `Chip` — parts `(UINode, UIMaterial, UIText, Interactable)`; `Themed`, `Layout`, `Typography`; inherent `selected(bool)`. `UITheme::chip(&self, impl Into<String>) -> Chip`.
  - `Checkbox` — parts `(UINode, UIMaterial, UICheckbox, Interactable, UIText)`; `Themed`, `Layout`, `Shape`, `Typography`. `UITheme::checkbox(&self, impl Into<String>, bool) -> Checkbox`.
  - `Slider` — parts `(UINode, UIMaterial, UISlider, Interactable)`; `Themed`, `Layout`, `Shape`. `UITheme::slider(&self, f32, f32, f32) -> Slider`.

- [ ] **Step 1: Write the failing tests** (append; add `checkbox::UICheckbox`, `focus::UIFocusable`, `slider::UISlider`, `text_input::UITextInput` to the imports)

```rust
#[test]
fn text_field_defaults() {
    let t = theme();
    let spawned = spawn(t.text_field("Search entities…"));
    assert!(spawned.has::<Interactable>());
    assert!(spawned.has::<UIFocusable>());
    assert_eq!(spawned.get::<UITextInput>().placeholder, "Search entities…");
    let node = spawned.get::<UINode>();
    assert_eq!(node.height, UIValue::Px(t.control_height));
    assert_eq!(node.flex_shrink, 0.0);
    assert_eq!(node.padding, UIRect::axes(t.spacing_xs, t.spacing_sm));
    let material = spawned.get::<UIMaterial>();
    assert_eq!(material.color, t.canvas.to_linear());
    assert_eq!(material.border_color, t.border.to_linear());
    assert_eq!(material.border_width, 1.0);
    assert_eq!(material.corner_radius, t.radius_md);
    let text = spawned.get::<UIText>();
    assert!(!text.wrap, "fields stay on one line");
    assert!(!text.ellipsis, "a field shows its whole value while editing");
    assert_eq!(text.color, t.text);
}

#[test]
fn a_bare_field_has_no_fill_or_border_and_fill_overrides() {
    let t = theme();
    let bare = spawn(t.text_field("").bare());
    assert_eq!(bare.get::<UIMaterial>().color, Color::TRANSPARENT.to_linear());
    assert_eq!(bare.get::<UIMaterial>().border_width, 0.0);
    let filled = spawn(t.text_field("").fill(t.surface));
    assert_eq!(filled.get::<UIMaterial>().color, t.surface.to_linear());
}

#[test]
fn chip_colours_follow_its_selection() {
    let t = theme();
    for selected in [false, true] {
        let spawned = spawn(t.chip("mesh").selected(selected));
        let ChipColors { fill, border, text } = t.chip_colors(selected);
        let material = spawned.get::<UIMaterial>();
        assert_eq!(material.color, fill.to_linear());
        assert_eq!(material.border_color, border.to_linear());
        assert_eq!(material.border_width, 1.0);
        assert_eq!(material.corner_radius, t.radius_sm);
        assert_eq!(spawned.get::<UIText>().color, text);
        assert_eq!(spawned.get::<UIText>().font_size, t.font_size_sm);
        assert!(spawned.has::<Interactable>());
    }
}

#[test]
fn chip_selection_overrides_text_colour() {
    let t = theme();
    let spawned = spawn(t.chip("mesh").muted().selected(true));
    assert_eq!(
        spawned.get::<UIText>().color,
        t.text,
        "a chip's text colour always comes from chip_colors"
    );
}

#[test]
fn a_checkbox_uses_the_accent_when_checked() {
    let t = theme();
    let spawned = spawn(t.checkbox("CHECK", true));
    let checkbox = spawned.get::<UICheckbox>();
    assert!(checkbox.checked);
    assert_eq!(checkbox.checked_color, t.accent);
    assert_eq!(checkbox.unchecked_color, t.surface_raised);
    assert_eq!(spawned.get::<UIText>().text, "CHECK");
    assert_eq!(spawned.get::<UINode>().height, UIValue::Px(t.control_height));
    assert!(spawned.has::<Interactable>());
}

#[test]
fn a_slider_holds_its_range() {
    let t = theme();
    let spawned = spawn(t.slider(0.62, 0.0, 1.0).width(UIValue::Px(140.0)));
    let slider = spawned.get::<UISlider>();
    assert_eq!((slider.value, slider.min, slider.max), (0.62, 0.0, 1.0));
    assert_eq!(spawned.get::<UIMaterial>().color, t.surface_raised.to_linear());
    assert_eq!(spawned.get::<UINode>().width, UIValue::Px(140.0));
    assert!(spawned.has::<Interactable>());
}
```

(The tuple in `a_slider_holds_its_range` is a test comparison, not a data type.)

- [ ] **Step 2: Run to verify failure** — compile errors for missing constructors.

- [ ] **Step 3: Create `crates/ui/src/elements/field.rs`**

```rust
//! Single-line text fields.
use concerto_color::Color;

use super::{Layout, Shape, Themed, Typography};
use crate::{
    focus::UIFocusable,
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    text::UIText,
    text_input::UITextInput,
    theme::UITheme,
    transform::UIValue,
};

/// An editable, focusable, one-line text field.
pub struct TextField {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    text: UIText,
    input: UITextInput,
}

impl UITheme {
    /// A control-height field on the canvas colour, bordered, never wrapping.
    pub fn text_field(&self, placeholder: impl Into<String>) -> TextField {
        TextField {
            theme: self.clone(),
            node: UINode::default()
                .with_height(UIValue::Px(self.control_height))
                .with_flex_shrink(0.0)
                .with_padding(UIRect::axes(self.spacing_xs, self.spacing_sm)),
            material: UIMaterial {
                corner_radius: self.radius_md,
                ..UIMaterial::with_border(self.canvas, self.border, 1.0)
            },
            text: UIText {
                wrap: false,
                ..self.body_text("")
            },
            input: UITextInput::new(placeholder),
        }
    }
}

impl TextField {
    pub fn fill(mut self, color: Color) -> Self {
        self.material.color = color.to_linear();
        self
    }

    /// No fill and no border, for a field sitting inside another surface.
    pub fn bare(mut self) -> Self {
        self.material.color = Color::TRANSPARENT.to_linear();
        self.material.border_color = Color::TRANSPARENT.to_linear();
        self.material.border_width = 0.0;
        self
    }

    fn into_parts(
        self,
    ) -> (
        UINode,
        UIMaterial,
        UIText,
        UITextInput,
        Interactable,
        UIFocusable,
    ) {
        (
            self.node,
            self.material,
            self.text,
            self.input,
            Interactable,
            UIFocusable,
        )
    }
}

bundle!(TextField => (UINode, UIMaterial, UIText, UITextInput, Interactable, UIFocusable));

impl Themed for TextField {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for TextField {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for TextField {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Typography for TextField {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}
```

- [ ] **Step 4: Create `crates/ui/src/elements/chip.rs`**

```rust
//! Small bordered toggles, such as filter tags.
use super::{Layout, Themed, Typography};
use crate::{
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
};
use concerto_color::Color;

/// A small bordered label; its colours come from [`UITheme::chip_colors`] at spawn.
pub struct Chip {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    text: UIText,
    selected: bool,
}

impl UITheme {
    pub fn chip(&self, label: impl Into<String>) -> Chip {
        Chip {
            theme: self.clone(),
            node: UINode::default()
                .with_flex_shrink(0.0)
                .with_padding(UIRect::axes(2.0, 7.0)),
            material: UIMaterial {
                corner_radius: self.radius_sm,
                ..UIMaterial::with_border(Color::TRANSPARENT, self.border, 1.0)
            },
            text: UIText {
                font_size: self.font_size_sm,
                line_height: self.line_height(self.font_size_sm),
                wrap: false,
                ..self.body_text(label)
            },
            selected: false,
        }
    }
}

impl Chip {
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn into_parts(self) -> (UINode, UIMaterial, UIText, Interactable) {
        let colors = self.theme.chip_colors(self.selected);
        let material = UIMaterial {
            color: colors.fill.to_linear(),
            border_color: colors.border.to_linear(),
            ..self.material
        };
        let text = UIText {
            color: colors.text,
            ..self.text
        };
        (self.node, material, text, Interactable)
    }
}

bundle!(Chip => (UINode, UIMaterial, UIText, Interactable));

impl Themed for Chip {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Chip {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Typography for Chip {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}
```

- [ ] **Step 5: Create `crates/ui/src/elements/controls.rs`**

```rust
//! Checkboxes and sliders in the theme's colours.
use super::{Layout, Shape, Themed, Typography};
use crate::{
    checkbox::UICheckbox,
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    slider::UISlider,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

/// A labelled checkbox that fills with the accent when checked.
pub struct Checkbox {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    checkbox: UICheckbox,
    text: UIText,
}

/// A horizontal value slider.
pub struct Slider {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    slider: UISlider,
}

impl UITheme {
    pub fn checkbox(&self, label: impl Into<String>, checked: bool) -> Checkbox {
        Checkbox {
            theme: self.clone(),
            node: UINode::default()
                .with_height(UIValue::Px(self.control_height))
                .with_flex_shrink(0.0)
                .with_padding(UIRect::axes(self.spacing_xs, self.spacing_sm)),
            material: UIMaterial::with_border(self.surface_raised, self.border, 1.0),
            checkbox: UICheckbox {
                checked,
                checked_color: self.accent,
                unchecked_color: self.surface_raised,
            },
            text: UIText {
                wrap: false,
                ..self.body_text(label)
            },
        }
    }

    pub fn slider(&self, value: f32, min: f32, max: f32) -> Slider {
        Slider {
            theme: self.clone(),
            node: UINode::default().with_height(UIValue::Px(self.control_height)),
            material: UIMaterial::flat(self.surface_raised),
            slider: UISlider::new(value, min, max),
        }
    }
}

impl Checkbox {
    fn into_parts(self) -> (UINode, UIMaterial, UICheckbox, Interactable, UIText) {
        (self.node, self.material, self.checkbox, Interactable, self.text)
    }
}

impl Slider {
    fn into_parts(self) -> (UINode, UIMaterial, UISlider, Interactable) {
        (self.node, self.material, self.slider, Interactable)
    }
}

bundle!(Checkbox => (UINode, UIMaterial, UICheckbox, Interactable, UIText));
bundle!(Slider => (UINode, UIMaterial, UISlider, Interactable));

impl Themed for Checkbox {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Checkbox {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Checkbox {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Typography for Checkbox {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}

impl Themed for Slider {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Slider {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Slider {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}
```

`UICheckbox`'s fields are `pub`; `UISlider::new(value, min, max)` is its public constructor (its `dragging` field is crate-private). `UITextInput::placeholder` is a `pub` field.

- [ ] **Step 6: Register them** — in `elements/mod.rs`: `mod chip; mod controls; mod field;` and `pub use chip::Chip; pub use controls::{Checkbox, Slider}; pub use field::TextField;`.

- [ ] **Step 7: Run the tests** — `cargo test -p concerto-ui --test elements` → PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/elements crates/ui/tests/elements.rs
git commit -m "Add themed text field, chip, checkbox and slider builders"
```

---

### Task 5: Anchored popups

**Files:**
- Create: `crates/ui/src/elements/popup.rs`
- Modify: `crates/ui/src/elements/mod.rs`, `crates/ui/tests/elements.rs`

**Interfaces:**
- Consumes: `Surface` (`pub(crate)` fields `theme`, `node`, `material`; `pub(crate) into_parts`) and `UITheme::popup` from Task 2; `crate::anchor::{UIAnchoredPanel, UIAnchorTarget, UIAnchorSide, UIAnchorAlign}`.
- Produces: `Popup` — parts `(UINode, UIMaterial, UIAnchoredPanel)`; `Themed`, `Layout`, `Shape`; inherent `fill`, `anchor_side`, `anchor_align`, `anchor_gap`, `anchor_target`, `focus_on_open`, `open`. `UITheme::dropdown(&self, Entity) -> Popup`, `UITheme::context_menu(&self) -> Popup`.

- [ ] **Step 1: Write the failing tests** (append; add `anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel}` to imports)

```rust
#[test]
fn a_dropdown_hangs_below_its_trigger_and_toggles_with_it() {
    let t = theme();
    let mut world = World::default();
    let trigger = world.spawn(Marker);
    let entity = world.spawn(t.dropdown(trigger));
    let panel = world.get_component_for_entity::<UIAnchoredPanel>(entity).unwrap();
    assert_eq!(panel.target, UIAnchorTarget::Node { entity: trigger });
    assert_eq!(panel.owner, Some(trigger));
    assert!(panel.toggled_by_owner);
    assert_eq!(panel.side, UIAnchorSide::Below);
    assert_eq!(panel.align, UIAnchorAlign::Start);
    assert_eq!(panel.gap, t.spacing_xs);
    assert!(!panel.open);
    let material = world.get_component_for_entity::<UIMaterial>(entity).unwrap();
    assert_eq!(material.color, t.surface_raised.to_linear());
    assert_eq!(material.border_width, 1.0);
}

#[test]
fn popup_anchor_modifiers_set_the_panel() {
    let t = theme();
    let mut world = World::default();
    let trigger = world.spawn(Marker);
    let search = world.spawn(Marker);
    let entity = world.spawn(
        t.dropdown(trigger)
            .anchor_side(UIAnchorSide::Above)
            .anchor_align(UIAnchorAlign::End)
            .anchor_gap(6.0)
            .focus_on_open(search)
            .open(true)
            .fill(t.canvas)
            .width(UIValue::Px(272.0)),
    );
    let panel = world.get_component_for_entity::<UIAnchoredPanel>(entity).unwrap();
    assert_eq!(panel.side, UIAnchorSide::Above);
    assert_eq!(panel.align, UIAnchorAlign::End);
    assert_eq!(panel.gap, 6.0);
    assert_eq!(panel.focus_on_open, Some(search));
    assert!(panel.open);
    assert_eq!(
        world.get_component_for_entity::<UIMaterial>(entity).unwrap().color,
        t.canvas.to_linear()
    );
    assert_eq!(
        world.get_component_for_entity::<UINode>(entity).unwrap().width,
        UIValue::Px(272.0)
    );
}

#[test]
fn a_context_menu_floats_at_a_point_with_no_owner() {
    let t = theme();
    let spawned = spawn((
        t.context_menu().anchor_target(UIAnchorTarget::Point { position: Vec2::new(4.0, 5.0) }),
        t.text("Rename"),
    ));
    let panel = spawned.get::<UIAnchoredPanel>();
    assert_eq!(panel.target, UIAnchorTarget::Point { position: Vec2::new(4.0, 5.0) });
    assert_eq!(panel.owner, None);
    assert!(!panel.toggled_by_owner);
    assert_eq!(spawned.get::<UIText>().text, "Rename");
}
```

If `UIAnchorTarget` does not derive `Debug`, `assert_eq!` on it will fail to compile; in that case replace those two `assert_eq!`s with `assert!(panel.target == ...)`.

- [ ] **Step 2: Run to verify failure** — compile errors for `dropdown`/`context_menu`.

- [ ] **Step 3: Create `crates/ui/src/elements/popup.rs`**

```rust
//! Floating menus anchored to a node or a point.
use concerto_color::Color;
use concerto_ecs::entity::Entity;

use super::{Layout, Shape, Surface, Themed};
use crate::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    material::UIMaterial,
    node::UINode,
    theme::UITheme,
};

/// A popup surface with its anchoring.
///
/// Anchored panels must be UI roots: spawn a `Popup` with `cmd.spawn(..)`, never `add_child`.
pub struct Popup {
    surface: Surface,
    panel: UIAnchoredPanel,
}

impl UITheme {
    /// A menu below `trigger`, opened and closed by clicking it.
    pub fn dropdown(&self, trigger: Entity) -> Popup {
        Popup {
            surface: self.popup(),
            panel: UIAnchoredPanel::new(UIAnchorTarget::from_node(trigger))
                .with_owner(trigger)
                .toggled_by_owner()
                .with_gap(self.spacing_xs),
        }
    }

    /// A menu at a point; the caller sets its target and opens it.
    pub fn context_menu(&self) -> Popup {
        Popup {
            surface: self.popup(),
            panel: UIAnchoredPanel::default().with_gap(self.spacing_xs),
        }
    }
}

impl Popup {
    pub fn fill(mut self, color: Color) -> Self {
        self.surface = self.surface.fill(color);
        self
    }

    pub fn anchor_side(mut self, side: UIAnchorSide) -> Self {
        self.panel = self.panel.with_side(side);
        self
    }

    pub fn anchor_align(mut self, align: UIAnchorAlign) -> Self {
        self.panel = self.panel.with_align(align);
        self
    }

    pub fn anchor_gap(mut self, gap: f32) -> Self {
        self.panel = self.panel.with_gap(gap);
        self
    }

    pub fn anchor_target(mut self, target: UIAnchorTarget) -> Self {
        self.panel = self.panel.with_target(target);
        self
    }

    pub fn focus_on_open(mut self, widget: Entity) -> Self {
        self.panel = self.panel.with_focus_on_open(widget);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.panel = self.panel.with_open(open);
        self
    }

    fn into_parts(self) -> (UINode, UIMaterial, UIAnchoredPanel) {
        let (node, material) = self.surface.into_parts();
        (node, material, self.panel)
    }
}

bundle!(Popup => (UINode, UIMaterial, UIAnchoredPanel));

impl Themed for Popup {
    fn theme(&self) -> &UITheme {
        &self.surface.theme
    }
}

impl Layout for Popup {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.surface.node
    }
}

impl Shape for Popup {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.surface.material
    }
}
```

Check `UIAnchoredPanel::with_open` exists (`crates/ui/src/anchor.rs:166`) — it does.

- [ ] **Step 4: Register it** — `mod popup;` and `pub use popup::Popup;` in `elements/mod.rs`.

- [ ] **Step 5: Run the tests** — `cargo test -p concerto-ui` → PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/elements crates/ui/tests/elements.rs
git commit -m "Add themed dropdown and context menu builders"
```

---

### Task 6: Migrate the editor shell, dock, tabs and chrome

**Files:**
- Modify: `crates/editor/src/fonts.rs:29-39`, `crates/editor/src/dock.rs:133-139,227-264`, `crates/editor/src/shell.rs:7-18,51-58,60-183`, `crates/editor/src/tabs.rs:8-16,96,149-159,166-241`, `crates/editor/src/window_chrome.rs:117-160`, `crates/editor/src/workspace.rs:40-57`, `crates/editor/src/diagnostics.rs:56-67`

**Interfaces:**
- Consumes: every builder from Tasks 1–5; `concerto_ui::elements::{Text, prelude::*}`; `concerto_ui::theme::ButtonVariant`.
- Produces: `crate::fonts::icon(theme: &UITheme, glyph: char, size: f32) -> concerto_ui::elements::Text` (was `UIText`) — callers in this task switch from `UIText { color, ..icon(..) }` to `icon(..).muted()` / `.color(..)`.

This task is behaviour-preserving (apart from body text colour); the regression net is the existing editor tests.

- [ ] **Step 1: Run the editor tests for a green baseline**

Run: `cargo test -p concerto-editor`
Expected: PASS. Record any pre-existing failures; they must not grow.

- [ ] **Step 2: `fonts.rs` — `icon` returns a `Text` builder**

```rust
use concerto_ui::elements::{Text, prelude::*};
use concerto_ui::text::FontFamily;
use concerto_ui::theme::UITheme;

/// A text node holding one icon glyph.
pub fn icon(theme: &UITheme, glyph: char, size: f32) -> Text {
    theme
        .text(glyph)
        .family(FontFamily::Name(PHOSPHOR.into()))
        .font_size(size)
        .no_wrap()
}
```

Drop the now-unused `UIText` import.

- [ ] **Step 3: `dock.rs`**

Root (in `build_dock`):

```rust
let root = cmd
    .spawn(
        theme
            .canvas()
            .size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .clipped(),
    )
    .entity();
```

`fill_region` loop body:

```rust
for panel in panels {
    let mut container_queue = cmd.entity(container);
    let mut body_queue = if region.is_card() {
        container_queue.spawn_child_queue(
            theme
                .panel()
                .grow()
                .column()
                .padding(theme.spacing_md)
                .clipped(),
        )
    } else {
        container_queue.spawn_child_queue(
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_direction(FlexDirection::Column)
                .clipped(),
        )
    };
    let body = body_queue.entity();
    if region != Region::Scene {
        body_queue.insert(Interactable);
    }
    if region.is_card() {
        body_queue.add_child(
            theme
                .label(panel.title.to_uppercase())
                .small()
                .muted()
                .weight(crate::fonts::MEDIUM)
                .height(UIValue::Px(16.0))
                .fixed(),
        );
    }
    bodies.push((panel.id, body));
}
```

Add `use concerto_ui::elements::prelude::*;`; drop `UIMaterial`/`UIRect`/`UIText` imports if unused.

- [ ] **Step 4: `shell.rs`**

Delete the private `fn text(..)`. Replace `use crate::marks::TRANSPARENT;` with `use concerto_color::Color;` (check `concerto-color` is in `crates/editor/Cargo.toml` — `window_chrome.rs` already imports it). Add `use concerto_ui::elements::prelude::*;`.

Brand row:

```rust
cmd.entity(brand).add_child_with(theme.row().grow(), |mut row| {
    row = row.add_child_with(
        theme
            .canvas()
            .fill(Color::TRANSPARENT)
            .border(theme.accent, 1.5)
            .radius(6.0)
            .size(UIValue::Px(13.0), UIValue::Px(19.0))
            .fixed()
            .align_items(taffy::AlignItems::Center)
            .padding(UIRect {
                top: 4.0,
                ..Default::default()
            }),
        |mark| {
            mark.add_child(
                theme
                    .canvas()
                    .fill(theme.accent)
                    .radius(2.0)
                    .size(UIValue::Px(4.0), UIValue::Px(4.0))
                    .fixed(),
            );
        },
    );

    row = row.add_child(theme.label("Concerto").weight(MEDIUM));

    // the tab strip `row.add_child_with(..)` block stays exactly as it is
});
```

Chatter strip:

```rust
cmd.entity(chatter).add_child_with(
    theme
        .panel()
        .radius_md()
        .height(UIValue::Px(30.0))
        .fixed()
        .align_items(taffy::AlignItems::Center)
        .padding(UIRect::axes(0.0, theme.spacing_md))
        .clipped(),
    |strip| {
        strip
            .add_child((
                UINode::default()
                    .with_width(UIValue::Px(18.0))
                    .with_flex_shrink(0.0),
                icon(&theme, glyph::INFO, theme.font_size_md).muted(),
                Label::ChatterGlyph,
            ))
            .add_child((
                theme.label("").muted().single_line().grow(),
                Label::Chatter,
            ));
    },
);
```

`refresh_chrome` is unchanged. Drop now-unused imports (`UIMaterial`, `UIText` only if no longer referenced — `refresh_chrome` still queries `&mut UIText`, so keep it).

- [ ] **Step 5: `tabs.rs`**

Add `use concerto_ui::{elements::prelude::*, theme::ButtonVariant};`.

In the refresh system change the query `tab_buttons: Query<(&EditorTab, &mut UIMaterial, &mut UIInteractionStyle)>` to `tab_buttons: Query<(&EditorTab, &mut UIInteractionStyle)>` and the loop to:

```rust
for (tab, mut interaction) in tab_buttons.iter() {
    if tab.document == entity {
        let style = theme.interaction(ButtonVariant::Tab, is_active);
        if interaction.normal != style.normal {
            *interaction = style;
        }
    }
}
```

(`apply_interaction_styles` drives `UIMaterial::color` from the style every frame, so the material write was redundant.)

Tab spawn:

```rust
let ink = if is_active { theme.text } else { theme.text_muted };
cmd.entity(content).add_child_with(
    (
        theme
            .pressable()
            .tab()
            .selected(is_active)
            .radius(0.0)
            .height(UIValue::Px(30.0))
            .row()
            .padding(UIRect::axes(0.0, theme.spacing_sm))
            .z_index(70),
        EditorTab { document: entity },
        WindowChromeControl,
    ),
    |button| {
        button
            .add_child((
                UINode::default()
                    .with_width(UIValue::Px(16.0))
                    .with_flex_shrink(0.0),
                icon(&theme, mark, theme.font_size_sm).color(ink),
            ))
            .add_child((
                theme
                    .label(title.clone())
                    .single_line()
                    .color(ink)
                    .max_width(UIValue::Px(220.0)),
                TabLabel { document: entity },
            ))
            .add_child((
                UINode::default()
                    .with_size(UIValue::Px(18.0), UIValue::Px(24.0))
                    .with_padding(UIRect::axes(3.0, 3.0))
                    .with_flex_shrink(0.0)
                    .with_z_index(71),
                icon(&theme, glyph::X, theme.font_size_sm).muted(),
                Interactable,
                EditorTabClose { document: entity },
                WindowChromeControl,
            ));
    },
);
```

Drop `UIMaterial` from imports if unused.

- [ ] **Step 6: `window_chrome.rs`**

```rust
let mut bar = body_queue.spawn_child_queue(
    theme.row().gap(2.0).padding(UIRect::axes(0.0, theme.spacing_md)),
);

for (control, mark) in [ /* unchanged */ ] {
    bar = bar.add_child_with(
        (
            theme
                .pressable()
                .pressed(match control {
                    Control::Close => theme.error,
                    _ => theme.accent,
                })
                .size(UIValue::Px(30.0), UIValue::Px(26.0))
                .padding(UIRect::axes(3.0, 8.0))
                .z_index(CONTROL_LAYER),
            icon(&theme, mark, theme.font_size_lg).muted(),
            control,
            WindowChromeControl,
        ),
        |mut button| {
            if matches!(control, Control::Maximise) {
                button.insert(MaximiseGlyph);
            }
        },
    );
}
```

Add `use concerto_ui::elements::prelude::*;`; drop `Color`, `UIMaterial`, `UIInteractionStyle`, `UIText` imports if no longer referenced elsewhere in the file (check with `grep`).

- [ ] **Step 7: `workspace.rs`**

```rust
let mut node = node.clone();
node.visible = false;
let surface = if index == 0 {
    theme.canvas()
} else {
    theme.panel()
};
let host = commands
    .entity(parent.parent())
    .spawn_child_queue((
        surface.node(|_| node),
        EditorOwned(editor),
        Interactable,
    ))
    .entity();
```

Add `use concerto_ui::elements::prelude::*;`; drop `UIMaterial` import if unused.

- [ ] **Step 8: `diagnostics.rs`**

```rust
panel.add_child((
    theme.label("").small().muted().mono().grow(),
    Readout,
));
```

Drop `FontFamily`/`UIText` imports if unused (the refresh system likely still queries `UIText`).

- [ ] **Step 9: Build, test, lint**

Run: `cargo test -p concerto-editor` → same result as Step 1 (in particular `tests/tabs.rs::activation_updates_style_and_closed_documents_remove_buttons` PASS).
Run: `cargo clippy -p concerto-editor --all-targets` → no new warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/editor/src/fonts.rs crates/editor/src/dock.rs crates/editor/src/shell.rs crates/editor/src/tabs.rs crates/editor/src/window_chrome.rs crates/editor/src/workspace.rs crates/editor/src/diagnostics.rs
git commit -m "Build editor shell, dock, tabs and chrome from themed elements"
```

---

### Task 7: Migrate the content and hierarchy panels

**Files:**
- Modify: `crates/editor/src/content.rs:10-26,110-279,394-453`, `crates/editor/src/hierarchy.rs:6,17-27,118-125,147-266,587-628`

**Interfaces:**
- Consumes: Tasks 1–5 builders; `UITheme::{selection, interaction, chip_colors}`; `ButtonVariant`.
- Produces: nothing new. After this task `content.rs` and `hierarchy.rs` no longer import `marks::{TRANSPARENT, selection_tint}` (they keep `marks::{self, Mark}`).

- [ ] **Step 1: Baseline** — `cargo test -p concerto-editor` → PASS.

- [ ] **Step 2: `content.rs` spawn code**

Delete `fn text(..)`. Change the marks import to `use crate::marks::{self, Mark};`, add `use concerto_color::Color; use concerto_ui::{elements::prelude::*, theme::ButtonVariant};`.

In `build_panel`, keep the outer panel `UINode` as is. Search pill:

```rust
panel = panel.add_child_with(
    theme
        .card()
        .radius_sm()
        .height(UIValue::Px(30.0))
        .fixed()
        .row()
        .padding(UIRect::axes(0.0, 8.0))
        .gap(6.0),
    |search| {
        search
            .add_child(theme.label("⌕").muted().no_wrap())
            .add_child((
                theme
                    .text_field("Find imported assets…")
                    .bare()
                    .height(UIValue::Auto)
                    .padding(0.0)
                    .single_line()
                    .grow()
                    .shrink(1.0)
                    .min_width(UIValue::Px(0.0)),
                Filter,
            ));
    },
);
```

Kind tags (keep the container `UINode` as is):

```rust
tags = tags.add_child((theme.chip(*name), Action::Kind(index), Tag(index)));
```

Pooled rows:

```rust
pool = pool.add_child_with(
    (
        theme
            .pressable()
            .height(UIValue::Px(ROW_HEIGHT))
            .row()
            .padding(UIRect::axes(4.0, 6.0))
            .gap(8.0),
        Action::Asset(slot),
    ),
    |row| {
        let (mark_node, mark_material) = marks::node();
        row.add_child((mark_node, mark_material, MarkSlot(slot)))
            .add_child((
                theme.label("").single_line().grow().shrink(1.0),
                Label::Asset(slot),
            ));
    },
);
```

Count line:

```rust
panel.add_child((
    theme
        .label("")
        .height(UIValue::Px(26.0))
        .fixed()
        .padding(UIRect::axes(4.0, 10.0)),
    Label::Count,
));
```

- [ ] **Step 3: `content.rs` refresh systems**

`render_marks`, the style loop:

```rust
for (action, mut style) in styles.iter() {
    let Action::Asset(slot) = *action else {
        continue;
    };
    let wanted = theme.interaction(ButtonVariant::Ghost, selected(slot));
    if style.normal != wanted.normal {
        *style = wanted;
    }
}
```

`render_tags`:

```rust
for (tag, mut material, mut text) in tags.iter() {
    let colors = theme.chip_colors(tag.0 == state.kind);
    let fill = colors.fill.to_linear();
    if material.color != fill {
        material.color = fill;
    }
    let border = colors.border.to_linear();
    if material.border_color != border {
        material.border_color = border;
    }
    if text.color != colors.text {
        text.color = colors.text;
    }
}
```

Drop `UIInteractionStyle`/`UIMaterial`/`UIText` imports only if no longer referenced (`render_tags` and `refresh_panel` still use them).

- [ ] **Step 4: `hierarchy.rs`**

Delete `fn text(..)` (keep `line` and `icon_column`). Imports: `use crate::marks::{self, Mark};`, add `use concerto_color::Color; use concerto_ui::elements::prelude::*;`.

`spawn_panel`:

```rust
cmd.entity(parent).add_child_with(
    (
        theme
            .panel()
            .radius(0.0)
            .width(UIValue::Percent(100.0))
            .grow()
            .column()
            .padding(8.0)
            .clipped(),
        Interactable,
        TreeRegion,
    ),
    |mut tree| {
        tree = tree
            .add_child((line(42.0), theme.text("WORLD"), Label::Title))
            .add_child((
                theme
                    .text_field("Search entities…")
                    .height(UIValue::Px(38.0))
                    .padding(UIRect::axes(6.0, 8.0)),
                Filter,
            ));

        // tree view: the `UINode` + Interactable + TreeRegion + TreeView + UIVirtualList tuple stays as is
        // inside it, each pooled row becomes:
        pool = pool.add_child_with(
            (
                theme
                    .canvas()
                    .fill(Color::TRANSPARENT)
                    .radius_sm()
                    .height(UIValue::Px(ROW_HEIGHT))
                    .fixed()
                    .row(),
                RowSlot(slot),
            ),
            |row| {
                let (mark_node, mark_material) = marks::node();
                row.add_child((
                    icon_column(20.0),
                    UIText {
                        line_height: theme.line_height(theme.font_size_md),
                        ..theme.text("").muted().font_size(9.0).no_wrap().into()
                    },
                    Interactable,
                    TreeRegion,
                    Action::Toggle(slot),
                    Label::Toggle(slot),
                ))
                .add_child((mark_node, mark_material, MarkSlot(slot)))
                .add_child((
                    line(ROW_HEIGHT).with_flex_grow(1.0).with_flex_shrink(1.0),
                    theme.text("").single_line(),
                    Interactable,
                    TreeRegion,
                    Action::Select(slot),
                    Label::Row(slot),
                ))
                .add_child((
                    theme
                        .label("")
                        .muted()
                        .mono()
                        .font_size(10.0)
                        .no_wrap()
                        .fixed()
                        .align_self(taffy::AlignItems::Center)
                        .margin(UIRect::axes(0.0, 8.0)),
                    Label::Count(slot),
                ));
            },
        );

        tree.add_child((line(46.0), theme.text(""), Label::Position));
    },
);
```

The toggle keeps its 9px glyph on the medium line height (today's layout) via `From<Text> for UIText` plus struct update. The row's `.row()` adds `align_items: Center`; every row child has an explicit row-height or centres itself, so nothing moves.

`render_marks`:

```rust
let color = if selected_row(slot.0) {
    theme.selection()
} else {
    Color::TRANSPARENT
}
.to_linear();
```

- [ ] **Step 5: Build, test, lint** — `cargo test -p concerto-editor` → PASS; `cargo clippy -p concerto-editor --all-targets` → no new warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/editor/src/content.rs crates/editor/src/hierarchy.rs
git commit -m "Build content and hierarchy panels from themed elements"
```

---

### Task 8: Migrate the inspector, marks and the custom property example

**Files:**
- Modify: `crates/editor/src/inspector/mod.rs:128-179`, `crates/editor/src/inspector/sync.rs:103-237`, `crates/editor/src/inspector/numeric.rs:144-176`, `crates/editor/src/inspector/add_component.rs` (whole spawn half, lines 1-358), `crates/editor/src/marks.rs:39-45,65,129`, `crates/editor/examples/custom_property.rs:59-83`

**Interfaces:**
- Consumes: Tasks 1–5 builders.
- Produces: `crate::marks` no longer exports `TRANSPARENT` or `selection_tint` (callers use `Color::TRANSPARENT` and `theme.selection()`).

- [ ] **Step 1: Baseline** — `cargo test -p concerto-editor` → PASS.

- [ ] **Step 2: `inspector/mod.rs`**

Delete `fn text(..)`. Add `use concerto_ui::elements::prelude::*;`. `spawn_panel`:

```rust
cmd.entity(parent).add_child_with(theme.column().grow().clipped(), |details| {
    details.add_child_with(
        (
            UINode::default()
                .with_flex_grow(1.0)
                .with_flex_direction(FlexDirection::Column)
                .clipped(),
            Interactable,
            DetailsView,
        ),
        |mut view| {
            let stack = view
                .spawn_child_queue((
                    theme.column().fixed().gap(theme.spacing_xs + 2.0),
                    ComponentStack::default(),
                ))
                .entity();

            view.insert(UIScrollArea {
                content: Some(stack),
                ..Default::default()
            });
        },
    );
});
```

- [ ] **Step 3: `inspector/sync.rs`**

`spawn_card`:

```rust
let mut stack_queue = cmd.entity(stack);
let mut card_queue = stack_queue.spawn_child_queue(
    theme
        .card()
        .column()
        .fixed()
        .padding(UIRect::axes(theme.spacing_xs + 2.0, theme.spacing_sm)),
);
let entity = card_queue.entity();

card_queue = card_queue.add_child_with(
    theme.row().fixed().gap(theme.spacing_xs + 2.0),
    |header| {
        header
            .add_child(theme.label(name).single_line().grow())
            .add_child((
                theme
                    .canvas()
                    .fill(theme.accent)
                    .radius(6.5)
                    .size(UIValue::Px(22.0), UIValue::Px(13.0))
                    .fixed(),
                UIDisabled,
            ));
    },
);
```

`spawn_row`:

```rust
let row_queue = body_queue.spawn_child_queue((
    theme.row().fixed().gap(theme.spacing_xs),
    target,
    property.value,
    BuildPropertyWidget,
));
let row = row_queue.entity();
row_queue.add_child(
    theme
        .label(label)
        .small()
        .muted()
        .single_line()
        .width(UIValue::Px(PROPERTY_LABEL_WIDTH))
        .fixed(),
);
```

`build_property_widgets` fallback:

```rust
cmd.entity(entity).add_child(theme.label("Unsupported type"));
```

`body_node` stays as it is (it returns a bare `UINode` that is re-inserted).

- [ ] **Step 4: `inspector/numeric.rs`** — `build`:

```rust
row_queue = row_queue.add_child((
    theme
        .text_field("")
        .small()
        .grow()
        .shrink(1.0)
        .width(UIValue::Px(0.0))
        .min_width(UIValue::Px(0.0))
        .padding(UIRect::axes(field_leading(theme), theme.spacing_xs))
        .clipped(),
    NumericSlot {
        row,
        slot,
        displayed: String::new(),
    },
));
```

`.shrink(1.0)` restores the field's original shrink (a `TextField` is fixed by default). Add `use concerto_ui::elements::prelude::*;`, drop unused imports (`UIMaterial`, `UIFocusable`, `Interactable`, `UITextInput` are still used elsewhere in the file — check each with grep before removing).

- [ ] **Step 5: `inspector/add_component.rs`**

Imports: remove `use crate::marks::{TRANSPARENT, selection_tint};` and `FontFamily`; add `use concerto_ui::elements::prelude::*;`. Delete `divider`, `sized`, `muted`, `mono`.

`spawn_row`:

```rust
fn spawn_row(cmd: &mut CommandQueue, stack: Entity, theme: &UITheme) -> Entity {
    cmd.entity(stack)
        .spawn_child_queue(
            theme
                .pressable()
                .bordered()
                .radius_md()
                .height(UIValue::Px(ROW_HEIGHT))
                .row()
                .gap(7.0)
                .padding(UIRect::axes(0.0, 9.0)),
        )
        .add_child(theme.label("⌕").muted().font_size(12.0).no_wrap())
        .add_child(
            theme
                .label("add component")
                .muted()
                .font_size(12.0)
                .no_wrap()
                .grow(),
        )
        .add_child(theme.label("⌘⇧A").muted().mono().font_size(10.0).no_wrap())
        .entity()
}
```

`spawn_menu`:

```rust
fn spawn_menu(cmd: &mut CommandQueue, row: Entity, theme: &UITheme) {
    let mut menu = cmd.spawn(theme.popup().fill(MENU_FILL).width(UIValue::Px(MENU_WIDTH)));

    let mut header = menu
        .spawn_child_queue(theme.row().fixed().gap(7.0).padding(UIRect::axes(9.0, 10.0)))
        .add_child(theme.label("⌕").muted().font_size(12.5).no_wrap());
    let search = header
        .spawn_child_queue((
            theme
                .text_field("Search components…")
                .bare()
                .height(UIValue::Auto)
                .padding(0.0)
                .font_size(12.5)
                .single_line()
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0)),
            AddComponentSearch,
        ))
        .entity();
    let count = header
        .spawn_child_queue(
            theme
                .label("")
                .muted()
                .mono()
                .font_size(9.5)
                .no_wrap()
                .fixed(),
        )
        .entity();

    menu = menu.add_child(theme.divider());
    let list = menu
        .spawn_child_queue(
            theme
                .column()
                .gap(1.0)
                .max_height(UIValue::Px(MENU_LIST_MAX_HEIGHT))
                .padding(6.0)
                .clipped(),
        )
        .entity();
    menu = menu.add_child(theme.divider());
    menu = menu.add_child_with(
        theme.row().fixed().gap(10.0).padding(UIRect::axes(7.0, 10.0)),
        |footer| {
            footer
                .add_child(theme.label("↑↓ move").muted().mono().font_size(9.5).no_wrap())
                .add_child(theme.label("↵ add").muted().mono().font_size(9.5).no_wrap())
                .add_child(UINode::default().with_flex_grow(1.0))
                .add_child(
                    theme
                        .label("⇧↵ add & open")
                        .muted()
                        .mono()
                        .font_size(9.5)
                        .no_wrap(),
                );
        },
    );

    menu.insert((
        menu_panel(row, search),
        AddComponentMenu {
            search,
            count,
            list,
        },
    ));
}
```

`menu_panel` stays (it re-anchors the persistent menu by re-inserting only the `UIAnchoredPanel`, and needs `search`, which exists only after spawning).

`populate_add_component_menu`:

```rust
cmd.insert(
    theme.text(&count).muted().mono().font_size(9.5).no_wrap(),
    menu.count,
);
// …
cmd.entity(menu.list).add_child(
    theme
        .label(&message)
        .muted()
        .font_size(12.0)
        .no_wrap()
        .padding(UIRect::axes(6.0, 8.0)),
);
```

`spawn_entry`:

```rust
fn spawn_entry(cmd: &mut CommandQueue, list: Entity, name: &str, highlighted: bool, theme: &UITheme) {
    let mut list = cmd.entity(list);
    let row = list
        .spawn_child_queue(
            theme
                .pressable()
                .selected(highlighted)
                .row()
                .gap(8.0)
                .padding(UIRect::axes(6.0, 8.0)),
        )
        .add_child(
            theme
                .canvas()
                .fill(theme.accent)
                .rotation(std::f32::consts::FRAC_PI_4)
                .size(UIValue::Px(MARK_SIZE), UIValue::Px(MARK_SIZE))
                .fixed(),
        )
        .add_child(
            theme
                .label(name)
                .font_size(12.5)
                .single_line()
                .color(if highlighted { theme.accent_hovered } else { theme.text })
                .grow()
                .min_width(UIValue::Px(0.0)),
        );
    if highlighted {
        row.add_child(theme.label("↵").muted().mono().font_size(9.5).no_wrap());
    }
}
```

- [ ] **Step 6: `marks.rs`** — delete `pub const TRANSPARENT` and `pub fn selection_tint`; replace the two remaining in-file uses of `TRANSPARENT` (`Shape` `plain.fill`/`border` defaults and `node()`'s `UIMaterial::flat(TRANSPARENT)`) with `Color::TRANSPARENT`. Confirm nothing else references them: `grep -rn "selection_tint\|marks::TRANSPARENT\|marks::{.*TRANSPARENT" crates/editor` → no matches.

- [ ] **Step 7: `crates/editor/examples/custom_property.rs`** — `build`:

```rust
let button = cmd
    .spawn((
        theme
            .label(label(snapshot))
            .height(UIValue::Px(theme.control_height))
            .grow(),
        Interactable,
        SettingButton(row),
    ))
    .entity();
cmd.add_child(row, button);
```

Add `use concerto_ui::elements::prelude::*;`; drop `UINode`/`UIText` imports if unused.

- [ ] **Step 8: Build, test, lint** — `cargo test -p concerto-editor` → PASS (including `inspector/tests.rs` and `tests/custom_property.rs`); `cargo build -p concerto-editor --examples`; `cargo clippy -p concerto-editor --all-targets` → no new warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/editor/src/inspector crates/editor/src/marks.rs crates/editor/examples/custom_property.rs
git commit -m "Build the inspector from themed elements and fold selection tint into the theme"
```

---

### Task 9: Migrate the showcase, delete the superseded helpers, verify

**Files:**
- Modify: `examples/ui-showcase/src/main.rs` (whole spawn function and helpers, lines 1-535), `crates/ui/src/widgets.rs:74-99`, `crates/ui/src/text/mod.rs:14-16,60-69`

**Interfaces:**
- Consumes: all builders.
- Produces: `widgets::button` and `UIText::from_theme` no longer exist.

- [ ] **Step 1: Delete `widgets::button` and `UIText::from_theme`**

Remove `pub fn button(..)` from `crates/ui/src/widgets.rs` and the now-unused `Color`, `Interactable`, `UIInteractionStyle`, `UIMaterial`, `UIRect`, `UITheme`, `UIValue` imports there (keep what `update_widgets`/`sync_tab_bodies`/`update_tooltips` still use). Remove `impl UIText { pub fn from_theme .. }` and the `theme::UITheme` import from `crates/ui/src/text/mod.rs`.

Run: `grep -rn "from_theme\|widgets::button" crates examples --include=*.rs` → only the showcase matches, which Step 2 fixes.

- [ ] **Step 2: Rewrite the showcase spawn code**

Imports: add `elements::prelude::*` inside the `ui::{..}` list; remove `widgets`, `UIInteractionStyle`, `UICheckbox`, `UISlider`, `UITextInput`, `UIRect` (if unused), `UIAnchoredPanel` is still used by `drive_panels`/`update_diagnostics` — keep it, and `UIAnchorSide` for the submenu.

Delete `fn label`, `fn panel`, `fn menu_panel`. Rewrite `menu_row`:

```rust
fn menu_row(cmd: &mut CommandQueue, theme: &UITheme, panel: Entity, name: &str) -> Entity {
    let row = cmd
        .spawn((
            theme
                .pressable()
                .solid()
                .radius(0.0)
                .height(UIValue::Px(theme.row_height))
                .padding(UIRect::axes(theme.spacing_xs, theme.spacing_sm)),
            theme.text(name).font_size(12.0),
        ))
        .entity();
    cmd.add_child(panel, row);
    row
}
```

Then, in `spawn_showcase`, replace each spawn with (keep the surrounding `cmd.add_child(parent, child)` lines and all non-visual components such as `UIScrollArea`, `UIVirtualList`, `UISplitPane`, `UISplitHandle`, `VirtualRow`, `ContextMenu`, `Diagnostics`):

| Entity | New bundle |
|---|---|
| `root` | `theme.canvas().size(UIValue::Percent(100.0), UIValue::Percent(100.0)).column().gap(theme.spacing_md).padding(theme.spacing_lg)` |
| `heading` | `theme.label("CONCERTO  /  UI SHOWCASE\nLooking Glass foundations and layout").font_size(18.0).height(UIValue::Px(54.0)).fixed()` |
| `body` | `theme.row().grow().min_height(UIValue::Px(320.0)).gap(theme.spacing_md).align_items(AlignItems::Stretch)` |
| `foundations` | `theme.panel().radius(0.0).bordered().size(UIValue::Percent(46.0), UIValue::Percent(100.0)).column().gap(8.0).padding(12.0)` |
| `layout` | `theme.panel().radius(0.0).bordered().size(UIValue::Auto, UIValue::Percent(100.0)).column().gap(8.0).padding(12.0)` |
| section titles (`FOUNDATIONS`, `LAYOUT`, `ANCHORED PANELS`) | `theme.label(title).font_size(15.0).height(UIValue::Px(32.0)).fixed()` |
| each `swatch` | `(theme.canvas().fill(color).height(UIValue::Px(theme.row_height)).fixed().padding(UIRect::axes(4.0, 8.0)), theme.text(name).font_size(13.0))` |
| `type_sample` | `theme.label("Display 24\nBody 14 — …").font_size(14.0).grow().min_height(UIValue::Px(90.0))` (keep the full sample string) |
| `centered` | `theme.card().radius(0.0).height(UIValue::Px(96.0)).min_width(UIValue::Px(260.0)).max_width(UIValue::Px(640.0)).fixed().row().gap(theme.spacing_sm).justify(AlignContent::Center).padding(theme.spacing_md)` |
| each centred `item` | `(theme.canvas().fill(theme.accent).border(theme.focus, 1.0).size(UIValue::Px(width), UIValue::Px(theme.control_height)).padding(UIRect::axes(6.0, 8.0)), theme.text(name).font_size(11.0))` |
| `panel_row` | `theme.row().height(UIValue::Px(theme.control_height)).fixed()` |
| `menu_trigger` | `theme.button("MENU  \u{25be}").width(UIValue::Px(120.0)).font_size(12.0)` |
| `dropdown` | `theme.dropdown(menu_trigger).width(UIValue::Px(180.0)).padding(theme.spacing_xs)` |
| `rename_field` | `theme.text_field("Rename…").fill(theme.surface)` |
| submenu | `(theme.dropdown(materials_row).width(UIValue::Px(150.0)).padding(theme.spacing_xs).anchor_side(UIAnchorSide::Right).anchor_gap(2.0), theme.text("Standard\nUnlit\nToon").font_size(12.0))` |
| context menu | `(theme.context_menu().width(UIValue::Px(170.0)).padding(theme.spacing_xs), ContextMenu, theme.text("").font_size(12.0))` |
| `nested` | `(theme.card().radius(0.0).grow().min_height(UIValue::Px(64.0)).column().gap(theme.spacing_sm).padding(theme.spacing_md), theme.text("Nested flex / percent sizing\n…").font_size(13.0))` (keep the full string) |
| `controls` | `theme.row().height(UIValue::Px(40.0)).fixed().align_items(AlignItems::Stretch)` |
| `button` | `theme.button("BUTTON").width(UIValue::Px(112.0)).font_size(12.0)` |
| `checkbox` | `theme.checkbox("CHECK", false).width(UIValue::Px(90.0)).padding(UIRect::axes(7.0, 9.0)).font_size(12.0)` |
| `slider` | `theme.slider(0.62, 0.0, 1.0).width(UIValue::Px(140.0))` |
| `input` | `theme.text_field("Unicode input…").fill(theme.surface).width(UIValue::Px(180.0)).padding(UIRect::axes(7.0, 9.0))` |
| `virtual_list` | `(theme.card().radius(0.0).height(UIValue::Px(96.0)).min_height(UIValue::Px(56.0)).column().gap(0.0).padding(theme.spacing_sm).node(\|n\| n.with_overflow_y(concerto::ui::node::Overflow::Hidden)), UIScrollArea { .. }, UIVirtualList::new(10_000, 28.0), Interactable)` |
| each virtual `row` | `(theme.label("").font_size(12.0).height(UIValue::Px(28.0)).fixed(), Interactable, VirtualRow { slot })` |
| `split_first` / `split_second` | `(theme.canvas().fill(theme.surface), theme.text("SPLIT A").font_size(12.0))` / `(theme.canvas().fill(theme.surface_raised), theme.text("SPLIT B").font_size(12.0))` |
| `split_handle` | `(theme.canvas().fill(theme.accent).size(UIValue::Px(5.0), UIValue::Percent(100.0)).fixed(), Interactable, UISplitHandle { pane: split })` |
| `flip_row` | `theme.row().height(UIValue::Px(theme.control_height)).fixed().justify(AlignContent::End)` |
| `flip_trigger` | `theme.button("VIEW  \u{25be}").width(UIValue::Px(120.0)).font_size(12.0)` |
| `flip_dropdown` | `theme.dropdown(flip_trigger).width(UIValue::Px(220.0)).padding(theme.spacing_xs)` |
| `diagnostics` | `(theme.card().radius(0.0).height(UIValue::Px(28.0)).fixed().padding(UIRect::axes(5.0, 8.0)), theme.text("").mono().font_size(11.0), Diagnostics)` |

The `split` pane node (no theme token) stays a raw `UINode`. `.align_items(AlignItems::Stretch)` on `body` and `controls` undoes `row()`'s centring where the original rows let children stretch.

- [ ] **Step 3: Build and lint the whole workspace**

Run: `cargo build --workspace --all-targets` → success.
Run: `cargo test --workspace` → PASS (or the same pre-existing failures recorded in Task 6 Step 1, none new).
Run: `cargo clippy --workspace --all-targets` → no new warnings.
Run: `grep -rn "from_theme\|widgets::button\|selection_tint\|marks::TRANSPARENT" crates examples --include=*.rs` → no matches.

- [ ] **Step 4: Commit**

```bash
git add examples/ui-showcase/src/main.rs crates/ui/src/widgets.rs crates/ui/src/text/mod.rs
git commit -m "Build the UI showcase from themed elements and drop the superseded helpers"
```

- [ ] **Step 5: Visual check (controller, not subagent)**

Launch `cargo run -p ui-showcase` and `cargo run -p concerto-editor` (use the `run` skill). Compare against the spec's "Expected visual changes": body text slightly cooler/greyer, kind tags 0.5px larger, showcase buttons rounded, checkbox accent. Anything else that moved is a bug in the task that migrated it.
