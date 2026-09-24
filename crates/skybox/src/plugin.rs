use concerto_app::plugins::Plugin;
use concerto_core::assets::asset_server::AssetServer;
use concerto_render::{MaterialPlugin, assets::mesh::Mesh};

use crate::{SKYBOX_INDICES, SKYBOX_VERTICES, SkyboxCube, material::SkyboxMaterial};

pub struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        // Setup skybox
        let skybox_cube = Mesh {
            vertices: SKYBOX_VERTICES.to_vec(),
            indices: SKYBOX_INDICES.to_vec(),
        };

        let skybox_cube = SkyboxCube(
            app.get_resource_mut::<AssetServer>()
                .expect("Could not find Mesh asset store")
                .add(skybox_cube),
        );
        app.register_plugin(MaterialPlugin::<SkyboxMaterial>::new())
            .insert_resource(skybox_cube);
    }
}
