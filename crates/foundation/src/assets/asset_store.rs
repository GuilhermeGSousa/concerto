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
        for event in self.drop_receiver.try_iter() {
            if self.assets.contains_key(&event.id()) {
                events.write(event.clone());
                match event {
                    AssetLifetimeEvent::Dropped(id) => {
                        self.assets.remove(&id);
                        asset_server.process_handle_drop(&id);
                    }
                }
            }
        }
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
