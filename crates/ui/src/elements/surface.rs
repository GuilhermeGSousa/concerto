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
