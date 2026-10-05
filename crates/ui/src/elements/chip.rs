use concerto_color::Color;
use concerto_ecs::component::bundle::IntoBundle;

use super::{Layout, Listen, Themed, Typography};
use crate::{
    interaction::Interactable,
    material::UIMaterial,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
};

/// A small bordered label coloured by `UITheme::chip_colors` at spawn.
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
}

impl IntoBundle for Chip {
    type Bundle = (UINode, UIMaterial, UIText, Interactable);

    fn into_bundle(self) -> Self::Bundle {
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

impl Listen for Chip {}

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
