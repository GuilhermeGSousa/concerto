//! The application shell of Concerto: plugins, schedules and the frame loop.
//!
//! # Examples
//!
//! ```
//! use concerto_app::{
//!     App, main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
//! };
//!
//! fn greet() {
//!     println!("hello");
//! }
//!
//! let mut app = App::new();
//! app.register_plugin(MainSchedulePlugin)
//!     .register_plugin(TimePlugin)
//!     .add_system(Update, greet);
//! app.finish_plugin_build();
//! app.update();
//! ```

#[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
use concerto_ecs::system::executor::multi_thread::MultiThreadedExecutor;
#[cfg(not(all(feature = "multithreaded", not(target_arch = "wasm32"))))]
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::{
    component::removed::ComponentRemoved,
    events::{
        event_channel::{update_event_channel, EventChannel},
        event_writer::EventWriter,
        Event,
    },
    resource::{ResMut, Resource},
    signal::Signal,
    system::schedule::{CompiledSchedules, ScheduleLabel, Schedules},
    Component, IntoSetConfig, IntoSystemConfig, SystemTag, TagFilter, World,
};
use log::info;

use concerto_foundation::assets::{
    asset_server::AssetServer,
    asset_store::AssetStore,
    derived::DerivedData,
    handle::{AssetLifetimeEvent, AssetLoaded},
    Asset, AssetId, CreatableAsset, LoadableAsset, PlaceableAsset,
};
use concerto_reflect::{GetTypeRegistration, Reflect};

use crate::{
    edit_time::{AppListener, AtEditTime, EditTime, Preview},
    plugins::PluginsState,
    schedule_groups::{First, Update},
    subapp::{SubApp, SubApps},
};

/// Marking systems and listeners to run in edit and preview `App`s as well as in a game.
pub mod edit_time;
/// Extraction: copying data from the main world into the render world each frame.
pub mod extractor;
/// [`InspectedAssets`], the asset kinds the editor can create, show and save without naming their
/// types.
pub mod inspected_asset;
/// [`MainSchedulePlugin`](main_schedule::MainSchedulePlugin), which runs the main world's frame.
pub mod main_schedule;
/// [`PlaceableAssets`], the asset kinds the editor can place in a scene without naming their types.
pub mod placeable_asset;
/// [`Plugin`] and the engine's base plugins, such as time and asset management.
pub mod plugins;
/// The labels of the schedules each frame runs, in the main and render worlds.
pub mod schedule_groups;
/// [`SubApp`], one world with its own schedules.
pub mod subapp;

pub use inspected_asset::{DecodedAsset, InspectedAsset, InspectedAssets};
pub use placeable_asset::{FillPlaced, PlaceableAssets, PlaceableKind};
pub use plugins::Plugin;

/// The scene a game starts in, declared with [`App::set_startup_scene`].
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupScene(pub AssetId);

pub(crate) struct HokeyPokeyPlugin;
impl Plugin for HokeyPokeyPlugin {
    fn build(&self, _: &mut App) {}
}

fn compile(schedules: Schedules, world: &mut World, filter: &TagFilter) -> CompiledSchedules {
    #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
    {
        schedules.compile_filtered::<MultiThreadedExecutor>(world, filter)
    }
    #[cfg(not(all(feature = "multithreaded", not(target_arch = "wasm32"))))]
    {
        schedules.compile_filtered::<SingleThreadedExecutor>(world, filter)
    }
}

/// What an [`App`] is for, which decides the systems and listeners it keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppMode {
    /// A game: every system and listener runs.
    Game,
    /// The editor's view of a scene: only what is marked
    /// [`at_edit_time`](edit_time::AtEditTime::at_edit_time) runs.
    Edit,
    /// A preview in the editor: only what is marked to run at edit or preview time runs.
    Preview,
}

/// The top-level container: the main and render [`SubApp`]s and the registered [`Plugin`]s.
pub struct App {
    subapps: SubApps,
    plugins: Vec<Box<dyn Plugin>>,
    plugin_types: std::collections::HashSet<std::any::TypeId>,
    plugin_state: PluginsState,
    mode: AppMode,
}

impl App {
    /// Creates a game `App` with empty main and render worlds and no plugins.
    pub fn new() -> App {
        Self {
            subapps: SubApps::default(),
            plugins: Vec::new(),
            plugin_types: Default::default(),
            plugin_state: PluginsState::Building,
            mode: AppMode::Game,
        }
    }

    /// Creates an edit `App`: only main-world systems and listeners marked `at_edit_time` run in it.
    pub fn for_editing() -> App {
        App {
            mode: AppMode::Edit,
            ..App::new()
        }
    }

    /// Creates a preview `App`: only main-world systems and listeners marked `at_edit_time` or `at_preview_time` run in it.
    pub fn for_previewing() -> App {
        App {
            mode: AppMode::Preview,
            ..App::new()
        }
    }

    /// Declares the scene a game starts in; `ScenePlugin` spawns it in a game `App`, never in an edit `App`.
    pub fn set_startup_scene(&mut self, id: AssetId) -> &mut Self {
        self.insert_resource(StartupScene(id))
    }

    /// Whether this `App` was created by [`App::for_editing`] or [`App::for_previewing`].
    pub fn is_editing(&self) -> bool {
        self.mode != AppMode::Game
    }

    /// Whether this `App` was created by [`App::for_previewing`].
    pub fn is_previewing(&self) -> bool {
        self.mode == AppMode::Preview
    }

    /// Builds `plugin` immediately and keeps it for [`Plugin::ready`] and [`Plugin::finish`].
    pub fn register_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        info!("Registering plugin: {}", plugin.name());
        self.plugin_types.insert(std::any::TypeId::of::<P>());
        plugin.build(self);
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Whether a plugin of type `P` has been registered on this app.
    pub fn is_plugin_added<P: Plugin>(&self) -> bool {
        self.plugin_types.contains(&std::any::TypeId::of::<P>())
    }

    /// Registers an asset type and the system that tracks its handles, once per type.
    ///
    /// # Panics
    ///
    /// Panics without [`AssetManagerPlugin`](plugins::AssetManagerPlugin), or once the schedules
    /// have been compiled.
    pub fn register_asset<A: Asset>(&mut self) -> &mut Self {
        self.register_asset_with::<A>(AssetServer::register_asset)
    }

    /// Registers a derived asset type as [`register_asset`](Self::register_asset) does, deriving its blob on every load.
    ///
    /// # Panics
    ///
    /// Panics without [`AssetManagerPlugin`](plugins::AssetManagerPlugin), or once the schedules
    /// have been compiled.
    pub fn register_derived_asset<A: DerivedData>(&mut self) -> &mut Self {
        self.register_asset_with::<A>(AssetServer::register_derived_asset)
    }

    /// Registers `A` as an asset and a reflected type, and records it in [`InspectedAssets`] so the editor can create, inspect and save it.
    ///
    /// # Panics
    ///
    /// Panics without [`AssetManagerPlugin`](plugins::AssetManagerPlugin), or once the schedules
    /// have been compiled.
    pub fn register_inspected_asset<
        A: CreatableAsset + LoadableAsset + Reflect + GetTypeRegistration,
    >(
        &mut self,
    ) -> &mut Self {
        self.register_asset::<A>();
        self.register_type::<A>();
        if self.get_resource::<InspectedAssets>().is_none() {
            self.insert_resource(InspectedAssets::default());
        }
        self.get_resource_mut::<InspectedAssets>()
            .expect("InspectedAssets was just inserted")
            .insert(InspectedAsset::of::<A>());
        self
    }

    /// Registers `A` as an asset and records it in [`PlaceableAssets`] so the editor can place it in a scene.
    ///
    /// # Panics
    ///
    /// Panics without [`AssetManagerPlugin`](plugins::AssetManagerPlugin), or once the schedules
    /// have been compiled.
    pub fn register_placeable_asset<A: PlaceableAsset>(&mut self) -> &mut Self {
        self.register_asset::<A>();
        placeable_asset::register::<A>(self);
        self
    }

    fn register_asset_with<A: Asset>(
        &mut self,
        register: fn(&mut AssetServer, &AssetStore<A>),
    ) -> &mut Self {
        if self
            .get_resource::<AssetServer>()
            .expect("Asset Server not found")
            .is_registered::<A>()
        {
            return self;
        }
        let asset_store = AssetStore::<A>::new();
        let asset_server = self
            .get_resource_mut::<AssetServer>()
            .expect("Asset Server not found");

        register(asset_server, &asset_store);

        self.add_system(
            Update,
            (|mut asset_store: ResMut<AssetStore<A>>,
              asset_server: ResMut<AssetServer>,
              events: EventWriter<AssetLifetimeEvent>| {
                if asset_store
                    .bypass_change_detection()
                    .track_assets(asset_server, events)
                {
                    asset_store.mark_changed();
                }
            })
            .at_edit_time(),
        );

        self.main_mut().insert_resource(asset_store);
        self.register_event::<AssetLoaded<A>>();
        self
    }

    /// Adds systems to the main world's `update_group` schedule.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn add_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M>,
    ) -> &mut Self {
        self.get_resource_mut::<Schedules>()
            .expect("Schedules resource not found!")
            .add_system(update_group, system);

        self
    }

    /// Adds systems to the render world's `update_group` schedule.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn add_render_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M>,
    ) -> &mut Self {
        self.subapps
            .render_mut()
            .get_resource_mut::<Schedules>()
            .expect("Schedules resource not found on render subapp!")
            .add_system(update_group, system);

        self
    }

    /// Adds ordering constraints between sets in the main world's `update_group` schedule.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfig,
    ) -> &mut Self {
        self.main_mut().configure_sets(update_group, configs);
        self
    }

    /// Adds ordering constraints between sets in the render world's `update_group` schedule.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn configure_render_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfig,
    ) -> &mut Self {
        self.subapps
            .render_mut()
            .configure_sets(update_group, configs);
        self
    }

    /// Registers an event type.
    ///
    /// Without this its `EventChannel` resource never exists, so systems taking `EventWriter`
    /// or `EventReader` for it are skipped.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn register_event<T: Event + 'static>(&mut self) -> &mut Self {
        let event_channel = EventChannel::<T>::new();

        self.insert_resource(event_channel);
        self.add_system(First, update_event_channel::<T>.at_edit_time());
        self
    }

    /// Records every removal of `T` in the main world, for systems taking `RemovedComponents<T>`.
    ///
    /// # Panics
    ///
    /// Panics once the schedules have been compiled.
    pub fn track_removals<T: Component>(&mut self) -> &mut Self {
        if self
            .get_resource::<EventChannel<ComponentRemoved<T>>>()
            .is_some()
        {
            return self;
        }
        self.add_system(
            First,
            update_event_channel::<ComponentRemoved<T>>.at_edit_time(),
        );
        self.main_mut().world_mut().track_removals::<T>();
        self
    }

    /// Inserts `value` into the main world, replacing any resource of the same type.
    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.main_mut().insert_resource(value);
        self
    }

    /// Runs `listener` every time a `T` is triggered in the main world.
    ///
    /// An edit or preview `App` keeps only listeners marked for it, with
    /// [`AtEditTime`] or [`AtPreviewTime`](edit_time::AtPreviewTime).
    pub fn add_listener<T: Signal, M>(&mut self, listener: impl AppListener<T, M>) -> &mut Self {
        let mode = self.mode;
        listener.add_to(self.main_mut().world_mut(), mode);
        self
    }

    /// Registers `T` in the main world's type registry with the type data its derive declared.
    pub fn register_type<T: concerto_reflect::GetTypeRegistration>(&mut self) -> &mut Self {
        self.main_mut().register_type::<T>();
        self
    }

    /// Makes `P` a member of every `InputSet<K>` in the main world.
    ///
    /// Register it before the schedules compile: an input set takes its members when its
    /// system is initialized.
    pub fn register_input<
        K: 'static,
        P: concerto_ecs::system::input::ReadOnlySystemInput + 'static,
    >(
        &mut self,
    ) -> &mut Self {
        concerto_ecs::system::input::register_input::<K, P>(self.main_mut().world_mut());
        self
    }

    /// Makes `T` require `U`, built with `U::default()`, in the main world.
    ///
    /// # Panics
    ///
    /// Panics if an entity holds `T` or a component that requires it, or if the requirement
    /// forms a cycle.
    pub fn register_required_components<T: Component, U: Component + Default>(
        &mut self,
    ) -> &mut Self {
        self.main_mut().register_required_components::<T, U>();
        self
    }

    /// Makes `T` require `U`, built by `constructor`, in the main world.
    ///
    /// # Panics
    ///
    /// Panics if an entity holds `T` or a component that requires it, or if the requirement
    /// forms a cycle.
    pub fn register_required_components_with<T: Component, U: Component>(
        &mut self,
        constructor: fn() -> U,
    ) -> &mut Self {
        self.main_mut()
            .register_required_components_with::<T, U>(constructor);
        self
    }

    /// Removes the resource `R` from the main world and returns it, or `None` when it is absent.
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.main_mut().remove_resource()
    }

    /// Returns the main world's resource `R`, or `None` when it is absent.
    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.main().get_resource()
    }

    /// Returns the main world's resource `R` mutably and marks it changed, or `None` when it is
    /// absent.
    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.main_mut().get_resource_mut()
    }

    /// Replaces resource `R` with the one `f` builds from it, if `R` is present.
    pub fn with_resource<R: Resource, F, T: Resource>(&mut self, f: F)
    where
        F: FnOnce(R) -> T,
    {
        let Some(resource) = self.remove_resource::<R>() else {
            return;
        };
        let output = f(resource);
        self.insert_resource(output);
    }

    /// Runs one frame: main world, extraction, then render world.
    pub fn update(&mut self) {
        profiling::scope!("App::update");

        self.subapps.update();
    }

    /// Runs one main-world schedule between frames, for a host.
    pub fn serve(&mut self, label: impl ScheduleLabel) {
        self.main_mut().world_mut().run_schedule(label);
    }

    /// Returns the main world's sub-app.
    pub fn main(&self) -> &SubApp {
        self.subapps.main()
    }

    /// Returns the main world's sub-app mutably.
    pub fn main_mut(&mut self) -> &mut SubApp {
        self.subapps.main_mut()
    }

    /// Returns the render world's sub-app.
    pub fn render(&self) -> &SubApp {
        self.subapps.render()
    }

    /// Returns the render world's sub-app mutably.
    pub fn render_mut(&mut self) -> &mut SubApp {
        self.subapps.render_mut()
    }

    /// Polls every plugin's [`ready`](Plugin::ready) and returns the resulting state.
    pub fn plugin_state(&mut self) -> PluginsState {
        let next_state = match self.plugin_state {
            PluginsState::Building => {
                if self.plugins.iter().all(|plugin| plugin.ready(self)) {
                    PluginsState::Ready
                } else {
                    PluginsState::Building
                }
            }
            state => state,
        };

        self.plugin_state = next_state;

        next_state
    }

    /// Finishes every plugin, compiles the schedules and runs [`Startup`](schedule_groups::Startup).
    ///
    /// # Panics
    ///
    /// Panics if called twice, or if any schedule's explicit constraints contain a cycle.
    pub fn finish_plugin_build(&mut self) {
        let mut hokeypokey: Box<dyn Plugin> = Box::new(HokeyPokeyPlugin);
        let mut i = 0;
        while i < self.plugins.len() {
            core::mem::swap(&mut self.plugins[i], &mut hokeypokey);
            {
                profiling::scope!("Plugin::finish", hokeypokey.name());
                hokeypokey.finish(self);
            }
            core::mem::swap(&mut self.plugins[i], &mut hokeypokey);
            i += 1;
        }

        self.plugin_state = PluginsState::Finished;

        {
            profiling::scope!("App::compile_schedules");
            self.compile_schedules();
        }
        {
            profiling::scope!("App::compile_render_schedules");
            self.compile_render_schedules();
        }

        profiling::scope!("Startup");
        self.subapps.startup();
    }

    fn compile_schedules(&mut self) {
        let schedules = self
            .remove_resource::<Schedules>()
            .expect("Schedules resource not found!");

        let filter = match self.mode {
            AppMode::Game => TagFilter::default(),
            AppMode::Edit => TagFilter {
                include: vec![EditTime.intern()],
                exclude: Vec::new(),
            },
            AppMode::Preview => TagFilter {
                include: vec![EditTime.intern(), Preview.intern()],
                exclude: Vec::new(),
            },
        };
        let world = self.main_mut().world_mut();
        let compiled_schedules = compile(schedules, world, &filter);
        self.insert_resource(compiled_schedules);
    }

    fn compile_render_schedules(&mut self) {
        let schedules = self
            .subapps
            .render_mut()
            .remove_resource::<Schedules>()
            .expect("Schedules resource not found on render subapp!");

        let world = self.render_mut().world_mut();
        let compiled_schedules = compile(schedules, world, &TagFilter::default());
        self.subapps
            .render_mut()
            .insert_resource(compiled_schedules);
    }

    /// Sets the function that copies main-world data into the render world each frame.
    pub fn set_extract_fn(
        &mut self,
        extract_fn: impl FnMut(&mut World, &mut World) + Send + Sync + 'static,
    ) {
        self.subapps.set_extract_fn(extract_fn);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{
        events::{event_channel::EventChannel, event_reader::EventReader},
        query::change_detection::DetectChanges,
        resource::Res,
    };

    #[derive(Event)]
    struct ExternalEvent;

    #[derive(Resource, Default)]
    struct Seen(usize);

    fn observe(mut events: EventReader<ExternalEvent>, mut seen: ResMut<Seen>) {
        seen.0 += events.read().count();
    }

    use crate::edit_time::{AtEditTime, AtPreviewTime};
    use concerto_ecs::{
        signal::{On, Signal},
        system::schedule::ScheduleLabel,
    };
    use concerto_foundation::assets::handle::AssetHandle;

    #[derive(Resource, Default)]
    struct Count(u32);

    fn count(mut count: ResMut<Count>) {
        count.0 += 1;
    }

    struct Ping;
    impl Signal for Ping {}

    fn hear(_: On<Ping>, mut count: ResMut<Count>) {
        count.0 += 1;
    }

    #[derive(Clone, Copy)]
    enum Mark {
        None,
        Edit,
        Preview,
    }

    fn counted_after_one_frame(mut app: App, mark: Mark) -> u32 {
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .insert_resource(Count::default());
        match mark {
            Mark::None => app.add_system(Update, count),
            Mark::Edit => app.add_system(Update, count.at_edit_time()),
            Mark::Preview => app.add_system(Update, count.at_preview_time()),
        };
        app.finish_plugin_build();
        app.update();
        app.get_resource::<Count>().unwrap().0
    }

    fn heard_after_one_ping(mut app: App, mark: Mark) -> u32 {
        app.insert_resource(Count::default());
        match mark {
            Mark::None => app.add_listener(hear),
            Mark::Edit => app.add_listener(hear.at_edit_time()),
            Mark::Preview => app.add_listener(hear.at_preview_time()),
        };
        app.main_mut().world_mut().trigger(Ping);
        app.get_resource::<Count>().unwrap().0
    }

    struct Role {
        name: &'static str,
        app: fn() -> App,
        editing: bool,
        previewing: bool,
        runs: Runs,
    }

    struct Runs {
        unmarked: u32,
        edit: u32,
        preview: u32,
    }

    #[test]
    fn edit_and_preview_marks_decide_which_systems_and_listeners_each_kind_of_app_runs() {
        let cases = [
            Role {
                name: "a game app runs everything",
                app: App::new,
                editing: false,
                previewing: false,
                runs: Runs {
                    unmarked: 1,
                    edit: 1,
                    preview: 1,
                },
            },
            Role {
                name: "an edit app runs edit-time work only",
                app: App::for_editing,
                editing: true,
                previewing: false,
                runs: Runs {
                    unmarked: 0,
                    edit: 1,
                    preview: 0,
                },
            },
            Role {
                name: "a preview app runs edit-time and preview-time work",
                app: App::for_previewing,
                editing: true,
                previewing: true,
                runs: Runs {
                    unmarked: 0,
                    edit: 1,
                    preview: 1,
                },
            },
        ];
        for case in cases {
            assert_eq!((case.app)().is_editing(), case.editing, "{}", case.name);
            assert_eq!(
                (case.app)().is_previewing(),
                case.previewing,
                "{}",
                case.name
            );
            let check = |mark: Mark, runs: u32| {
                assert_eq!(
                    counted_after_one_frame((case.app)(), mark),
                    runs,
                    "{}",
                    case.name
                );
                assert_eq!(
                    heard_after_one_ping((case.app)(), mark),
                    runs,
                    "{}",
                    case.name
                );
            };
            check(Mark::None, case.runs.unmarked);
            check(Mark::Edit, case.runs.edit);
            check(Mark::Preview, case.runs.preview);
        }
    }

    #[derive(Resource, Default)]
    struct RenderTicks(u32);

    #[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
    struct RenderTick;

    fn tick_render(mut ticks: ResMut<RenderTicks>) {
        ticks.0 += 1;
    }

    #[test]
    fn the_render_world_of_an_edit_app_runs_unmarked_systems() {
        let mut app = App::for_editing();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin);
        app.render_mut().set_update_schedule(RenderTick);
        app.render_mut().insert_resource(RenderTicks::default());
        app.add_render_system(RenderTick, tick_render);
        app.finish_plugin_build();
        app.update();
        assert_eq!(
            app.render()
                .world()
                .get_resource::<RenderTicks>()
                .unwrap()
                .0,
            1
        );
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Note;

    impl Asset for Note {
        fn name() -> &'static str {
            "Note"
        }
    }

    impl concerto_foundation::assets::LoadableAsset for Note {}

    #[test]
    fn a_dropped_asset_is_freed_in_an_edit_app() {
        let mut app = App::for_editing();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .register_plugin(plugins::AssetManagerPlugin)
            .register_asset::<Note>();
        app.finish_plugin_build();

        let handle = app.get_resource::<AssetServer>().unwrap().add(Note);
        let id = handle.id();
        app.update();
        let weak = AssetHandle::<Note>::weak(id);
        assert!(app
            .get_resource::<AssetStore<Note>>()
            .unwrap()
            .get(&weak)
            .is_some());

        drop(handle);
        app.update();
        assert!(app
            .get_resource::<AssetStore<Note>>()
            .unwrap()
            .get(&weak)
            .is_none());
    }

    #[derive(Resource, Default)]
    struct NoteStoreLog {
        changed: Vec<bool>,
        loaded: Vec<AssetId>,
    }

    fn log_note_store(
        store: Res<AssetStore<Note>>,
        mut loaded: EventReader<AssetLoaded<Note>>,
        mut log: ResMut<NoteStoreLog>,
    ) {
        log.changed.push(store.is_changed());
        log.loaded.extend(loaded.read().map(|event| event.id));
    }

    #[test]
    fn an_asset_store_reads_as_changed_only_on_a_load_or_a_drop() {
        let mut app = App::new();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .register_plugin(plugins::AssetManagerPlugin)
            .register_asset::<Note>()
            .insert_resource(NoteStoreLog::default())
            .add_system(First, log_note_store);
        app.finish_plugin_build();
        for _ in 0..3 {
            app.update();
        }

        let handle = app.get_resource::<AssetServer>().unwrap().add(Note);
        let id = handle.id();
        for _ in 0..3 {
            app.update();
        }
        drop(handle);
        for _ in 0..3 {
            app.update();
        }

        let log = app.get_resource::<NoteStoreLog>().unwrap();
        assert_eq!(
            log.changed[1..],
            [false, false, false, true, false, false, true, false]
        );
        assert_eq!(log.loaded, [id]);
    }

    #[test]
    fn externally_queued_events_survive_until_late_update() {
        let mut app = App::new();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .register_event::<ExternalEvent>()
            .insert_resource(Seen::default())
            .add_system(schedule_groups::LateUpdate, observe);
        app.get_resource_mut::<EventChannel<ExternalEvent>>()
            .unwrap()
            .push_event(ExternalEvent);
        app.finish_plugin_build();

        app.update();

        assert_eq!(app.get_resource::<Seen>().unwrap().0, 1);
        app.update();
        assert_eq!(app.get_resource::<Seen>().unwrap().0, 1);
    }
}

#[cfg(test)]
mod type_registration_tests {
    use super::*;
    use concerto_ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent};
    use concerto_foundation::transform::Transform;
    use concerto_reflect::ReflectDefault;
    use std::any::TypeId;

    fn registry(app: &App) -> &AppTypeRegistry {
        app.get_resource::<AppTypeRegistry>().unwrap()
    }

    #[test]
    fn the_transform_plugin_registers_transform_with_everything_it_declares() {
        let mut app = App::new();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .register_plugin(plugins::TransformPlugin);

        let registration = registry(&app)
            .read()
            .get(TypeId::of::<Transform>())
            .unwrap();
        assert_eq!(registration.short_name(), "Transform");
        assert!(registration.data::<ReflectComponent>().is_some());
        assert!(registration.data::<ReflectDefault>().is_some());
        assert!(registration.data::<ReflectSceneComponent>().is_some());
    }
}
