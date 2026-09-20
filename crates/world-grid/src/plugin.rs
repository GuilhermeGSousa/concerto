use app::{schedule_groups::LateUpdate, Plugin};
use render::MaterialPlugin;

use crate::{material::WorldGridMaterial, world_grid::on_world_grid_changed};

pub struct WorldGridPlugin;

impl Plugin for WorldGridPlugin {
    fn build(&self, app: &mut app::App) {
        app.register_plugin(MaterialPlugin::<WorldGridMaterial>::new());
        app.add_system(LateUpdate, on_world_grid_changed);
    }
}
