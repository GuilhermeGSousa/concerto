use concerto_ecs::{events::event_channel::EventChannel, world::World};

use super::{asset_store::AssetStore, handle::AssetLoaded, Asset, AssetId};

pub(crate) trait AssetContainer: Send + Sync + 'static {
    fn insert(self: Box<Self>, id: AssetId, world: &mut World);
}

impl<A: Asset + 'static> AssetContainer for A {
    fn insert(self: Box<Self>, id: AssetId, world: &mut World) {
        world
            .get_resource_mut::<AssetStore<A>>()
            .expect("AssetStore not found")
            .insert(id, *self);
        if let Some(loaded) = world.get_resource_mut::<EventChannel<AssetLoaded<A>>>() {
            loaded.push_event(AssetLoaded::new(id));
        }
    }
}
