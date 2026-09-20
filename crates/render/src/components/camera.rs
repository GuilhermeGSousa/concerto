use app::extractor::Extracted;
use color::{Color, LinearRgba};
use encase::{ShaderType, UniformBuffer};
use essential::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
    transform::GlobalTransform,
};

use ecs::{
    command::CommandQueue,
    component::scene::{SceneComponent, SceneSpawnContext},
    component::Component,
    query::Query,
    resource::{Res, ResMut},
    Entity,
};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use wgpu::util::DeviceExt;
use window::plugin::Window;

use crate::{
    assets::texture::Texture,
    components::render_entity::RenderEntity,
    device::RenderDevice,
    layouts::CameraLayout,
    queue::RenderQueue,
    render_asset::{render_texture::RenderTexture, RenderAssets},
    resources::RenderContext,
};

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Mat4 = Mat4::from_cols_array(
    &[
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0
    ]
);

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub enum RenderTarget {
    MainWindow,
    Texture(AssetHandle<Texture>),
}

#[allow(dead_code)]
impl RenderTarget {
    pub fn main_window() -> Self {
        RenderTarget::MainWindow
    }

    pub fn texture(handle: AssetHandle<Texture>) -> Self {
        RenderTarget::Texture(handle)
    }
}

#[derive(Component, Serialize, Deserialize)]
pub struct Camera {
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
    pub clear_color: Color,
    pub render_target: RenderTarget,
}

impl SceneComponent for Camera {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let RenderTarget::Texture(handle) = &self.render_target {
            if let Some(server) = ctx.get_resource::<AssetServer>() {
                self.render_target = RenderTarget::Texture(server.load(handle.id()));
            }
        }
        ctx.insert(self, entity);
    }
}

impl Camera {
    /// Creates a perspective camera with the given field-of-view (radians) and aspect ratio.
    /// Uses sensible defaults for near/far planes and clear colour.
    pub fn perspective(fovy: f32, aspect: f32) -> Self {
        Self {
            fovy,
            aspect,
            ..Default::default()
        }
    }

    pub fn build_projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar)
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            aspect: 1.0,
            fovy: std::f32::consts::FRAC_PI_4,
            znear: 0.1,
            zfar: 100.0,
            clear_color: Color::rgba(0.118, 0.831, 0.922, 1.0),
            render_target: RenderTarget::main_window(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, ShaderType)]
pub struct CameraUniform {
    view_pos: Vec3,
    view_proj: Mat4,
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_pos: Vec3::ZERO,
            view_proj: Mat4::IDENTITY,
        }
    }

    pub fn update_view_proj(&mut self, camera: &Camera, transform: &GlobalTransform) {
        self.view_pos = transform.translation();
        self.view_proj = camera.build_projection_matrix() * transform.matrix().inverse();
    }
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Component)]
pub struct RenderCamera {
    pub(crate) clear_color: LinearRgba,
    pub camera_bind_group: wgpu::BindGroup,
    pub camera_uniform: CameraUniform,
    pub camera_buffer: wgpu::Buffer,
    pub(crate) depth_texture: RenderTexture,
    pub render_target: Option<RenderTexture>,
}

impl RenderCamera {
    pub fn depth_texture(&self) -> &RenderTexture {
        &self.depth_texture
    }

    pub fn resize_render_target(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        if self.render_target.is_some() {
            self.render_target = Some(create_rtt(device, format, width, height));
        }
        self.depth_texture =
            RenderTexture::create_depth_texture(device, width, height, "depth_texture");
    }
}

fn publish_render_target(
    camera: &Camera,
    render_camera: &RenderCamera,
    render_textures: &mut RenderAssets<RenderTexture>,
) {
    let RenderTarget::Texture(handle) = &camera.render_target else {
        return;
    };
    let Some(target) = render_camera.render_target.as_ref() else {
        return;
    };

    render_textures.insert(handle.id(), target.clone());
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_cameras(
    cameras: Extracted<Query<(&Camera, &GlobalTransform, &RenderEntity)>>,
    render_cameras: Query<&mut RenderCamera>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    context: Res<RenderContext>,
    camera_layouts: Res<CameraLayout>,
    texture_assets: Extracted<Res<AssetStore<Texture>>>,
    mut render_textures: ResMut<RenderAssets<RenderTexture>>,
    queue: Res<RenderQueue>,
) {
    for (camera, transform, render_entity) in cameras.iter() {
        let render_entity = **render_entity;

        let mut camera_uniform = CameraUniform::new();
        camera_uniform.update_view_proj(camera, transform);

        if let Some(mut render_camera) = render_cameras.get_entity(render_entity) {
            render_camera.camera_uniform = camera_uniform;
            render_camera.clear_color = camera.clear_color.to_linear();

            if let RenderTarget::Texture(handle) = &camera.render_target {
                if let Some(texture) = texture_assets.get(handle) {
                    let requested = texture.size();
                    let current = render_camera
                        .render_target
                        .as_ref()
                        .map(|target| target.texture.size());
                    if current.is_none_or(|size| {
                        size.width != requested.width || size.height != requested.height
                    }) {
                        render_camera.resize_render_target(
                            &device,
                            context.surface_config.format,
                            requested.width.max(1),
                            requested.height.max(1),
                        );
                    }
                }
            }
            publish_render_target(camera, &render_camera, &mut render_textures);

            let mut buffer = UniformBuffer::new(Vec::new());
            buffer.write(&render_camera.camera_uniform).unwrap();
            queue.write_buffer(&render_camera.camera_buffer, 0, &buffer.into_inner());

            continue;
        }

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(&camera_uniform).unwrap();
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: &buffer.into_inner(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &camera_layouts.camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("camera_bind_group"),
        });

        let render_target: Option<RenderTexture> = match &camera.render_target {
            RenderTarget::Texture(handle) => {
                let texture = texture_assets
                    .get(handle)
                    .expect("Texture asset for camera render target not found in AssetStore");
                let size = texture.size();
                Some(create_rtt(
                    &device,
                    context.surface_config.format,
                    size.width.max(1),
                    size.height.max(1),
                ))
            }
            RenderTarget::MainWindow => None,
        };

        let (depth_w, depth_h) = match &render_target {
            Some(rt) => {
                let size = rt.texture.size();
                (size.width, size.height)
            }
            None => (
                context.surface_config.width.max(1),
                context.surface_config.height.max(1),
            ),
        };
        let depth_texture =
            RenderTexture::create_depth_texture(&device, depth_w, depth_h, "depth_texture");

        let render_cam = RenderCamera {
            camera_bind_group,
            camera_uniform,
            camera_buffer,
            depth_texture,
            clear_color: camera.clear_color.to_linear(),
            render_target,
        };
        publish_render_target(camera, &render_cam, &mut render_textures);

        cmd.insert(render_cam, render_entity);
    }
}

/// Creates a single [`RenderTexture`] for use as a camera render target.
///
/// The texture is owned exclusively by [`RenderCamera::color_target`].
/// Viewports that want to display this camera's output look up the matching
/// camera by handle ID and borrow the view directly — no shared allocation needed.
pub fn create_rtt(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> RenderTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Camera RTT"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("RTT Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    RenderTexture {
        texture,
        view,
        sampler,
    }
}

/// Keeps every main-window [`Camera`]'s aspect ratio matched to the window size.
///
/// Runs in the main world, where [`Camera`] lives. The window is the
/// authoritative size source; the render world's surface is reconfigured
/// downstream of it, so there's no need to reach across worlds for it.
pub(crate) fn sync_camera_aspect(cameras: Query<&mut Camera>, window: Res<Window>) {
    let (width, height) = window.size();
    if width == 0 || height == 0 {
        return;
    }

    let aspect = width as f32 / height as f32;
    for mut camera in cameras.iter() {
        if matches!(camera.render_target, RenderTarget::MainWindow) && camera.aspect != aspect {
            camera.aspect = aspect;
        }
    }
}
