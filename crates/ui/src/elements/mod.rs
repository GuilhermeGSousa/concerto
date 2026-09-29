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

/// The modifier traits.
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
