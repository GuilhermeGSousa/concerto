use concerto::{asset_id, foundation::assets::AssetId};
use concerto::{
    ecs::{CommandQueue, Res},
    foundation::{assets::asset_server::AssetServer, transform::Transform},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
};

const FOREST_SCENE: AssetId = asset_id!("content/forest/scene.gasset");

pub(crate) fn spawn_scene(mut cmd: CommandQueue, asset_server: Res<AssetServer>) {
    // TODO(asset-import-pipeline): the forest lost its per-scene shadow opt-in
    // (old GLTFSpawnerComponent::with_shadows) — the importer hard-codes
    // Light::shadowmaps_enabled to false.
    cmd.spawn((
        SceneSpawnerComponent(asset_server.load::<Scene>(FOREST_SCENE)),
        Transform::default(),
    ));
}
