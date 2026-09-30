use concerto_ecs::component::bundle::IntoBundle;

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

impl IntoBundle for Checkbox {
    type Bundle = (UINode, UIMaterial, UICheckbox, Interactable, UIText);

    fn into_bundle(self) -> Self::Bundle {
        (
            self.node,
            self.material,
            self.checkbox,
            Interactable,
            self.text,
        )
    }
}

impl IntoBundle for Slider {
    type Bundle = (UINode, UIMaterial, UISlider, Interactable);

    fn into_bundle(self) -> Self::Bundle {
        (self.node, self.material, self.slider, Interactable)
    }
}

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
