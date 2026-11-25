use concerto_color::Color;
use concerto_ecs::{component::bundle::IntoBundle, entity::Entity};

use super::{Layout, Shape, Themed, Typography};
use crate::{
    focus::UIFocusable,
    interaction::Interactable,
    material::UIMaterial,
    node::{Overflow, UINode, UIRect},
    search::UISearchField,
    text::UIText,
    text_input::UITextInput,
    theme::UITheme,
    transform::UIValue,
};

/// An editable, focusable, one-line text field whose text is drawn by a label in a viewport child that clips it to the field's content box.
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
                .with_padding(UIRect::axes(self.spacing_xs, self.spacing_sm))
                .with_overflow_x(Overflow::Hidden),
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

/// A [`TextField`] that narrows a `UIList` or `UITree` to the items whose `UISearchText` holds its text, and drives that list's highlight from the keyboard.
pub struct SearchField {
    field: TextField,
    search: UISearchField,
}

impl UITheme {
    /// A text field with a search placeholder that narrows the list `target`.
    pub fn search_field(&self, placeholder: impl Into<String>, target: Entity) -> SearchField {
        SearchField {
            field: self.text_field(placeholder),
            search: UISearchField { target },
        }
    }
}

impl IntoBundle for SearchField {
    type Bundle = (<TextField as IntoBundle>::Bundle, UISearchField);

    fn into_bundle(self) -> Self::Bundle {
        (self.field.into_bundle(), self.search)
    }
}

impl Themed for SearchField {
    fn theme(&self) -> &UITheme {
        self.field.theme()
    }
}

impl Layout for SearchField {
    fn node_mut(&mut self) -> &mut UINode {
        self.field.node_mut()
    }
}

impl Shape for SearchField {
    fn material_mut(&mut self) -> &mut UIMaterial {
        self.field.material_mut()
    }
}

impl Typography for SearchField {
    fn text_mut(&mut self) -> &mut UIText {
        self.field.text_mut()
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
