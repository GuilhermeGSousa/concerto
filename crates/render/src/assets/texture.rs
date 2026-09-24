use concerto_core::assets::{Asset, LoadableAsset};

pub use wgpu_types::TextureFormat;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureKind {
    Sampled,
    RenderTarget,
}

#[derive(Asset, serde::Serialize, serde::Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: wgpu_types::TextureFormat,
    pub kind: TextureKind,
    /// RGBA8 pixels matching `format`. Empty for `TextureKind::RenderTarget`.
    pub data: Vec<u8>,
}

impl Texture {
    /// A GPU-only render target; the camera system allocates the wgpu texture.
    pub fn render_target(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            format: wgpu_types::TextureFormat::Rgba8UnormSrgb,
            kind: TextureKind::RenderTarget,
            data: Vec::new(),
        }
    }

    pub fn size(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl LoadableAsset for Texture {}
