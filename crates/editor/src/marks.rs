//! The small geometric marks the design uses in place of icons.
//!
//! Nocturne has no icon set. A row says what it holds with a 7–11px shape in
//! the palette — a filled diamond for a mesh, an outlined rounded rect for a
//! camera — which is quieter than a pictogram and reads at the size a list row
//! actually gives it. Each mark is one node: the shape is the node's material,
//! not a glyph, so it costs no font lookup and scales with the theme.
use color::Color;
use ui::{
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

pub const TRANSPARENT: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);

/// The wash of accent behind a selected row.
///
/// A tint rather than a block: the name stays readable and the row still reads
/// as part of the list, which is how the design marks selection everywhere.
pub fn selection_tint(theme: &UITheme) -> Color {
    let accent = theme.accent.to_srgba();
    Color::srgba(accent.r, accent.g, accent.b, 0.2)
}
/// A square turned onto its corner. The shader keeps it inside its node, so the
/// node has to be the diagonal rather than the side.
const DIAMOND: f32 = std::f32::consts::FRAC_PI_4;

impl Mark {
    /// The mark's geometry, taking the accent when the row is selected so the
    /// mark reads as part of the highlight rather than a hole in it.
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
            fill: TRANSPARENT,
            border: TRANSPARENT,
            border_width: 0.0,
        };
        match self {
            Mark::Group => Shape {
                size: 8.0,
                border: outline,
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
                // The one mark that is not square: a lens is wider than tall.
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

/// A mark node, centred in its column and sized for `Mark::None` until a panel
/// says otherwise.
pub fn node() -> (UINode, UIMaterial) {
    (
        UINode {
            width: UIValue::Px(0.0),
            height: UIValue::Px(0.0),
            flex_shrink: 0.0,
            align_self: Some(taffy::AlignItems::Center),
            // Centres the shape in its column whatever size it is.
            margin: UIRect::axes(0.0, COLUMN / 2.0),
            ..Default::default()
        },
        UIMaterial::flat(TRANSPARENT),
    )
}

/// Applies a mark to the node and material spawned by [`node`].
///
/// Only writes on change: a mark node that rewrote its style every frame would
/// invalidate the whole layout pass, and there is one per visible row.
pub fn apply(
    mark: Mark,
    theme: &UITheme,
    selected: bool,
    node: &mut UINode,
    material: &mut UIMaterial,
) {
    let shape = mark.shape(theme, selected);
    let size = UIValue::Px(shape.size);
    if node.width != size {
        node.width = size;
        node.height = size;
        node.margin = UIRect::axes(0.0, (COLUMN - shape.size) / 2.0);
    }
    let fill = shape.fill.to_linear();
    if material.color != fill {
        material.color = fill;
    }
    let border = shape.border.to_linear();
    if material.border_color != border {
        material.border_color = border;
    }
    if material.border_width != shape.border_width {
        material.border_width = shape.border_width;
    }
    if material.corner_radius != shape.corner_radius {
        material.corner_radius = shape.corner_radius;
    }
    if material.rotation != shape.rotation {
        material.rotation = shape.rotation;
    }
}
