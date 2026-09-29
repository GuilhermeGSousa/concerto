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
