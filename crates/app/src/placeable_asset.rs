use std::{any::Any, collections::HashMap, marker::PhantomData};

use concerto_ecs::{
    events::event_reader::EventReader,
    resource::{Res, ResMut, Resource},
    signal::{On, Signal},
    CommandQueue, Entity, World,
};
use concerto_foundation::assets::{
    asset_server::AssetServer,
    asset_store::AssetStore,
    handle::{AssetHandle, AssetLoadFailed},
    AssetId, PlaceableAsset,
};

use crate::{edit_time::AtEditTime, schedule_groups::Update, App};

/// Asks the placement of `A` to fill `entity` from the loaded asset `asset`.
pub struct FillPlaced<A: PlaceableAsset> {
    pub asset: AssetId,
    pub entity: Entity,
    _asset: PhantomData<fn() -> A>,
}

impl<A: PlaceableAsset> FillPlaced<A> {
    /// The request to fill `entity` from `asset`.
    pub fn new(asset: AssetId, entity: Entity) -> Self {
        Self {
            asset,
            entity,
            _asset: PhantomData,
        }
    }
}

impl<A: PlaceableAsset> Signal for FillPlaced<A> {}

/// What the editor needs to place an asset kind it can't name.
#[derive(Clone, Copy)]
pub struct PlaceableKind {
    pub kind: &'static str,
    pub request_load: fn(&mut World, AssetId),
    pub is_loaded: fn(&World, AssetId) -> bool,
    pub fill: fn(&mut World, AssetId, Entity),
    pub release: fn(&mut World, AssetId),
}

impl PlaceableKind {
    /// The record of `A`.
    pub fn of<A: PlaceableAsset>() -> Self {
        Self {
            kind: A::name(),
            request_load: request_load::<A>,
            is_loaded: is_loaded::<A>,
            fill: fill::<A>,
            release,
        }
    }
}

/// Every asset kind marked with `register_placeable_asset`, in registration order.
#[derive(Resource, Default)]
pub struct PlaceableAssets(Vec<PlaceableKind>);

impl PlaceableAssets {
    /// The records, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &PlaceableKind> {
        self.0.iter()
    }

    /// The record of the kind named `kind`.
    pub fn get(&self, kind: &str) -> Option<&PlaceableKind> {
        self.0.iter().find(|known| known.kind == kind)
    }
}

#[derive(Resource, Default)]
struct PlacementLoads(HashMap<AssetId, Box<dyn Any + Send + Sync>>);

pub(crate) fn register<A: PlaceableAsset>(app: &mut App) {
    if app.get_resource::<PlaceableAssets>().is_none() {
        app.insert_resource(PlaceableAssets::default());
        app.insert_resource(PlacementLoads::default());
        app.add_system(Update, release_failed_loads.at_edit_time());
    }
    let placeable = app
        .get_resource_mut::<PlaceableAssets>()
        .expect("PlaceableAssets was just inserted");
    if placeable.get(A::name()).is_some() {
        return;
    }
    placeable.0.push(PlaceableKind::of::<A>());
    app.add_listener(fill_placed::<A>.at_edit_time());
}

fn release_failed_loads(
    mut failed: EventReader<AssetLoadFailed>,
    mut loads: ResMut<PlacementLoads>,
) {
    for event in failed.read() {
        loads.0.remove(&event.id);
    }
}

fn fill_placed<A: PlaceableAsset>(
    on: On<FillPlaced<A>>,
    assets: Res<AssetStore<A>>,
    server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let request = on.signal();
    let Some(asset) = assets.get(&AssetHandle::weak(request.asset)) else {
        log::warn!(
            "cannot place {} {:?}: it is not loaded",
            A::name(),
            request.asset
        );
        return;
    };
    A::place(
        asset,
        server.load::<A>(request.asset),
        request.entity,
        &mut cmd,
    );
}

fn request_load<A: PlaceableAsset>(world: &mut World, asset: AssetId) {
    let Some(server) = world.get_resource::<AssetServer>() else {
        return;
    };
    let handle = server.load::<A>(asset);
    if let Some(loads) = world.get_resource_mut::<PlacementLoads>() {
        loads.0.insert(asset, Box::new(handle));
    }
}

fn is_loaded<A: PlaceableAsset>(world: &World, asset: AssetId) -> bool {
    world
        .get_resource::<AssetStore<A>>()
        .is_some_and(|assets| assets.get(&AssetHandle::weak(asset)).is_some())
}

fn fill<A: PlaceableAsset>(world: &mut World, asset: AssetId, entity: Entity) {
    world.trigger(FillPlaced::<A>::new(asset, entity));
    release(world, asset);
}

fn release(world: &mut World, asset: AssetId) {
    if let Some(loads) = world.get_resource_mut::<PlacementLoads>() {
        loads.0.remove(&asset);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        edit_time::AtEditTime,
        main_schedule::MainSchedulePlugin,
        plugins::{AssetManagerPlugin, TimePlugin},
        schedule_groups::Update,
        App,
    };
    use concerto_ecs::{
        events::event_reader::EventReader,
        resource::{ResMut, Resource},
        CommandQueue, Component, Entity,
    };
    use concerto_foundation::assets::{
        asset_server::AssetServer,
        content::save_content_asset_into,
        handle::{AssetHandle, AssetLoadFailed},
        registry::AssetRegistry,
        Asset, AssetId, LoadableAsset, PlaceableAsset,
    };

    use super::PlaceableAssets;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Prop {
        size: u32,
    }
    impl Asset for Prop {
        type EditorOnly = ();

        fn name() -> &'static str {
            "TestProp"
        }
    }
    impl LoadableAsset for Prop {}
    impl PlaceableAsset for Prop {
        fn place(asset: &Self, handle: AssetHandle<Self>, entity: Entity, cmd: &mut CommandQueue) {
            cmd.insert(
                Placed {
                    size: asset.size,
                    asset: handle.id(),
                },
                entity,
            );
        }
    }

    #[derive(Component, PartialEq, Debug)]
    struct Placed {
        size: u32,
        asset: AssetId,
    }

    #[derive(Resource, Default)]
    struct Failed(Vec<AssetId>);

    fn collect_failed(mut events: EventReader<AssetLoadFailed>, mut failed: ResMut<Failed>) {
        failed.0.extend(events.read().map(|event| event.id));
    }

    fn run_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(app) {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} never happened"
            );
            app.update();
        }
    }

    #[test]
    fn a_placeable_kind_loads_its_asset_fills_an_entity_and_lets_the_asset_go() {
        let root =
            std::env::temp_dir().join(format!("placeable-asset-{}", AssetId::new().simple_hex()));
        let mut registry = AssetRegistry::new();
        let first = save_content_asset_into(
            &Prop { size: 3 },
            None,
            &root,
            "content/props/first.gasset",
            &mut registry,
        )
        .unwrap();
        let without_second = registry.clone();
        let second = save_content_asset_into(
            &Prop { size: 4 },
            None,
            &root,
            "content/props/second.gasset",
            &mut registry,
        )
        .unwrap();

        let mut app = App::for_editing();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_placeable_asset::<Prop>()
            .register_placeable_asset::<Prop>()
            .insert_resource(Failed::default())
            .add_system(Update, collect_failed.at_edit_time());
        app.get_resource::<AssetServer>()
            .unwrap()
            .publish_project_content(&root, without_second)
            .unwrap();
        app.finish_plugin_build();

        let placeable = app.get_resource::<PlaceableAssets>().unwrap();
        assert_eq!(
            placeable.iter().map(|kind| kind.kind).collect::<Vec<_>>(),
            vec!["TestProp"]
        );
        assert!(placeable.get("Texture").is_none());
        let kind = *placeable.get("TestProp").unwrap();
        let loaded = move |id: AssetId| move |app: &App| (kind.is_loaded)(app.main().world(), id);

        (kind.request_load)(app.main_mut().world_mut(), first);
        assert!(!loaded(first)(&app));
        run_until(&mut app, "the load of the first prop", loaded(first));

        let entity = app.main_mut().world_mut().spawn(());
        (kind.fill)(app.main_mut().world_mut(), first, entity);
        assert_eq!(
            app.main()
                .world()
                .get_component_for_entity::<Placed>(entity),
            Some(&Placed {
                size: 3,
                asset: first
            })
        );
        run_until(&mut app, "the unload of the placed prop", |app| {
            !loaded(first)(app)
        });

        (kind.request_load)(app.main_mut().world_mut(), first);
        run_until(&mut app, "the second load of the first prop", loaded(first));
        (kind.release)(app.main_mut().world_mut(), first);
        (kind.release)(app.main_mut().world_mut(), first);
        run_until(&mut app, "the unload of the released prop", |app| {
            !loaded(first)(app)
        });

        (kind.request_load)(app.main_mut().world_mut(), second);
        run_until(&mut app, "the failure of the second prop", |app| {
            app.get_resource::<Failed>().unwrap().0 == vec![second]
        });
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        server.publish_registry(registry);
        let handle = server.load::<Prop>(second);
        run_until(&mut app, "the load of the second prop", loaded(second));
        drop(handle);
        run_until(&mut app, "the unload of the second prop", |app| {
            !loaded(second)(app)
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
