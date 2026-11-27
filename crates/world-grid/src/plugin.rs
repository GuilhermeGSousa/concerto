use concerto_app::{edit_time::AtEditTime, schedule_groups::LateUpdate, Plugin};
use concerto_render::MaterialPlugin;

use crate::{material::WorldGridMaterial, world_grid::on_world_grid_changed};

/// Registers the [`WorldGridMaterial`] and keeps each grid's material in step with its
/// [`WorldGrid`](crate::WorldGrid), in the editor as well as in a game.
pub struct WorldGridPlugin;

impl Plugin for WorldGridPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_plugin(MaterialPlugin::<WorldGridMaterial>::new());
        app.add_system(LateUpdate, on_world_grid_changed.at_edit_time());
    }
}
