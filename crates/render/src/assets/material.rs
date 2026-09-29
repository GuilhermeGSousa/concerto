use bytemuck::{Pod, Zeroable};
use concerto_color::{Color, LinearRgba};
use concerto_foundation::assets::{
    asset_server::{AssetLoadContext, AssetServer},
    handle::AssetHandle,
    Asset, AssetId, LoadableAsset,
};
use concerto_render_macros::AsBindGroup;
use glam::Vec3;

use crate::{
    assets::texture::Texture,
    render_asset::{
        render_texture::{DummyRenderTexture, RenderTexture},
        AssetPreparationError, RenderAssets,
    },
    shader::ShaderDefs,
};

use bitflags::bitflags;

// ────────────────────────────────────────────────────────────────────────────
// ShaderRef — reference to a WGSL shader used by a material
// ────────────────────────────────────────────────────────────────────────────

/// A reference to a WGSL shader source.
///
/// Pass [`ShaderRef::Default`] to use the engine's built-in PBR shader, or
/// [`ShaderRef::Source`] to supply your own WGSL source string.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaderRef {
    /// Use the engine's built-in shader.
    Default,
    /// Use the provided WGSL source string.
    Source(&'static str),
}

// ────────────────────────────────────────────────────────────────────────────
// AsBindGroup — trait implemented (manually or via derive) by material types
// ────────────────────────────────────────────────────────────────────────────

/// Describes a type that can produce a wgpu bind-group layout and a bind-group
/// populated from its own data.
///
/// Implement this trait manually, or use `#[derive(AsBindGroup)]` from the
/// `render-macros` crate to generate the implementation automatically.
///
/// # Example (manual implementation)
///
/// ```rust,ignore
/// use concerto_render::assets::material::{AsBindGroup, ShaderRef};
///
/// pub struct MyMaterial {
///     pub tint: [f32; 4],
/// }
///
/// impl AsBindGroup for MyMaterial {
///     fn vertex_shader() -> ShaderRef { ShaderRef::Default }
///     fn fragment_shader() -> ShaderRef { ShaderRef::Source(include_str!("my.wgsl")) }
///
///     fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout { … }
///     fn create_bind_group(&self, …) -> Result<wgpu::BindGroup, …> { … }
/// }
/// ```
pub trait AsBindGroup {
    /// The vertex shader to use when rendering with this material.
    fn vertex_shader() -> ShaderRef
    where
        Self: Sized,
    {
        ShaderRef::Default
    }

    /// The fragment shader to use when rendering with this material.
    fn fragment_shader() -> ShaderRef
    where
        Self: Sized,
    {
        ShaderRef::Default
    }

    /// Build the `BindGroupLayout` that describes the shader bindings for this material.
    fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout
    where
        Self: Sized;

    /// Create a `BindGroup` populated with the material's actual GPU resources.
    fn create_bind_group(
        &self,
        device: &wgpu::Device,
        render_textures: &RenderAssets<RenderTexture>,
        dummy_texture: &DummyRenderTexture,
        layout: &wgpu::BindGroupLayout,
    ) -> Result<wgpu::BindGroup, AssetPreparationError>;
}

// ────────────────────────────────────────────────────────────────────────────
// StandardMaterial — the engine's built-in PBR material
// ────────────────────────────────────────────────────────────────────────────

/// The engine's built-in physically based (metallic/roughness) material.
///
/// Follows the glTF PBR conventions: the metallic-roughness texture stores
/// roughness in the green channel and metalness in the blue channel, and the
/// occlusion texture stores ambient occlusion in the red channel.  Texture
/// samples are multiplied by the corresponding scalar factors, so untextured
/// materials are fully described by the factors alone.
///
/// Use this as a starting point, or define your own material by implementing
/// [`AsBindGroup`] (manually or via `#[derive(AsBindGroup)]`).
#[derive(AsBindGroup, Default, serde::Serialize, serde::Deserialize)]
#[material(
    vertex_shader = include_str!("../shaders/shader.wgsl"),
    fragment_shader = include_str!("../shaders/shader.wgsl"),
    lighting = true,
    skeleton = true,
)]
pub struct StandardMaterial {
    #[texture(0)]
    #[sampler(1)]
    base_color_texture: Option<AssetHandle<Texture>>,

    #[texture(2)]
    #[sampler(3)]
    normal_texture: Option<AssetHandle<Texture>>,

    #[texture(4)]
    #[sampler(5)]
    metallic_roughness_texture: Option<AssetHandle<Texture>>,

    #[texture(6)]
    #[sampler(7)]
    emissive_texture: Option<AssetHandle<Texture>>,

    #[texture(8)]
    #[sampler(9)]
    occlusion_texture: Option<AssetHandle<Texture>>,

    #[uniform(10)]
    uniform: MaterialUniform,
}

impl StandardMaterial {
    pub fn new(
        base_color_texture: Option<AssetHandle<Texture>>,
        normal_texture: Option<AssetHandle<Texture>>,
    ) -> Self {
        let mut material = Self::default();
        if let Some(texture) = base_color_texture {
            material.set_base_color_texture(texture);
        }
        if let Some(texture) = normal_texture {
            material.set_normal_texture(texture);
        }
        material
    }

    pub fn with_base_color_factor(mut self, factor: Color) -> Self {
        self.set_base_color_factor(factor);
        self
    }

    pub fn with_metallic_roughness_texture(mut self, texture: AssetHandle<Texture>) -> Self {
        self.set_metallic_roughness_texture(texture);
        self
    }

    pub fn with_emissive_texture(mut self, texture: AssetHandle<Texture>) -> Self {
        self.set_emissive_texture(texture);
        self
    }

    pub fn with_occlusion_texture(mut self, texture: AssetHandle<Texture>) -> Self {
        self.set_occlusion_texture(texture);
        self
    }

    pub fn set_base_color_texture(&mut self, texture: AssetHandle<Texture>) {
        self.base_color_texture = Some(texture);
        self.uniform.flags |= MaterialFlags::HAS_BASE_COLOR_TEXTURE;
    }

    pub fn base_color_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.base_color_texture.as_ref()
    }

    #[deprecated(note = "renamed to `set_base_color_texture`")]
    pub fn set_diffuse_texture(&mut self, texture: AssetHandle<Texture>) {
        self.set_base_color_texture(texture);
    }

    #[deprecated(note = "renamed to `base_color_texture`")]
    pub fn diffuse_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.base_color_texture()
    }

    pub fn set_normal_texture(&mut self, texture: AssetHandle<Texture>) {
        self.normal_texture = Some(texture);
        self.uniform.flags |= MaterialFlags::HAS_NORMAL_TEXTURE;
    }

    pub fn normal_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.normal_texture.as_ref()
    }

    pub fn set_metallic_roughness_texture(&mut self, texture: AssetHandle<Texture>) {
        self.metallic_roughness_texture = Some(texture);
        self.uniform.flags |= MaterialFlags::HAS_METALLIC_ROUGHNESS_TEXTURE;
    }

    pub fn metallic_roughness_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.metallic_roughness_texture.as_ref()
    }

    pub fn set_emissive_texture(&mut self, texture: AssetHandle<Texture>) {
        self.emissive_texture = Some(texture);
        self.uniform.flags |= MaterialFlags::HAS_EMISSIVE_TEXTURE;
    }

    pub fn emissive_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.emissive_texture.as_ref()
    }

    pub fn set_occlusion_texture(&mut self, texture: AssetHandle<Texture>) {
        self.occlusion_texture = Some(texture);
        self.uniform.flags |= MaterialFlags::HAS_OCCLUSION_TEXTURE;
    }

    pub fn occlusion_texture(&self) -> Option<&AssetHandle<Texture>> {
        self.occlusion_texture.as_ref()
    }

    /// Multiplied with the base color texture (or used directly when no
    /// texture is set).  Linear RGBA; defaults to white.
    pub fn set_base_color_factor(&mut self, factor: Color) {
        self.uniform.base_color_factor = factor.to_linear();
    }

    pub fn base_color_factor(&self) -> Color {
        Color::from(self.uniform.base_color_factor)
    }

    /// Multiplied with the blue channel of the metallic-roughness texture.
    /// Defaults to `0.0`.
    pub fn set_metallic_factor(&mut self, factor: f32) {
        self.uniform.metallic_factor = factor;
    }

    pub fn metallic_factor(&self) -> f32 {
        self.uniform.metallic_factor
    }

    /// Multiplied with the green channel of the metallic-roughness texture.
    /// Defaults to `0.5`.
    pub fn set_roughness_factor(&mut self, factor: f32) {
        self.uniform.roughness_factor = factor;
    }

    pub fn roughness_factor(&self) -> f32 {
        self.uniform.roughness_factor
    }

    /// Multiplied with the emissive texture (or used directly when no texture
    /// is set).  Linear RGB; defaults to black (no emission).
    pub fn set_emissive_factor(&mut self, factor: Vec3) {
        self.uniform.emissive_factor = factor.to_array();
    }

    pub fn emissive_factor(&self) -> Vec3 {
        Vec3::from_array(self.uniform.emissive_factor)
    }

    /// Blends the occlusion texture's effect from none (`0.0`) to full
    /// (`1.0`, the default).
    pub fn set_occlusion_strength(&mut self, strength: f32) {
        self.uniform.occlusion_strength = strength;
    }

    pub fn occlusion_strength(&self) -> f32 {
        self.uniform.occlusion_strength
    }

    /// Enable alpha-cutout (mask) mode with the given threshold.
    /// Fragments whose base-color alpha is below `cutoff` are discarded.
    pub fn set_alpha_cutoff(&mut self, cutoff: f32) {
        self.uniform.alpha_cutoff = cutoff;
        self.uniform.flags |= MaterialFlags::ALPHA_CUTOUT;
    }

    pub fn with_alpha_cutoff(mut self, cutoff: f32) -> Self {
        self.set_alpha_cutoff(cutoff);
        self
    }

    /// Returns the alpha cutoff threshold if alpha-cutout mode is active.
    pub fn alpha_cutoff(&self) -> Option<f32> {
        if self.uniform.flags.contains(MaterialFlags::ALPHA_CUTOUT) {
            Some(self.uniform.alpha_cutoff)
        } else {
            None
        }
    }

    /// Scale UV coordinates for all texture samples.  Defaults to `[1.0, 1.0]`
    /// (no scaling).  Values greater than 1 tile the texture more densely.
    pub fn set_uv_scale(&mut self, scale: [f32; 2]) {
        self.uniform.uv_scale = scale;
    }

    pub fn with_uv_scale(mut self, scale: [f32; 2]) -> Self {
        self.set_uv_scale(scale);
        self
    }

    pub fn uv_scale(&self) -> [f32; 2] {
        self.uniform.uv_scale
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Asset loading for StandardMaterial
// ────────────────────────────────────────────────────────────────────────────

impl Asset for StandardMaterial {
    fn name() -> &'static str {
        "StandardMaterial"
    }

    fn referenced_sub_assets(&self) -> Vec<AssetId> {
        [
            &self.base_color_texture,
            &self.normal_texture,
            &self.metallic_roughness_texture,
            &self.emissive_texture,
            &self.occlusion_texture,
        ]
        .into_iter()
        .filter_map(|field| field.as_ref().map(|handle| handle.id()))
        .collect()
    }
}

impl StandardMaterial {
    /// Upgrades every `Weak` texture handle to a live `Strong` one via the
    /// given `AssetServer`. Defined in this module so it can reach the private
    /// texture fields directly.
    pub(crate) fn resolve_asset_handles(&mut self, asset_server: &AssetServer) {
        for handle in [
            &mut self.base_color_texture,
            &mut self.normal_texture,
            &mut self.metallic_roughness_texture,
            &mut self.emissive_texture,
            &mut self.occlusion_texture,
        ]
        .into_iter()
        .flatten()
        {
            *handle = asset_server.load(handle.id());
        }
    }
}

impl LoadableAsset for StandardMaterial {
    fn on_load(&mut self, context: &AssetLoadContext) -> anyhow::Result<()> {
        self.resolve_asset_handles(context.asset_server());
        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────────────────
// MaterialFlags / MaterialUniform (used by StandardMaterial)
// ────────────────────────────────────────────────────────────────────────────

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub struct MaterialFlags(u32);

bitflags! {
    impl MaterialFlags: u32 {
        const HAS_BASE_COLOR_TEXTURE = 1 << 0;
        const HAS_NORMAL_TEXTURE = 1 << 1;
        const HAS_METALLIC_ROUGHNESS_TEXTURE = 1 << 2;
        const HAS_EMISSIVE_TEXTURE = 1 << 3;
        const HAS_OCCLUSION_TEXTURE = 1 << 4;
        const ALPHA_CUTOUT = 1 << 5;
    }
}

/// GPU-side material parameters.  The field order and padding must match the
/// `MaterialUniform` struct in `shaders/shader.wgsl`.
///
/// Layout (64 bytes, 16-byte aligned):
/// ```text
///  0..16  base_color_factor  vec4<f32>
/// 16..28  emissive_factor    vec3<f32>
/// 28..32  metallic_factor    f32
/// 32..36  roughness_factor   f32
/// 36..40  occlusion_strength f32
/// 40..44  flags              u32
/// 44..48  alpha_cutoff       f32
/// 48..56  uv_scale           vec2<f32>
/// 56..64  _padding           vec2<u32>
/// ```
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, serde::Serialize, serde::Deserialize)]
pub(crate) struct MaterialUniform {
    base_color_factor: LinearRgba,
    emissive_factor: [f32; 3],
    metallic_factor: f32,
    roughness_factor: f32,
    occlusion_strength: f32,
    flags: MaterialFlags,
    alpha_cutoff: f32,
    uv_scale: [f32; 2],
    _padding: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<MaterialUniform>() == 64);

impl Default for MaterialUniform {
    fn default() -> Self {
        Self {
            base_color_factor: LinearRgba::WHITE,
            emissive_factor: [0.0; 3],
            metallic_factor: 0.0,
            roughness_factor: 0.5,
            occlusion_strength: 1.0,
            flags: MaterialFlags::empty(),
            alpha_cutoff: 0.5,
            uv_scale: [1.0, 1.0],
            _padding: [0; 2],
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Material — high-level trait that unifies all material types
// ────────────────────────────────────────────────────────────────────────────

/// High-level material trait.
///
/// Every renderable material type implements `Material`.  The trait extends
/// [`AsBindGroup`] with a set of optional *engine bind-group* flags that tell
/// [`crate::material_plugin::MaterialPlugin`] which built-in uniform groups
/// (`@group(1)` camera, `@group(2)` lighting, `@group(3)` skeleton) to include
/// in the pipeline layout and bind before each draw call.
///
/// Shaders only need to declare the `@group(N)` bindings that their material
/// actually uses.  Any group not requested via these flags is absent from the
/// pipeline layout entirely, so the WGSL source never has to reference it.
///
/// # Default flag values
///
/// | Flag              | Default  | Standard material |
/// |-------------------|----------|--------------------|
/// | `needs_camera()`  | `true`   | `true`             |
/// | `needs_lighting()`| `false`  | `true`             |
/// | `needs_skeleton()`| `false`  | `true`             |
///
/// The built-in group indices follow this fixed assignment:
///
/// | `@group(0)` | always the material's own bind group                             |
/// | `@group(1)` | camera uniform (present when `needs_camera()` is `true`)         |
/// | `@group(2)` | lighting uniform *and* both shadow-map arrays (present when `needs_lighting()` is `true`) |
/// | `@group(3)` | skeleton/bone uniform (present when `needs_skeleton()` is `true`)|
///
/// Lighting and shadow sampling share one bind group rather than being split
/// across several — wgpu only guarantees 4 bind groups (`max_bind_groups`),
/// and splitting them would push a material needing lighting+skeleton+shadows
/// past that limit. A material that only wants the lights uniform (and
/// ignores the shadow bindings) can still enable `needs_lighting()` alone and
/// simply not declare the shadow bindings in its own WGSL.
///
/// The flags are *independent* — each can be toggled on or off without implying
/// the others.  However, skipping an intermediate group creates a gap in the
/// bind-group slot numbering, which is not supported by wgpu.  To avoid this,
/// only declare a higher-numbered group active if all lower-numbered engine
/// groups are also active (e.g. `needs_skeleton = true` implies
/// `needs_lighting = true` and `needs_camera = true`).
pub trait Material: AsBindGroup + Asset + Send + Sync + 'static {
    /// Whether this material's vertex shader reads the built-in camera uniform
    /// at `@group(1) @binding(0)`.
    ///
    /// Almost all 3-D materials need this for the view-projection matrix.
    /// Defaults to `true`.
    fn needs_camera() -> bool
    where
        Self: Sized,
    {
        true
    }

    /// Whether this material's shaders use the built-in lighting uniform and
    /// shadow-map arrays at `@group(2)`.
    ///
    /// Defaults to `false`.  Enable for Phong / PBR materials. A material can
    /// read just the lights uniform and ignore the shadow-map bindings if it
    /// doesn't want shadows — this flag controls the whole group's presence
    /// in the pipeline layout, not each binding independently.
    fn needs_lighting() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Whether this material's vertex shader reads the built-in skeleton
    /// (bone) uniform at `@group(3) @binding(0)`.
    ///
    /// Defaults to `false`.  Enable only for skinned-mesh materials.
    fn needs_skeleton() -> bool
    where
        Self: Sized,
    {
        false
    }

    /// The cull mode to use when rendering this material.
    ///
    /// Defaults to `Some(wgpu::Face::Back)` (back-face culling, suitable for
    /// all standard outward-facing meshes).  Override with
    /// `Some(wgpu::Face::Front)` for materials that must cull front faces
    /// (e.g. skybox cube interiors), or `None` to disable culling.
    fn cull_mode() -> Option<wgpu::Face>
    where
        Self: Sized,
    {
        Some(wgpu::Face::Back)
    }

    /// The vertex buffer layouts used by this material's render pipeline.
    ///
    /// Defaults to the standard mesh layout (`Vertex` + `GlobalTransformRaw`
    /// instance data).  Override for materials that use a different vertex
    /// format – for example, the skybox uses `SkyboxVertex` and the UI uses
    /// `UIVertex`.
    fn vertex_layouts() -> Vec<wgpu::VertexBufferLayout<'static>>
    where
        Self: Sized,
    {
        use crate::assets::vertex::{Vertex, VertexBufferLayout};
        use concerto_foundation::transform::GlobalTransformRaw;
        vec![Vertex::describe(), GlobalTransformRaw::describe()]
    }

    /// The depth/stencil state to use when rendering this material.
    ///
    /// Defaults to depth testing and writing with `Depth32Float`.  Override
    /// with `None` for materials that must not interact with the depth buffer
    /// (e.g. skybox and UI render passes).
    fn depth_stencil() -> Option<wgpu::DepthStencilState>
    where
        Self: Sized,
    {
        Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        })
    }

    fn topology() -> wgpu::PrimitiveTopology
    where
        Self: Sized,
    {
        wgpu::PrimitiveTopology::TriangleList
    }

    /// Whether this material's render pass should clear the depth buffer to 1.0
    /// before drawing.
    ///
    /// Defaults to `true`. Set to `false` for materials that render on top of
    /// another pass (e.g. a material that always follows a skybox or opaque
    /// pre-pass) so that depth values from earlier passes are preserved.
    fn clear_depth() -> bool
    where
        Self: Sized,
    {
        true
    }

    fn blend_state() -> Option<wgpu::BlendState>
    where
        Self: Sized,
    {
        Some(wgpu::BlendState::REPLACE)
    }

    /// Adds shader defs on top of the platform defs. Defaults to none.
    fn shader_defs(defs: &mut ShaderDefs)
    where
        Self: Sized,
    {
        let _ = defs;
    }
}
