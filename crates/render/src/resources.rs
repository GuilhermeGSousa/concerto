use concerto_ecs::resource::Resource;
use std::sync::Arc;

#[derive(Resource)]
pub struct RenderContext {
    pub(crate) surface: Option<Arc<wgpu::Surface<'static>>>,
    pub surface_config: wgpu::SurfaceConfiguration,
}
