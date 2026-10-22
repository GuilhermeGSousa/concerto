use concerto_color::Color;
use concerto_ecs::{component::bundle::IntoBundle, entity::Entity};

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
    /// A menu below `trigger`, opened and closed by a trigger that listens with `toggle_owned_panels`.
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
}

impl IntoBundle for Popup {
    type Bundle = (UINode, UIMaterial, UIAnchoredPanel);

    fn into_bundle(self) -> Self::Bundle {
        let (node, material) = self.surface.into_bundle();
        (node, material, self.panel)
    }
}

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
