use concerto_ecs::world::World;

use super::{asset_store::AssetStore, Asset, AssetId};

pub(crate) trait AssetContainer: Send + Sync + 'static {
    fn insert(self: Box<Self>, id: AssetId, world: &mut World);
}

impl<A: Asset + 'static> AssetContainer for A {
    fn insert(self: Box<Self>, id: AssetId, world: &mut World) {
        world
            .get_resource_mut::<AssetStore<A>>()
            .expect("AssetStore not found")
            .insert(id, *self);
    }
}
