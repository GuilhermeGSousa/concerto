use concerto_ecs::resource::Resource;

use crate::shader::{ShaderDefs, NO_CUBE_ARRAY_TEXTURES_SUPPORT};

/// What the active GPU backend can do, wherever the renderer has to adapt to
/// it. Captured from the adapter by [`RenderPlugin`](crate::plugin::RenderPlugin)
/// and inserted into the render world.
///
/// Everything here is `true` on the native Vulkan/Metal/DX12 backends; the
/// gaps are WebGL2's (and, for `single_layer_texture_arrays`, any GL's).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// Cube-map *array* textures can be sampled. `false` on WebGL2, whose
    /// GLSL ES 3.00 has no `samplerCubeArray`.
    pub cube_array_textures: bool,
    /// Pipelines can clamp depth bias (`DepthBiasState::clamp != 0`).
    pub depth_bias_clamp: bool,
    /// A single-layer texture can be viewed as a 2D (or cube) array. `false`
    /// on GL, where wgpu picks the texture's GL target once, at creation,
    /// from its layer count: one layer makes a `TEXTURE_2D`, which can't then
    /// be sampled through a `texture_2d_array` binding.
    pub single_layer_texture_arrays: bool,
}

impl RenderCapabilities {
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        let is_gl = adapter.get_info().backend == wgpu::Backend::Gl;
        // wgpu's GL backend reports CUBE_ARRAY_TEXTURES unconditionally, but
        // WebGL2 shaders are GLSL ES 3.00, which can't declare one.
        let is_webgl2 = is_gl && cfg!(target_arch = "wasm32");
        let downlevel = adapter.get_downlevel_capabilities().flags;

        Self {
            cube_array_textures: downlevel.contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES)
                && !is_webgl2,
            depth_bias_clamp: downlevel.contains(wgpu::DownlevelFlags::DEPTH_BIAS_CLAMP),
            single_layer_texture_arrays: !is_gl,
        }
    }

    /// The platform shader defs every engine shader is preprocessed with.
    pub fn shader_defs(&self) -> ShaderDefs {
        let mut defs = ShaderDefs::default();
        if !self.cube_array_textures {
            defs.define(NO_CUBE_ARRAY_TEXTURES_SUPPORT);
        }
        defs
    }
}
