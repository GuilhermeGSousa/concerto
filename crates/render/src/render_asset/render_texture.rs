use crate::{
    assets::texture::{block_extent, block_row_bytes, block_rows, Texture, TextureKind},
    capabilities::RenderCapabilities,
    device::RenderDevice,
    queue::RenderQueue,
    render_asset::{AssetPreparationError, RenderAsset},
};
use concerto_app::extractor::Extracted;
use concerto_ecs::{
    resource::{Res, Resource},
    system::input::SystemInputData,
};
use concerto_foundation::assets::{asset_server::AssetServer, AssetId};
use std::ops::Deref;
use wgpu::TextureUsages;

#[allow(dead_code)]
#[derive(Clone)]
pub struct RenderTexture {
    /// The GPU texture view (used for bind groups and render pass attachments).
    pub view: wgpu::TextureView,
    /// The sampler associated with this texture.
    pub sampler: wgpu::Sampler,
    /// The underlying GPU texture.  Kept alive here so the view remains valid.
    pub(crate) texture: wgpu::Texture,
}

impl RenderTexture {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn from_texture(texture: &Texture, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let usage = match texture.kind {
            TextureKind::Sampled => TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            TextureKind::RenderTarget { .. } => {
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING
            }
        };
        let levels = texture.levels();
        let format = levels.map_or(wgpu::TextureFormat::Rgba8UnormSrgb, |levels| levels.format);
        let wgpu_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: texture.size(),
            mip_level_count: levels.map_or(1, |levels| levels.levels.len() as u32),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = wgpu_texture.create_view(&wgpu::TextureViewDescriptor::default());

        if let Some(levels) = levels {
            for (mip_level, data) in levels.levels.iter().enumerate() {
                let extent = levels.extent(mip_level as u32);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        aspect: wgpu::TextureAspect::All,
                        texture: &wgpu_texture,
                        mip_level: mip_level as u32,
                        origin: wgpu::Origin3d::ZERO,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: block_row_bytes(format, extent),
                        rows_per_image: Some(block_rows(format, extent)),
                    },
                    block_extent(format, extent),
                );
            }
        }

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("RenderTexture Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        Self {
            texture: wgpu_texture,
            view,
            sampler,
        }
    }

    pub fn create_depth_texture(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        label: &str,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 100.0,
            ..Default::default()
        });
        Self {
            texture,
            view,
            sampler,
        }
    }
}

impl RenderAsset for RenderTexture {
    type SourceAsset = Texture;
    type PreparationParams = (
        Res<'static, RenderDevice>,
        Res<'static, RenderQueue>,
        Res<'static, RenderCapabilities>,
        Res<'static, DummyRenderTexture>,
        Extracted<'static, 'static, Option<Res<'static, AssetServer>>>,
    );

    fn prepare_asset(
        id: AssetId,
        source_asset: &Self::SourceAsset,
        params: &mut SystemInputData<Self::PreparationParams>,
    ) -> Result<Self, AssetPreparationError> {
        // Render-target textures (created via Texture::render_target()) carry no
        // CPU data and are managed directly by the camera system.  Skip them here
        // so prepare_render_asset never overwrites the camera's allocation.
        if matches!(source_asset.kind, TextureKind::RenderTarget { .. })
            || source_asset.levels().is_none()
        {
            return Err(AssetPreparationError::NotReady);
        }

        let (device, queue, capabilities, dummy, server) = params;
        let format = source_asset.levels().map(|levels| levels.format);
        if let Some(format) = format.filter(|format| {
            format
                .required_features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
                && !capabilities.bc_textures
        }) {
            let location = (**server)
                .as_ref()
                .and_then(|server| server.registry())
                .and_then(|registry| registry.get(id).map(|entry| entry.location()))
                .unwrap_or_else(|| format!("{id:?}"));
            log::error!(
                "{location} is {format:?}, but this device has no BC texture compression; set its compression to Uncompressed or use a browser with BC"
            );
            return Ok(dummy.inner().clone());
        }
        Ok(RenderTexture::from_texture(source_asset, device, queue))
    }

    fn byte_len(source: &Self::SourceAsset) -> Option<usize> {
        match source.kind {
            TextureKind::Sampled => source.levels().map(|levels| levels.byte_len()),
            TextureKind::RenderTarget { .. } => None,
        }
    }
}

#[derive(Resource)]
pub struct DummyRenderTexture(pub(crate) RenderTexture);

impl DummyRenderTexture {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("DummyRenderTexture Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self(RenderTexture {
            texture,
            view,
            sampler,
        })
    }

    pub fn inner(&self) -> &RenderTexture {
        &self.0
    }
}

impl Deref for DummyRenderTexture {
    type Target = RenderTexture;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
