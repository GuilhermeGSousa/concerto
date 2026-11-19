use concerto_app::{App, edit_time::AtEditTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::systems::propagate_global_transforms;
use concerto_render::plugin::RenderPlugin;

use crate::{
    lighting::{
        AtmosphereLuts, build_atmosphere_luts, forget_sky_light, forget_sun_tint,
        update_sun_lighting,
    },
    render::SkyRenderPlugin,
    sun::{Sky, Sun},
};

/// Keeps the sun's tint and every sky's lighting up to date in the main world; needs no GPU.
pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Sun>()
            .register_type::<Sky>()
            .insert_resource(AtmosphereLuts::default())
            .track_removals::<Sun>()
            .track_removals::<Sky>()
            .add_system(
                LateUpdate,
                (build_atmosphere_luts, update_sun_lighting)
                    .chain()
                    .after(propagate_global_transforms)
                    .at_edit_time(),
            )
            .add_listener(forget_sky_light.at_edit_time())
            .add_listener(forget_sun_tint.at_edit_time());
    }
}

/// The sun and the sky, drawn when `RenderPlugin` is registered before it; part of `DefaultPlugins`.
pub struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(SkyPlugin);
        if app.is_plugin_added::<RenderPlugin>() {
            app.register_plugin(SkyRenderPlugin);
        }
    }

    fn finish(&self, app: &mut App) {
        if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<SkyRenderPlugin>() {
            log::warn!(
                "SkyboxPlugin was registered before RenderPlugin, so no sky is drawn: register it after RenderPlugin"
            );
        }
    }
}
