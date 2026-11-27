//! Themed element builders, created from a [`UITheme`] and spawnable as bundles.
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

mod button;
mod chip;
mod controls;
mod field;
mod list;
mod listen;
mod popup;
mod surface;
mod text;

pub use button::{Button, Pressable};
pub use chip::Chip;
pub use controls::{Checkbox, ColorPicker, Slider, Splitter};
pub use field::{SearchField, TextField};
pub use list::{ListBuilder, ListItem, SpawnedList};
pub use listen::{Listen, Listening};
pub use popup::Popup;
pub use surface::{Divider, Stack, Surface};
pub use text::{Label, Text};

/// The modifier traits.
pub mod prelude {
    pub use super::{Interactive, Layout, Listen, Shape, Themed, Typography};
}

/// A builder that carries the theme it was created from.
pub trait Themed {
    /// The theme the builder resolves its defaults against.
    fn theme(&self) -> &UITheme;
}

/// Size, flex and spacing modifiers for any builder with a [`UINode`].
pub trait Layout: Sized {
    /// The builder's node, which the modifiers change.
    fn node_mut(&mut self) -> &mut UINode;

    /// Escape hatch for any [`UINode`] setting without a modifier of its own.
    fn node(mut self, edit: impl FnOnce(UINode) -> UINode) -> Self {
        let node = std::mem::take(self.node_mut());
        *self.node_mut() = edit(node);
        self
    }

    /// Sets the node's width.
    fn width(self, width: UIValue) -> Self {
        self.node(|node| node.with_width(width))
    }

    /// Sets the node's height.
    fn height(self, height: UIValue) -> Self {
        self.node(|node| node.with_height(height))
    }

    /// Sets the node's width and height.
    fn size(self, width: UIValue, height: UIValue) -> Self {
        self.node(|node| node.with_size(width, height))
    }

    /// Sets the node's least width.
    fn min_width(self, min_width: UIValue) -> Self {
        self.node(|node| node.with_min_width(min_width))
    }

    /// Sets the node's least height.
    fn min_height(self, min_height: UIValue) -> Self {
        self.node(|node| node.with_min_height(min_height))
    }

    /// Sets the node's greatest width.
    fn max_width(self, max_width: UIValue) -> Self {
        self.node(|node| node.with_max_width(max_width))
    }

    /// Sets the node's greatest height.
    fn max_height(self, max_height: UIValue) -> Self {
        self.node(|node| node.with_max_height(max_height))
    }

    /// Lays children out left to right and centres them.
    ///
    /// Call `align_items` after it to change the alignment.
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

    /// Sets the space inside the node's edges; an `f32` pads every side.
    fn padding(self, padding: impl Into<UIRect>) -> Self {
        let padding = padding.into();
        self.node(|node| node.with_padding(padding))
    }

    /// Sets the space outside the node's edges; an `f32` applies to every side.
    fn margin(self, margin: impl Into<UIRect>) -> Self {
        let margin = margin.into();
        self.node(|node| node.with_margin(margin))
    }

    /// Takes up the free space along the parent's main axis.
    fn grow(self) -> Self {
        self.node(|node| node.with_flex_grow(1.0))
    }

    /// Sets the node's share of its parent's overflow it gives up; 0 never shrinks.
    fn shrink(self, shrink: f32) -> Self {
        self.node(|node| node.with_flex_shrink(shrink))
    }

    /// Never shrinks below its own size.
    fn fixed(self) -> Self {
        self.shrink(0.0)
    }

    /// Sets how children are aligned across the main axis.
    fn align_items(self, align: AlignItems) -> Self {
        self.node(|node| node.with_align_items(align))
    }

    /// Sets how the node is aligned across its parent's main axis.
    fn align_self(self, align: AlignItems) -> Self {
        self.node(|node| node.with_align_self(align))
    }

    /// Sets how children are distributed along the main axis.
    fn justify(self, justify: AlignContent) -> Self {
        self.node(|node| node.with_justify_content(justify))
    }

    /// Clips children to the node's box on both axes.
    fn clipped(self) -> Self {
        self.node(UINode::clipped)
    }

    /// Hides the node: it and its subtree are neither laid out nor drawn.
    fn hidden(self) -> Self {
        self.node(|node| node.with_visible(false))
    }

    /// Sets the node's paint layer, which its subtree inherits; higher is on top.
    fn z_index(self, z_index: i32) -> Self {
        self.node(|node| node.with_z_index(z_index))
    }
}

/// Type-scale and colour modifiers for any builder with a [`UIText`].
pub trait Typography: Themed + Sized {
    /// The builder's text, which the modifiers change.
    fn text_mut(&mut self) -> &mut UIText;

    /// Sets the size and the theme's leading for it.
    fn font_size(mut self, size: f32) -> Self {
        let line_height = self.theme().line_height(size);
        let text = self.text_mut();
        text.font_size = size;
        text.line_height = line_height;
        self
    }

    /// Sets the distance between baselines, in logical pixels.
    fn line_height(mut self, line_height: f32) -> Self {
        self.text_mut().line_height = line_height;
        self
    }

    /// Uses the theme's small font size.
    fn small(self) -> Self {
        let size = self.theme().font_size_sm;
        self.font_size(size)
    }

    /// Uses the theme's large font size.
    fn large(self) -> Self {
        let size = self.theme().font_size_lg;
        self.font_size(size)
    }

    /// Uses the theme's muted text colour.
    fn muted(self) -> Self {
        let color = self.theme().text_muted;
        self.color(color)
    }

    /// Sets the text colour.
    fn color(mut self, color: Color) -> Self {
        self.text_mut().color = color;
        self
    }

    /// Sets the font family.
    fn family(mut self, family: FontFamily) -> Self {
        self.text_mut().font_family = family;
        self
    }

    /// Uses the monospace font family.
    fn mono(self) -> Self {
        self.family(FontFamily::Monospace)
    }

    /// Sets the font weight, from 100 to 900: 400 is regular, 700 bold.
    fn weight(mut self, weight: u16) -> Self {
        self.text_mut().font_weight = weight;
        self
    }

    /// One line, cut short with an ellipsis when it does not fit.
    ///
    /// A text field's label never shows an ellipsis.
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
    /// The builder's material, which the modifiers change.
    fn material_mut(&mut self) -> &mut UIMaterial;

    /// Sets the corner radius, in logical pixels.
    fn radius(mut self, radius: f32) -> Self {
        self.material_mut().corner_radius = radius;
        self
    }

    /// Uses the theme's small corner radius.
    fn radius_sm(self) -> Self {
        let radius = self.theme().radius_sm;
        self.radius(radius)
    }

    /// Uses the theme's medium corner radius.
    fn radius_md(self) -> Self {
        let radius = self.theme().radius_md;
        self.radius(radius)
    }

    /// Uses the theme's large corner radius.
    fn radius_lg(self) -> Self {
        let radius = self.theme().radius_lg;
        self.radius(radius)
    }

    /// Draws a border of `color`, `width` logical pixels wide.
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

    /// No border at all.
    fn borderless(self) -> Self {
        self.border(Color::TRANSPARENT, 0.0)
    }

    /// Rotates the drawn shape within its node by `radians`.
    fn rotation(mut self, radians: f32) -> Self {
        self.material_mut().rotation = radians;
        self
    }
}

/// How an interactive element colours itself.
///
/// It is resolved against the theme when the element spawns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionSpec {
    /// The theme's colour scheme to start from.
    pub variant: ButtonVariant,
    /// Whether the element shows as selected, which changes its normal colour.
    pub selected: bool,
    /// Replaces the variant's normal colour.
    pub normal: Option<Color>,
    /// Replaces the variant's hovered colour.
    pub hovered: Option<Color>,
    /// Replaces the variant's pressed colour.
    pub pressed: Option<Color>,
    /// Replaces the variant's disabled colour.
    pub disabled: Option<Color>,
}

impl InteractionSpec {
    /// The unselected colours of `variant`, with nothing replaced.
    pub fn new(variant: ButtonVariant) -> Self {
        Self {
            variant,
            selected: false,
            normal: None,
            hovered: None,
            pressed: None,
            disabled: None,
        }
    }

    /// The colours `theme` gives the variant and state, with the replacements applied.
    pub fn resolve(&self, theme: &UITheme) -> UIInteractionStyle {
        let mut style = theme.interaction(self.variant, self.selected);
        if let Some(normal) = self.normal {
            style.normal = normal;
        }
        if let Some(hovered) = self.hovered {
            style.hovered = hovered;
        }
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
    /// The builder's interaction colours, which the modifiers change.
    fn interaction_mut(&mut self) -> &mut InteractionSpec;

    /// Sets the colour scheme.
    fn variant(mut self, variant: ButtonVariant) -> Self {
        self.interaction_mut().variant = variant;
        self
    }

    /// Uses the [`ButtonVariant::Solid`] colours.
    fn solid(self) -> Self {
        self.variant(ButtonVariant::Solid)
    }

    /// Uses the [`ButtonVariant::Ghost`] colours.
    fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }

    /// Uses the [`ButtonVariant::Tab`] colours.
    fn tab(self) -> Self {
        self.variant(ButtonVariant::Tab)
    }

    /// Shows the element as selected or not.
    fn selected(mut self, selected: bool) -> Self {
        self.interaction_mut().selected = selected;
        self
    }

    /// Replaces the colour with the pointer off the element.
    fn normal(mut self, color: Color) -> Self {
        self.interaction_mut().normal = Some(color);
        self
    }

    /// Replaces the colour with the pointer over the element.
    fn hovered(mut self, color: Color) -> Self {
        self.interaction_mut().hovered = Some(color);
        self
    }

    /// Replaces the colour while the element is pressed.
    fn pressed(mut self, color: Color) -> Self {
        self.interaction_mut().pressed = Some(color);
        self
    }

    /// Replaces the colour while the element is disabled.
    fn disabled_color(mut self, color: Color) -> Self {
        self.interaction_mut().disabled = Some(color);
        self
    }
}
