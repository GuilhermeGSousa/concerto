use concerto_ecs::resource::Resource;

use crate::{
    shader::{ShaderDefValue, ShaderDefs, NO_CUBE_ARRAY_TEXTURES_SUPPORT, SKY_RADIANCE_SQRT},
    sky_light::SKY_RADIANCE_SCALE,
};

/// GPU features the renderer adapts to, read from the adapter.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// Cube-map array textures can be sampled. `false` on WebGL2.
    pub cube_array_textures: bool,
    /// Depth bias can be clamped. `false` on WebGL2.
    pub depth_bias_clamp: bool,
    /// A single-layer texture can be viewed as an array. `false` on GL.
    pub single_layer_texture_arrays: bool,
    /// BC1 to BC7 textures can be sampled. `false` where the adapter lacks `TEXTURE_COMPRESSION_BC`.
    pub bc_textures: bool,
    /// `Rgba16Float` can be rendered to. `false` on WebGL2 without `EXT_color_buffer_float`.
    pub float_render_targets: bool,
}

impl RenderCapabilities {
    /// Reads the capabilities from `adapter`.
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        let is_gl = adapter.get_info().backend == wgpu::Backend::Gl;
        let is_webgl2 = is_gl && cfg!(target_arch = "wasm32");
        let downlevel = adapter.get_downlevel_capabilities().flags;

        Self {
            cube_array_textures: downlevel.contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES)
                && !is_webgl2,
            depth_bias_clamp: downlevel.contains(wgpu::DownlevelFlags::DEPTH_BIAS_CLAMP),
            single_layer_texture_arrays: !is_gl,
            bc_textures: adapter
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
            float_render_targets: adapter
                .get_texture_format_features(wgpu::TextureFormat::Rgba16Float)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        }
    }

    /// The format cameras draw their scene in: `Rgba16Float`, or `Rgba8UnormSrgb`, which clamps scene light at 1, where a float target can't be rendered to.
    pub fn view_target_format(&self) -> wgpu::TextureFormat {
        if self.float_render_targets {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        }
    }

    /// Whether a multisampled depth texture can be read in a shader. `false` on GL.
    pub fn multisampled_depth_sampling(&self) -> bool {
        self.single_layer_texture_arrays
    }

    /// The platform shader defs for these capabilities.
    pub fn shader_defs(&self) -> ShaderDefs {
        let mut defs = ShaderDefs::default();
        if !self.cube_array_textures {
            defs.define(NO_CUBE_ARRAY_TEXTURES_SUPPORT);
        }
        if !self.float_render_targets {
            defs.insert(SKY_RADIANCE_SQRT, ShaderDefValue::UInt(SKY_RADIANCE_SCALE));
        }
        defs
    }
}
