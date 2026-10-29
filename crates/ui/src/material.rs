use concerto_color::{Color, LinearRgba, Srgba};
use concerto_ecs::component::Component;
use concerto_foundation::assets::{Asset, LoadableAsset, handle::AssetHandle};
use concerto_render::{AsBindGroup, assets::texture::Texture, assets::vertex::VertexBufferLayout};

use crate::vertex::UIVertex;

/// A fill the UI shader computes per pixel in place of `color`'s RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum UIGradient {
    #[default]
    None,
    /// Saturation from 0 at the left to 1 at the right, value from 1 at the top to 0 at the bottom, at `hue` degrees, in sRGB.
    SaturationValue { hue: f32 },
    /// Hue from 0 to 360 degrees, left to right, at full saturation and value.
    Hue,
    /// `color`, in sRGB, from alpha 0 at the left to 1 at the right, over a checkerboard of `cell` logical pixels.
    Alpha { color: Srgba, cell: f32 },
}

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
#[require(concerto_render::components::render_entity::SyncWithRenderWorld)]
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

    /// GPU-side gradient parameters — **do not set manually**.
    #[uniform(6)]
    pub gradient_params: [f32; 4],

    /// GPU-side gradient colour — **do not set manually**.
    #[uniform(7)]
    pub gradient_color: [f32; 4],

    /// Border width in logical pixels. Set this; the engine manages `border_params` automatically.
    pub border_width: f32,

    /// Corner radius in logical pixels, clamped to half the node's shorter side.
    pub corner_radius: f32,

    /// Rotation of the drawn shape within its node, in radians.
    pub rotation: f32,

    /// A computed fill drawn in place of [`color`](Self::color)'s RGB; its alpha still comes from `color`.
    pub gradient: UIGradient,
}

impl LoadableAsset for UIMaterial {}

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
            gradient_params: [0.0; 4],
            gradient_color: [0.0; 4],
            rotation: 0.0,
            gradient: UIGradient::None,
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
            gradient_params: [0.0; 4],
            gradient_color: [0.0; 4],
            rotation: 0.0,
            gradient: UIGradient::None,
        }
    }
}

impl Default for UIMaterial {
    fn default() -> Self {
        Self::flat(Color::WHITE)
    }
}
