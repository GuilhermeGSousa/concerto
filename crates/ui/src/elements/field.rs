use concerto_color::Color;
use concerto_ecs::component::bundle::IntoBundle;

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
    /// The starting value, with the cursor after it.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.input.value = value.into();
        self.input.cursor = self.input.value.len();
        self
    }

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
}

impl IntoBundle for TextField {
    type Bundle = (
        UINode,
        UIMaterial,
        UIText,
        UITextInput,
        Interactable,
        UIFocusable,
    );

    fn into_bundle(self) -> Self::Bundle {
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
