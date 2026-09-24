use concerto_app::Plugin;

pub struct RenderThreadPlugin;

impl Plugin for RenderThreadPlugin {
    fn build(&self, app: &mut concerto_app::App) {}
}
