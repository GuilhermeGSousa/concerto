use concerto_ecs::resource::Resource;

use crate::shader::{ShaderDefs, NO_CUBE_ARRAY_TEXTURES_SUPPORT};

/// GPU features the renderer adapts to, read from the adapter.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCapabilities {
    /// Cube-map array textures can be sampled. `false` on WebGL2.
    pub cube_array_textures: bool,
    /// Depth bias can be clamped. `false` on WebGL2.
    pub depth_bias_clamp: bool,
    /// A single-layer texture can be viewed as an array. `false` on GL.
    pub single_layer_texture_arrays: bool,
}

impl RenderCapabilities {
    pub fn from_adapter(adapter: &wgpu::Adapter) -> Self {
        let is_gl = adapter.get_info().backend == wgpu::Backend::Gl;
        let is_webgl2 = is_gl && cfg!(target_arch = "wasm32");
        let downlevel = adapter.get_downlevel_capabilities().flags;

        Self {
            cube_array_textures: downlevel.contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES)
                && !is_webgl2,
            depth_bias_clamp: downlevel.contains(wgpu::DownlevelFlags::DEPTH_BIAS_CLAMP),
            single_layer_texture_arrays: !is_gl,
        }
    }

    /// The platform shader defs for these capabilities.
    pub fn shader_defs(&self) -> ShaderDefs {
        let mut defs = ShaderDefs::default();
        if !self.cube_array_textures {
            defs.define(NO_CUBE_ARRAY_TEXTURES_SUPPORT);
        }
        defs
    }
}
