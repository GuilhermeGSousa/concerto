use concerto_ecs::resource::Resource;

use crate::draw_target::DrawTarget;

#[derive(Resource)]
pub struct RenderContext {
    pub(crate) target: DrawTarget,
    pub surface_config: wgpu::SurfaceConfiguration,
}
