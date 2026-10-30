use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;

use concerto_ecs::{
    events::event_writer::EventWriter,
    resource::{ResMut, Resource},
};

use super::{
    asset_server::AssetServer,
    handle::{AssetHandle, AssetLifetimeEvent},
    Asset, AssetId,
};

pub struct AssetStoreEntry<A: Asset> {
    pub(crate) asset: A,
}

pub struct AssetStore<A: Asset + 'static> {
    assets: HashMap<AssetId, AssetStoreEntry<A>>,
    drop_sender: Sender<AssetLifetimeEvent>,
    drop_receiver: Receiver<AssetLifetimeEvent>,
    _marker: std::marker::PhantomData<A>,
}

impl<A: Asset + 'static> Resource for AssetStore<A> {
    fn name() -> &'static str {
        A::name()
    }
}

impl<A: Asset + 'static> AssetStore<A> {
    pub fn new() -> Self {
        let (lifetime_sender, lifetime_recv) = crossbeam_channel::unbounded();
        AssetStore {
            assets: HashMap::new(),
            drop_sender: lifetime_sender,
            drop_receiver: lifetime_recv,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn get(&self, handle: &AssetHandle<A>) -> Option<&A> {
        self.assets.get(&handle.id()).map(|entry| &entry.asset)
    }

    pub fn get_mut(&mut self, handle: &AssetHandle<A>) -> Option<&mut A> {
        self.assets
            .get_mut(&handle.id())
            .map(|entry| &mut entry.asset)
    }

    pub fn insert(&mut self, id: AssetId, asset: A)
    where
        A: 'static,
    {
        let entry = AssetStoreEntry { asset };
        self.assets.insert(id, entry);
    }

    pub fn track_assets(
        &mut self,
        mut asset_server: ResMut<AssetServer>,
        mut events: EventWriter<AssetLifetimeEvent>,
    ) {
        for event in self.drain_drops(&mut asset_server) {
            events.write(event);
        }
    }

    pub(crate) fn drain_drops(&mut self, server: &mut AssetServer) -> Vec<AssetLifetimeEvent> {
        let mut processed = Vec::new();
        for event in self.drop_receiver.try_iter() {
            if self.assets.contains_key(&event.id()) {
                match event {
                    AssetLifetimeEvent::Dropped(id) => {
                        server.process_handle_drop(&id);
                        self.assets.remove(&id);
                        processed.push(event);
                    }
                }
            }
        }
        processed
    }

    pub(crate) fn clone_drop_sender(&self) -> Sender<AssetLifetimeEvent> {
        self.drop_sender.clone()
    }
}

impl<'a, A: Asset + 'static> IntoIterator for &'a AssetStore<A> {
    type Item = (&'a AssetId, &'a A);

    type IntoIter = std::iter::Map<
        std::collections::hash_map::Iter<'a, AssetId, AssetStoreEntry<A>>,
        fn((&'a AssetId, &'a AssetStoreEntry<A>)) -> (&'a AssetId, &'a A),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.assets.iter().map(|(id, entry)| (id, &entry.asset))
    }
}

impl<A: Asset + 'static> Default for AssetStore<A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{asset_server::handle_asset_load_events, LoadableAsset};
    use concerto_ecs::world::World;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Pebble;

    impl Asset for Pebble {
        fn name() -> &'static str {
            "Pebble"
        }
    }

    impl LoadableAsset for Pebble {}

    fn pebble_world() -> (AssetServer, World) {
        let mut server = AssetServer::new();
        let store = AssetStore::<Pebble>::new();
        server.register_asset(&store);
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        (server, world)
    }

    fn loaded_pebble(server: &AssetServer, world: &mut World) -> AssetHandle<Pebble> {
        let handle = server.add(Pebble);
        handle_asset_load_events(world);
        handle
    }

    fn drain(server: &AssetServer, world: &mut World) -> Vec<AssetLifetimeEvent> {
        let mut store = world.remove_resource::<AssetStore<Pebble>>().unwrap();
        let events = store.drain_drops(&mut server.clone());
        world.insert_resource(store);
        events
    }

    fn holds(world: &World, id: AssetId) -> bool {
        world
            .get_resource::<AssetStore<Pebble>>()
            .unwrap()
            .assets
            .contains_key(&id)
    }

    #[test]
    fn a_handle_requested_before_the_drop_is_processed_keeps_the_asset_loaded() {
        let (server, mut world) = pebble_world();
        let handle = loaded_pebble(&server, &mut world);
        let id = handle.id();
        drop(handle);

        let returned = server.load::<Pebble>(id);
        let events = drain(&server, &mut world);

        assert!(events.is_empty());
        assert!(holds(&world, id));
        assert!(server.is_loaded(&id));
        assert!(!server.is_loading());
        drop(returned);
    }

    #[test]
    fn a_dropped_asset_with_no_returned_handle_unloads() {
        let (server, mut world) = pebble_world();
        let handle = loaded_pebble(&server, &mut world);
        let id = handle.id();
        drop(handle);

        let events = drain(&server, &mut world);

        assert!(
            matches!(events.as_slice(), [AssetLifetimeEvent::Dropped(dropped)] if *dropped == id)
        );
        assert!(!holds(&world, id));
        assert!(!server.is_loaded(&id));
    }

    #[test]
    fn dropping_the_returned_handle_unloads_the_asset_on_the_next_drain() {
        let (server, mut world) = pebble_world();
        let handle = loaded_pebble(&server, &mut world);
        let id = handle.id();
        drop(handle);
        let returned = server.load::<Pebble>(id);
        assert!(drain(&server, &mut world).is_empty());

        drop(returned);
        let events = drain(&server, &mut world);

        assert!(
            matches!(events.as_slice(), [AssetLifetimeEvent::Dropped(dropped)] if *dropped == id)
        );
        assert!(!holds(&world, id));
        assert!(!server.is_loaded(&id));
    }

    #[test]
    fn a_duplicate_drop_event_for_an_unloaded_id_is_ignored() {
        let (server, mut world) = pebble_world();
        let handle = loaded_pebble(&server, &mut world);
        let id = handle.id();
        let sender = world
            .get_resource::<AssetStore<Pebble>>()
            .unwrap()
            .clone_drop_sender();
        drop(handle);
        sender.send(AssetLifetimeEvent::Dropped(id)).unwrap();

        let events = drain(&server, &mut world);

        assert_eq!(events.len(), 1);
        assert!(!holds(&world, id));
        assert!(!server.is_loaded(&id));
    }
}
