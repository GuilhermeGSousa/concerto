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

/// One loaded asset in an [`AssetStore`].
pub struct AssetStoreEntry<A: Asset> {
    pub(crate) asset: A,
}

/// The resource that holds every loaded asset of type `A`, by id.
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
    /// Creates an empty store.
    pub fn new() -> Self {
        let (lifetime_sender, lifetime_recv) = crossbeam_channel::unbounded();
        AssetStore {
            assets: HashMap::new(),
            drop_sender: lifetime_sender,
            drop_receiver: lifetime_recv,
            _marker: std::marker::PhantomData,
        }
    }

    /// Returns the asset `handle` refers to, or `None` while it is not loaded.
    pub fn get(&self, handle: &AssetHandle<A>) -> Option<&A> {
        self.assets.get(&handle.id()).map(|entry| &entry.asset)
    }

    /// Returns the asset `handle` refers to mutably, or `None` while it is not loaded.
    pub fn get_mut(&mut self, handle: &AssetHandle<A>) -> Option<&mut A> {
        self.assets
            .get_mut(&handle.id())
            .map(|entry| &mut entry.asset)
    }

    /// Stores `asset` under `id`, replacing any asset already there.
    pub fn insert(&mut self, id: AssetId, asset: A)
    where
        A: 'static,
    {
        let entry = AssetStoreEntry { asset };
        self.assets.insert(id, entry);
    }

    /// Removes the assets whose last handle dropped and reports each as an event.
    ///
    /// Returns whether any was removed.
    pub fn track_assets(
        &mut self,
        mut asset_server: ResMut<AssetServer>,
        mut events: EventWriter<AssetLifetimeEvent>,
    ) -> bool {
        let dropped = self.drain_drops(&mut asset_server);
        let removed = !dropped.is_empty();
        for event in dropped {
            events.write(event);
        }
        removed
    }

    pub(crate) fn drain_drops(&mut self, server: &mut AssetServer) -> Vec<AssetLifetimeEvent> {
        let mut processed = Vec::new();
        for event in self.drop_receiver.try_iter() {
            if self.assets.contains_key(&event.id()) {
                match event {
                    AssetLifetimeEvent::Dropped(id) => {
                        if !server.process_handle_drop(&id) {
                            continue;
                        }
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

    struct Pebbles {
        server: AssetServer,
        world: World,
    }

    fn pebble_world() -> Pebbles {
        let mut server = AssetServer::new();
        let store = AssetStore::<Pebble>::new();
        server.register_asset(&store);
        let mut world = World::new();
        world.insert_resource(store);
        world.insert_resource(server.clone());
        Pebbles { server, world }
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

    enum Step {
        DropFirst,
        LoadAgain,
        DropAgain,
        Drain { dropped: bool },
    }

    struct Lifetime {
        name: &'static str,
        steps: &'static [Step],
        loaded: bool,
    }

    #[test]
    fn an_asset_unloads_once_its_last_handle_drops_and_a_drain_sees_it() {
        use Step::*;
        let cases = [
            Lifetime {
                name: "a dropped asset with no returned handle",
                steps: &[DropFirst, Drain { dropped: true }],
                loaded: false,
            },
            Lifetime {
                name: "a handle requested before the drop is processed",
                steps: &[DropFirst, LoadAgain, Drain { dropped: false }],
                loaded: true,
            },
            Lifetime {
                name: "the returned handle dropped before the next drain",
                steps: &[
                    DropFirst,
                    LoadAgain,
                    Drain { dropped: false },
                    DropAgain,
                    Drain { dropped: true },
                ],
                loaded: false,
            },
        ];
        for case in cases {
            let Pebbles { server, mut world } = pebble_world();
            let mut first = Some(loaded_pebble(&server, &mut world));
            let id = first.as_ref().unwrap().id();
            let mut again = None;
            for step in case.steps {
                match step {
                    DropFirst => drop(first.take()),
                    LoadAgain => again = Some(server.load::<Pebble>(id)),
                    DropAgain => drop(again.take()),
                    Drain { dropped } => {
                        let events = drain(&server, &mut world);
                        assert_eq!(
                            events.len(),
                            usize::from(*dropped),
                            "{}: {} events",
                            case.name,
                            events.len()
                        );
                        assert!(
                            events.iter().all(|event| matches!(event, AssetLifetimeEvent::Dropped(dropped) if *dropped == id)),
                            "{}",
                            case.name
                        );
                    }
                }
            }
            assert_eq!(holds(&world, id), case.loaded, "{}", case.name);
            assert_eq!(
                server.is_loading_or_loaded(id),
                case.loaded,
                "{}",
                case.name
            );
            assert!(!server.is_loading(), "{}", case.name);
            drop(again);
        }
    }
}
