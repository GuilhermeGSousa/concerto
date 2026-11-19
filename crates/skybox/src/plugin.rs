use concerto_app::{App, edit_time::AtEditTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::systems::propagate_global_transforms;

use crate::{
    lighting::{
        AtmosphereLuts, build_atmosphere_luts, forget_sky_light, forget_sun_tint,
        update_sun_lighting,
    },
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

/// The sun and the sky; part of `DefaultPlugins`.
pub struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        app.register_plugin(SkyPlugin);
    }
}
