use concerto_color::{Color, LinearRgba};
use concerto_ecs::component::Component;
use concerto_foundation::assets::{Asset, handle::AssetHandle};
use concerto_render::{AsBindGroup, assets::texture::Texture, assets::vertex::VertexBufferLayout};

use crate::vertex::UIVertex;

/// Material for UI elements.
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

    /// Border outline colour (RGBA). Only visible when `border_width > 0`.
    #[uniform(1)]
    pub border_color: LinearRgba,

    /// GPU-side shape parameters — **do not set manually**.
    #[uniform(2)]
    pub border_params: [f32; 4],

    /// GPU-side shape flags — **do not set manually**.
    #[uniform(3)]
    pub flags: [f32; 4],

    /// Optional texture, multiplied by [`color`](Self::color).
    #[texture(4)]
    #[sampler(5)]
    pub texture: Option<AssetHandle<Texture>>,

    /// Border width in logical pixels. Set this; the engine manages `border_params` automatically.
    pub border_width: f32,

    /// Corner radius in logical pixels, clamped to half the node's shorter side.
    pub corner_radius: f32,

    /// Rotation of the drawn shape within its node, in radians.
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
