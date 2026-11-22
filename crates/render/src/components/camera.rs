use concerto_app::extractor::Extracted;
use concerto_color::{Color, LinearRgba};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    transform::GlobalTransform,
};
use concerto_reflect::ReflectDefault;
use encase::{ShaderType, UniformBuffer};

use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    query::{change_detection::DetectChanges, Query},
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{Res, ResMut, Resource},
    Entity,
};
use concerto_window::plugin::Window;
use glam::{Mat4, UVec2, Vec2, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::components::visibility::RenderLayers;
use crate::{
    assets::texture::Texture,
    assets::texture::TextureRole,
    components::{
        camera_environment::{CameraEnvironment, DEFAULT_AMBIENT_INTENSITY},
        render_entity::RenderEntity,
    },
    device::RenderDevice,
    layouts::CameraLayout,
    queue::RenderQueue,
    render_asset::{
        render_texture::{FallbackTextures, RenderTexture},
        render_window::RenderWindow,
        RenderAssets,
    },
    resources::RenderContext,
    sky_light::{CameraSkyLight, SkyRadianceMap},
    tonemapping::{Exposure, Tonemapping},
    view_target::{Msaa, ViewTarget},
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
#[derive(concerto_reflect::Reflect, Default)]
pub enum RenderTarget {
    #[default]
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

/// The part of its target a camera draws to, in fractions of the target's size from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, concerto_reflect::Reflect)]
#[reflect(Default)]
pub struct Viewport {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            min: Vec2::ZERO,
            max: Vec2::ONE,
        }
    }
}

impl Viewport {
    /// A viewport from `min` to `max`, in fractions of the target.
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// The pixels this viewport covers on a target of `target` size.
    pub fn to_pixels(&self, target: UVec2) -> PixelRect {
        if !self.min.is_finite() || !self.max.is_finite() {
            return PixelRect::default();
        }
        let x0 = edge(self.min.x, target.x, 0);
        let x1 = edge(self.max.x, target.x, x0);
        let y0 = edge(self.min.y, target.y, 0);
        let y1 = edge(self.max.y, target.y, y0);
        PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

fn edge(fraction: f32, size: u32, floor: u32) -> u32 {
    let pixel = (fraction * size as f32).round().clamp(0.0, size as f32) as u32;
    pixel.clamp(floor, size)
}

/// A rect on a render target in physical pixels, from its top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    /// True when the rect covers no pixel.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Width over height, or `None` for an empty rect.
    pub fn aspect(&self) -> Option<f32> {
        (!self.is_empty()).then(|| self.width as f32 / self.height as f32)
    }

    /// Restricts a render pass's viewport and scissor rect to this rect.
    pub fn apply(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_viewport(
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(self.x, self.y, self.width, self.height);
    }
}

/// The pixels a camera with `viewport` covers on a target of `target` size; the whole target for `None`.
pub fn viewport_rect(viewport: Option<&Viewport>, target: UVec2) -> PixelRect {
    match viewport {
        Some(viewport) => viewport.to_pixels(target),
        None => PixelRect {
            x: 0,
            y: 0,
            width: target.x,
            height: target.y,
        },
    }
}

#[derive(Component, concerto_reflect::Reflect)]
#[reflect(Component, SceneComponent)]
#[require(crate::components::render_entity::SyncWithRenderWorld)]
pub struct Camera {
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
    pub clear_color: Color,
    pub render_target: RenderTarget,
    pub viewport: Option<Viewport>,
}

impl Camera {
    /// The colour a camera clears to unless told otherwise, the editor viewport's background.
    pub const DEFAULT_CLEAR_COLOR: Color = Color::srgba(0.086, 0.094, 0.157, 1.0);

    /// Creates a perspective camera with the given field-of-view (radians) and aspect ratio.
    /// Uses sensible defaults for near/far planes and clear colour.
    pub fn perspective(fovy: f32, aspect: f32) -> Self {
        Self {
            fovy,
            aspect,
            ..Default::default()
        }
    }

    /// Draws this camera to `viewport` of its target instead of the whole of it.
    pub fn with_viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = Some(viewport);
        self
    }

    /// The pixels this camera covers on a target of `target` size.
    pub fn viewport_rect(&self, target: UVec2) -> PixelRect {
        viewport_rect(self.viewport.as_ref(), target)
    }

    pub fn build_projection_matrix(&self) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(self.fovy, self.aspect, self.znear, self.zfar)
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            aspect: 1.0,
            fovy: std::f32::consts::FRAC_PI_4,
            znear: 0.1,
            zfar: 100.0,
            clear_color: Self::DEFAULT_CLEAR_COLOR,
            render_target: RenderTarget::main_window(),
            viewport: None,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, ShaderType)]
pub struct CameraUniform {
    view_pos: Vec3,
    view_proj: Mat4,
    pub(crate) fog_color: Vec4,
    pub(crate) fog_params: Vec4,
    pub(crate) ambient: Vec4,
    inverse_view_proj: Mat4,
    pub(crate) sky_irradiance: [Vec4; 9],
    pub(crate) night_color: Vec4,
    pub(crate) night_ambient: Vec4,
    pub(crate) sky_lit: u32,
    pub(crate) exposure: f32,
    pub(crate) tonemapping: u32,
}

impl CameraUniform {
    pub fn new() -> Self {
        Self {
            view_pos: Vec3::ZERO,
            view_proj: Mat4::IDENTITY,
            fog_color: Vec4::ZERO,
            fog_params: Vec4::ZERO,
            ambient: Vec3::splat(DEFAULT_AMBIENT_INTENSITY).extend(1.0),
            inverse_view_proj: Mat4::IDENTITY,
            sky_irradiance: [Vec4::ZERO; 9],
            night_color: Vec4::ZERO,
            night_ambient: Vec4::ZERO,
            sky_lit: 0,
            exposure: 1.0,
            tonemapping: Tonemapping::default().shader_value(),
        }
    }

    pub(crate) fn set_display(&mut self, tonemapping: Tonemapping, exposure: Exposure) {
        self.tonemapping = tonemapping.shader_value();
        self.exposure = exposure.multiplier();
    }

    pub fn update_view_proj(
        &mut self,
        camera: &Camera,
        environment: Option<&CameraEnvironment>,
        sky_light: Option<&CameraSkyLight>,
        transform: &GlobalTransform,
    ) {
        self.view_pos = transform.translation();
        self.view_proj = camera.build_projection_matrix() * transform.matrix().inverse();
        self.inverse_view_proj = self.view_proj.inverse();
        environment.copied().unwrap_or_default().fill(self);
        self.fill_sky_light(sky_light);
    }

    fn fill_sky_light(&mut self, sky_light: Option<&CameraSkyLight>) {
        let Some(sky_light) = sky_light else {
            self.sky_irradiance = [Vec4::ZERO; 9];
            self.night_color = Vec4::ZERO;
            self.night_ambient = Vec4::ZERO;
            self.sky_lit = 0;
            return;
        };
        self.sky_irradiance = sky_light
            .irradiance
            .coefficients
            .map(|coefficient| coefficient.extend(0.0));
        self.night_color = Vec4::from_array(sky_light.night_color.to_array());
        self.night_ambient = Vec4::from_array(sky_light.night_ambient.to_array());
        self.sky_lit = 1;
    }

    /// The matrix from world space to this camera's clip space.
    pub fn view_proj(&self) -> Mat4 {
        self.view_proj
    }

    /// Where the camera is in world space.
    pub fn view_position(&self) -> Vec3 {
        self.view_pos
    }

    /// The matrix from this camera's clip space to world space.
    pub fn inverse_view_proj(&self) -> Mat4 {
        self.inverse_view_proj
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
    /// The HDR colour and depth textures this camera's scene is drawn into.
    pub view_target: ViewTarget,
    pub render_target: Option<RenderTexture>,
    pub(crate) viewport: Option<Viewport>,
    pub(crate) layers: RenderLayers,
    pub(crate) ambient_occlusion: Option<wgpu::TextureView>,
    pub(crate) adapted_exposure: Option<wgpu::TextureView>,
    pub(crate) msaa: Msaa,
}

impl RenderCamera {
    pub fn depth_texture(&self) -> &RenderTexture {
        self.view_target.depth_texture()
    }

    /// The layers this camera draws.
    pub fn layers(&self) -> RenderLayers {
        self.layers
    }

    /// The view the finished frame is written to: this camera's texture, or the window's.
    pub fn output_view<'a>(&'a self, window: &'a RenderWindow) -> Option<&'a wgpu::TextureView> {
        match &self.render_target {
            Some(target) => Some(&target.view),
            None => window.get_view(),
        }
    }

    /// The size of the target this camera draws to: its texture, or the window's surface.
    pub fn target_size(&self, context: &RenderContext) -> UVec2 {
        match &self.render_target {
            Some(target) => {
                let size = target.texture.size();
                UVec2::new(size.width, size.height)
            }
            None => UVec2::new(context.surface_config.width, context.surface_config.height),
        }
    }

    /// The pixels this camera covers on a target of `target` size.
    pub fn viewport_rect(&self, target: UVec2) -> PixelRect {
        viewport_rect(self.viewport.as_ref(), target)
    }

    pub(crate) fn rebind(
        &mut self,
        device: &wgpu::Device,
        layout: &CameraLayout,
        sky_radiance: &SkyRadianceMap,
        fallbacks: &FallbackTextures,
    ) {
        self.camera_bind_group = camera_bind_group(
            device,
            layout,
            &self.camera_buffer,
            sky_radiance,
            self.ambient_occlusion
                .as_ref()
                .unwrap_or(&fallbacks.for_role(TextureRole::Data).view),
        );
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
    }
}

fn scene_clear_color(shown: Color, tonemapping: Tonemapping, exposure: Exposure) -> LinearRgba {
    let shown = shown.to_linear();
    let light =
        tonemapping.scene_light(Vec3::new(shown.r, shown.g, shown.b)) / exposure.multiplier();
    LinearRgba {
        r: light.x,
        g: light.y,
        b: light.z,
        a: shown.a,
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

/// While present, only the named camera is extracted into the render world.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExclusiveCamera(pub Entity);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn extract_cameras(
    exclusive: Extracted<Option<Res<ExclusiveCamera>>>,
    cameras: Extracted<
        Query<(
            Entity,
            &Camera,
            Option<&CameraEnvironment>,
            Option<&CameraSkyLight>,
            Option<&RenderLayers>,
            Option<&Tonemapping>,
            Option<&Exposure>,
            Option<&Msaa>,
            &GlobalTransform,
            &RenderEntity,
        )>,
    >,
    render_cameras: Query<&mut RenderCamera>,
    mut cmd: CommandQueue,
    device: Res<RenderDevice>,
    context: Res<RenderContext>,
    camera_layouts: Res<CameraLayout>,
    texture_assets: Extracted<Res<AssetStore<Texture>>>,
    mut render_textures: ResMut<RenderAssets<RenderTexture>>,
    queue: Res<RenderQueue>,
    sky_radiance: Res<SkyRadianceMap>,
    fallbacks: Res<FallbackTextures>,
) {
    for (
        entity,
        camera,
        environment,
        sky_light,
        layers,
        tonemapping,
        exposure,
        msaa,
        transform,
        render_entity,
    ) in cameras.iter()
    {
        let layers = layers.copied().unwrap_or_default();
        let msaa = msaa.copied().unwrap_or_default();
        if exclusive.as_ref().is_some_and(|only| only.0 != entity) {
            continue;
        }
        let render_entity = **render_entity;

        let mut camera_uniform = CameraUniform::new();
        camera_uniform.update_view_proj(camera, environment, sky_light, transform);
        let tonemapping = tonemapping.copied().unwrap_or_default();
        let exposure = exposure.copied().unwrap_or_default();
        camera_uniform.set_display(tonemapping, exposure);
        let clear_color = scene_clear_color(camera.clear_color, tonemapping, exposure);

        if let Some(mut render_camera) = render_cameras.get_entity(render_entity) {
            render_camera.camera_uniform = camera_uniform;
            render_camera.clear_color = clear_color;
            render_camera.viewport = camera.viewport;
            render_camera.layers = layers;
            render_camera.msaa = msaa;

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

        let target_size = match &camera.render_target {
            RenderTarget::Texture(handle) => match texture_assets.get(handle) {
                Some(texture) => Some(texture.size()),
                None => continue,
            },
            RenderTarget::MainWindow => None,
        };

        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(&camera_uniform).unwrap();
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: &buffer.into_inner(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = camera_bind_group(
            &device,
            &camera_layouts,
            &camera_buffer,
            &sky_radiance,
            &fallbacks.for_role(TextureRole::Data).view,
        );

        let render_target: Option<RenderTexture> = target_size.map(|size| {
            create_rtt(
                &device,
                context.surface_config.format,
                size.width.max(1),
                size.height.max(1),
            )
        });

        let view_size = match &render_target {
            Some(rt) => {
                let size = rt.texture.size();
                UVec2::new(size.width, size.height)
            }
            None => UVec2::new(context.surface_config.width, context.surface_config.height),
        };
        let view_target = ViewTarget::new(&device, &context, view_size, msaa);

        let render_cam = RenderCamera {
            camera_bind_group,
            camera_uniform,
            camera_buffer,
            view_target,
            clear_color,
            render_target,
            viewport: camera.viewport,
            layers,
            ambient_occlusion: None,
            adapted_exposure: None,
            msaa,
        };
        publish_render_target(camera, &render_cam, &mut render_textures);

        cmd.insert(render_cam, render_entity);
    }
}

fn camera_bind_group(
    device: &wgpu::Device,
    layout: &CameraLayout,
    camera_buffer: &wgpu::Buffer,
    sky_radiance: &SkyRadianceMap,
    ambient_occlusion: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layout.camera_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&sky_radiance.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sky_radiance.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(ambient_occlusion),
            },
        ],
        label: Some("camera_bind_group"),
    })
}

pub(crate) fn rebind_sky_radiance(
    sky_radiance: Res<SkyRadianceMap>,
    render_cameras: Query<&mut RenderCamera>,
    device: Res<RenderDevice>,
    camera_layouts: Res<CameraLayout>,
    fallbacks: Res<FallbackTextures>,
) {
    if !sky_radiance.is_changed() {
        return;
    }
    for mut render_camera in render_cameras.iter() {
        render_camera.rebind(&device, &camera_layouts, &sky_radiance, &fallbacks);
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
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
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

/// Keeps every main-window [`Camera`]'s aspect ratio matched to its viewport on the window.
///
/// Runs in the main world, where [`Camera`] lives. The window is the
/// authoritative size source; the render world's surface is reconfigured
/// downstream of it, so there's no need to reach across worlds for it.
pub(crate) fn sync_camera_aspect(cameras: Query<&mut Camera>, window: Res<Window>) {
    let target = UVec2::from(window.size());
    for mut camera in cameras.iter() {
        if !matches!(camera.render_target, RenderTarget::MainWindow) {
            continue;
        }
        let Some(aspect) = camera.viewport_rect(target).aspect() else {
            continue;
        };
        if camera.aspect != aspect {
            camera.aspect = aspect;
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{Entity, IntoSystem, System, World};
    use concerto_foundation::assets::{handle::AssetHandle, AssetId};
    use concerto_window::plugin::Window;
    use encase::ShaderType;
    use glam::{Mat4, UVec2, Vec2, Vec3, Vec4};

    use super::{
        sync_camera_aspect, viewport_rect, Camera, CameraUniform, PixelRect, RenderTarget, Viewport,
    };
    use crate::layout_test::{float_words, uniform_words};

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn camera_uniform_keeps_its_wgsl_layout() {
        let uniform = CameraUniform {
            view_pos: Vec3::new(1.0, 2.0, 3.0),
            view_proj: Mat4::from_cols_array(&std::array::from_fn(|index| 10.0 + index as f32)),
            fog_color: Vec4::new(30.0, 31.0, 32.0, 33.0),
            fog_params: Vec4::new(40.0, 41.0, 42.0, 43.0),
            ambient: Vec4::new(50.0, 51.0, 52.0, 53.0),
            inverse_view_proj: Mat4::from_cols_array(&std::array::from_fn(|index| {
                60.0 + index as f32
            })),
            sky_irradiance: std::array::from_fn(|index| Vec4::splat(80.0 + index as f32)),
            night_color: Vec4::new(90.0, 91.0, 92.0, 93.0),
            night_ambient: Vec4::new(94.0, 95.0, 96.0, 97.0),
            sky_lit: 1,
            exposure: 2.0,
            tonemapping: 3,
        };

        let mut expected = float_words(&[1.0, 2.0, 3.0, 0.0]);
        expected.extend(float_words(&std::array::from_fn::<f32, 16, _>(|index| {
            10.0 + index as f32
        })));
        expected.extend(float_words(&[30.0, 31.0, 32.0, 33.0]));
        expected.extend(float_words(&[40.0, 41.0, 42.0, 43.0]));
        expected.extend(float_words(&[50.0, 51.0, 52.0, 53.0]));
        expected.extend(float_words(&std::array::from_fn::<f32, 16, _>(|index| {
            60.0 + index as f32
        })));
        for index in 0..9 {
            expected.extend(float_words(&[80.0 + index as f32; 4]));
        }
        expected.extend(float_words(&[90.0, 91.0, 92.0, 93.0]));
        expected.extend(float_words(&[94.0, 95.0, 96.0, 97.0]));
        expected.extend([1, 2.0f32.to_bits(), 3, 0]);

        assert_eq!(uniform_words(&uniform), expected);
        assert_eq!(uniform.size().get(), 384);
    }

    struct Case {
        case: &'static str,
        viewport: Option<Viewport>,
        target: UVec2,
        rect: PixelRect,
        aspect: Option<f32>,
    }

    #[test]
    fn a_viewport_covers_its_fraction_of_the_target_in_whole_pixels() {
        let cases = [
            Case {
                case: "the left half of an odd width",
                viewport: Some(Viewport::new(Vec2::ZERO, Vec2::new(0.5, 1.0))),
                target: UVec2::new(1281, 720),
                rect: rect(0, 0, 641, 720),
                aspect: Some(641.0 / 720.0),
            },
            Case {
                case: "the right half of an odd width",
                viewport: Some(Viewport::new(Vec2::new(0.5, 0.0), Vec2::ONE)),
                target: UVec2::new(1281, 720),
                rect: rect(641, 0, 640, 720),
                aspect: Some(640.0 / 720.0),
            },
            Case {
                case: "no viewport",
                viewport: None,
                target: UVec2::new(1280, 720),
                rect: rect(0, 0, 1280, 720),
                aspect: Some(1280.0 / 720.0),
            },
        ];
        for case in &cases {
            let pixels = viewport_rect(case.viewport.as_ref(), case.target);
            assert_eq!(pixels, case.rect, "{}", case.case);
            assert_eq!(pixels.is_empty(), case.aspect.is_none(), "{}", case.case);
            assert_eq!(pixels.aspect(), case.aspect, "{}", case.case);
        }
    }

    fn sync(world: &mut World) {
        let mut system = sync_camera_aspect.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn aspect(world: &World, camera: Entity) -> f32 {
        world
            .get_component_for_entity::<Camera>(camera)
            .unwrap()
            .aspect
    }

    #[test]
    fn aspect_follows_the_viewport() {
        let mut world = World::new();
        world.insert_resource(Window::new((1280, 720), 1.0));
        let left_half = world
            .spawn(Camera::default().with_viewport(Viewport::new(Vec2::ZERO, Vec2::new(0.5, 1.0))));
        let whole = world.spawn(Camera::default());
        let texture = world.spawn(
            Camera {
                aspect: 0.25,
                render_target: RenderTarget::texture(AssetHandle::weak(AssetId::from_path(
                    "targets/minimap#texture",
                ))),
                ..Camera::default()
            }
            .with_viewport(Viewport::new(Vec2::ZERO, Vec2::new(0.5, 1.0))),
        );

        sync(&mut world);

        assert_eq!(aspect(&world, left_half), 640.0 / 720.0);
        assert_eq!(aspect(&world, whole), 1280.0 / 720.0);
        assert_eq!(aspect(&world, texture), 0.25);

        world.insert_resource(Window::new((1000, 1000), 1.0));
        sync(&mut world);

        assert_eq!(aspect(&world, left_half), 500.0 / 1000.0);
        assert_eq!(aspect(&world, texture), 0.25);

        world
            .get_component_for_entity_mut::<Camera>(left_half)
            .unwrap()
            .viewport = Some(Viewport::new(Vec2::ZERO, Vec2::new(0.25, 1.0)));
        sync(&mut world);
        assert_eq!(aspect(&world, left_half), 250.0 / 1000.0);
    }
}
