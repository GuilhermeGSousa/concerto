use concerto_ecs::resource::Resource;

use crate::draw_target::DrawTarget;

#[derive(Resource)]
pub struct RenderContext {
    pub(crate) target: DrawTarget,
    pub surface_config: wgpu::SurfaceConfiguration,
    /// The format of every camera's HDR colour texture, which scene pipelines draw into.
    pub view_target_format: wgpu::TextureFormat,
    /// Whether a view target's multisampled depth is created so that a pass can read it.
    pub multisampled_depth_sampled: bool,
}
