use concerto_app::{schedule_groups::LateUpdate, Plugin};
use concerto_render::MaterialPlugin;

use crate::{material::WorldGridMaterial, world_grid::on_world_grid_changed};

pub struct WorldGridPlugin;

impl Plugin for WorldGridPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_plugin(MaterialPlugin::<WorldGridMaterial>::new());
        app.add_system(LateUpdate, on_world_grid_changed);
    }
}
