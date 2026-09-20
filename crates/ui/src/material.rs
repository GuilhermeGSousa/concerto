use color::{Color, LinearRgba};
use ecs::component::Component;
use essential::assets::{Asset, handle::AssetHandle};
use render::{AsBindGroup, assets::texture::Texture, assets::vertex::VertexBufferLayout};

use crate::vertex::UIVertex;

/// Material for UI elements.
///
/// # Border rendering
///
/// Set `border_width` (pixels) and `border_color` to draw a solid rectangular
/// outline.  The engine automatically syncs the node's computed pixel size into
/// `border_params` each frame, so you only need to supply `border_width`.
///
/// # Example
/// ```rust,ignore
/// UIMaterial {
///     border_width: 1.0,
///     ..UIMaterial::with_border(
///         Color::rgba(0.15, 0.15, 0.15, 1.0),
///         Color::rgba(0.4, 0.4, 0.4, 1.0),
///         1.0,
///     )
/// }
/// ```
#[derive(Component, Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader = include_str!("shaders/ui.wgsl"),
    fragment_shader = include_str!("shaders/ui.wgsl"),
    camera = false,
    depth_stencil = "none",
    blend = "alpha",
    vertex_layouts = vec![UIVertex::describe()],
)]
pub struct UIMaterial {
    /// Background fill colour (RGBA, values in `[0.0, 1.0]`).
    #[uniform(0)]
    pub color: LinearRgba,

    /// Border outline colour (RGBA).  Only visible when `border_width > 0`.
    #[uniform(1)]
    pub border_color: LinearRgba,

    /// GPU-side shape parameters — **do not set manually**.
    ///
    /// Layout: `[border_width_px, node_width_px, node_height_px, corner_radius_px]`.
    /// The `sync_material_params` system fills these in each frame from the
    /// user-facing fields and the node's measured size.
    #[uniform(2)]
    pub border_params: [f32; 4],

    /// GPU-side shape flags — **do not set manually**.
    ///
    /// Layout: `[has_texture, rotation_radians, 0, 0]`. `has_texture` is an
    /// explicit flag rather than relying on the dummy texture's contents,
    /// matching `StandardMaterial`; the rotation is packed here by
    /// `sync_material_params` from the user-facing field.
    #[uniform(3)]
    pub flags: [f32; 4],

    /// Optional texture, multiplied by [`color`](Self::color).
    ///
    /// This is what makes a UI node able to show anything sampled: a camera's
    /// render target, an icon, a baked gradient.
    #[texture(4)]
    #[sampler(5)]
    pub texture: Option<AssetHandle<Texture>>,

    /// Border width in logical pixels.  Set this; the engine manages
    /// `border_params` automatically.
    pub border_width: f32,

    /// Corner radius in logical pixels, clamped to half the node's shorter
    /// side so a fully-rounded pill is just a large value.
    pub corner_radius: f32,

    /// Rotation of the drawn shape within its node, in radians.
    ///
    /// The node itself does not rotate — layout is unaffected — only the
    /// rectangle drawn inside it, which is shrunk to stay within the node's
    /// box. A square at `FRAC_PI_4` is how the design system draws a diamond.
    pub rotation: f32,
}

impl UIMaterial {
    /// A plain filled rectangle with no border.
    pub fn flat(color: Color) -> Self {
        Self {
            color: color.to_linear(),
            border_color: LinearRgba::TRANSPARENT,
            border_width: 0.0,
            corner_radius: 0.0,
            border_params: [0.0; 4],
            flags: [0.0; 4],
            texture: None,
            rotation: 0.0,
        }
    }

    /// A filled rectangle with a solid-colour border.
    pub fn with_border(color: Color, border_color: Color, border_width: f32) -> Self {
        Self {
            color: color.to_linear(),
            border_color: border_color.to_linear(),
            border_width,
            corner_radius: 0.0,
            border_params: [border_width, 0.0, 0.0, 0.0],
            flags: [0.0; 4],
            texture: None,
            rotation: 0.0,
        }
    }
}

impl Default for UIMaterial {
    fn default() -> Self {
        Self::flat(Color::WHITE)
    }
}
