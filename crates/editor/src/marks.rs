//! The small geometric marks the design uses in place of icons.
use concerto_color::Color;
use concerto_ecs::query::change_detection::Mut;
use concerto_ui::{
    material::UIMaterial,
    node::{UINode, UIRect},
    theme::UITheme,
    transform::UIValue,
};

/// Width of the column a mark sits in, in logical pixels.
pub const COLUMN: f32 = 18.0;

/// What a row holds, as far as the panel can tell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A group: something with children and nothing of its own to draw.
    Group,
    /// A folder of assets: an outlined square in the secondary accent.
    Folder,
    Mesh,
    Camera,
    Light,
    Material,
    Texture,
    /// A row whose kind we cannot name; drawn as a dim dot.
    Plain,
    /// An empty slot: no row here at all.
    None,
}

/// The shape of a mark: how big its node is, and what fills it.
pub struct Shape {
    pub size: f32,
    pub corner_radius: f32,
    pub rotation: f32,
    pub fill: Color,
    pub border: Color,
    pub border_width: f32,
}

const DIAMOND: f32 = std::f32::consts::FRAC_PI_4;

impl Mark {
    /// The mark's geometry, taking the accent when the row is selected.
    pub fn shape(self, theme: &UITheme, selected: bool) -> Shape {
        let outline = if selected {
            theme.text
        } else {
            theme.text_muted
        };
        let solid = if selected {
            theme.accent
        } else {
            theme.text_muted
        };
        let plain = Shape {
            size: 8.0,
            corner_radius: 2.0,
            rotation: 0.0,
            fill: Color::TRANSPARENT,
            border: Color::TRANSPARENT,
            border_width: 0.0,
        };
        match self {
            Mark::Group => Shape {
                size: 8.0,
                border: outline,
                border_width: 1.5,
                ..plain
            },
            Mark::Folder => Shape {
                size: 8.0,
                border: if selected {
                    theme.accent
                } else {
                    theme.accent_secondary
                },
                border_width: 1.5,
                ..plain
            },
            Mark::Mesh => Shape {
                size: 11.0,
                corner_radius: 1.0,
                rotation: DIAMOND,
                fill: solid,
                ..plain
            },
            Mark::Camera => Shape {
                size: 9.0,
                border: outline,
                border_width: 1.5,
                ..plain
            },
            Mark::Light => Shape {
                size: 7.0,
                corner_radius: 4.0,
                fill: if selected {
                    theme.accent
                } else {
                    theme.accent_secondary
                },
                ..plain
            },
            Mark::Material => Shape {
                size: 9.0,
                corner_radius: 5.0,
                fill: if selected {
                    theme.accent
                } else {
                    theme.accent_secondary
                },
                ..plain
            },
            Mark::Texture => Shape {
                size: 8.0,
                fill: solid,
                ..plain
            },
            Mark::Plain => Shape {
                size: 4.0,
                corner_radius: 2.0,
                fill: outline,
                ..plain
            },
            Mark::None => Shape { size: 0.0, ..plain },
        }
    }
}

/// A mark node, centred in its column and sized for `Mark::None` until a panel says otherwise.
pub fn node() -> (UINode, UIMaterial) {
    (
        UINode::default()
            .with_size(UIValue::Px(0.0), UIValue::Px(0.0))
            .with_flex_shrink(0.0)
            .with_align_self(taffy::AlignItems::Center)
            .with_margin(UIRect::axes(0.0, COLUMN / 2.0)),
        UIMaterial::flat(Color::TRANSPARENT),
    )
}

/// A mark node already drawn as `mark`, unselected, for a row spawned this frame.
pub fn shaped(mark: Mark, theme: &UITheme) -> (UINode, UIMaterial) {
    let shape = mark.shape(theme, false);
    let (mut node, mut material) = node();
    fit_node(&shape, &mut node);
    fit_material(&shape, &mut material);
    (node, material)
}

/// Applies a mark to the node and material spawned by [`node`], writing to each only when it differs, so an unchanged mark is not flagged as changed.
pub fn apply(
    mark: Mark,
    theme: &UITheme,
    selected: bool,
    node: &mut Mut<UINode>,
    material: &mut Mut<UIMaterial>,
) {
    let shape = mark.shape(theme, selected);
    if !node_fits(&shape, node) {
        fit_node(&shape, node);
    }
    if !material_fits(&shape, material) {
        fit_material(&shape, material);
    }
}

fn node_fits(shape: &Shape, node: &UINode) -> bool {
    node.width == UIValue::Px(shape.size)
}

fn fit_node(shape: &Shape, node: &mut UINode) {
    let size = UIValue::Px(shape.size);
    node.width = size;
    node.height = size;
    node.margin = UIRect::axes(0.0, (COLUMN - shape.size) / 2.0);
}

fn material_fits(shape: &Shape, material: &UIMaterial) -> bool {
    material.color == shape.fill.to_linear()
        && material.border_color == shape.border.to_linear()
        && material.border_width == shape.border_width
        && material.corner_radius == shape.corner_radius
        && material.rotation == shape.rotation
}

fn fit_material(shape: &Shape, material: &mut UIMaterial) {
    material.color = shape.fill.to_linear();
    material.border_color = shape.border.to_linear();
    material.border_width = shape.border_width;
    material.corner_radius = shape.corner_radius;
    material.rotation = shape.rotation;
}
