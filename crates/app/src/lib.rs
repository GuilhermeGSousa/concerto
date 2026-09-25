//! The application shell of Concerto: plugins, schedules and the frame loop.
//!
//! An [`App`] owns two [`SubApp`]s, each with its own [`World`]: the main world,
//! where gameplay runs, and the render world, which data is extracted into every
//! frame. [`Plugin`]s add systems and resources to either, and the app's runner
//! drives the frame loop.
//!
//! # Examples
//!
//! ```
//! use concerto_app::{
//!     App,
//!     main_schedule::MainSchedulePlugin,
//!     plugins::TimePlugin,
//!     schedule_groups::Update,
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
    component::scene::SceneComponent,
    events::{
        event_channel::{update_event_channel, EventChannel},
        event_writer::EventWriter,
        Event,
    },
    resource::{ResMut, Resource},
    system::schedule::{CompiledSchedules, ScheduleLabel, Schedules},
    IntoSetConfigs, IntoSystemConfig, IntoSystemConfigs, World,
};
use log::info;
use runner::AppExit;

use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, handle::AssetLifetimeEvent, Asset,
};

use crate::{
    plugins::PluginsState,
    runner::run_once,
    schedule_groups::{First, Update},
    subapp::{SubApp, SubApps},
};

/// Copying data from the main world into the render world.
pub mod extractor;
/// The schedule that drives one frame of the main world.
pub mod main_schedule;
/// The [`Plugin`] trait and the engine's core plugins.
pub mod plugins;
/// Runners that drive the frame loop.
pub mod runner;
/// The labels of the engine's built-in schedules.
pub mod schedule_groups;
/// The main and render sub-applications.
pub mod subapp;

pub use plugins::Plugin;

pub(crate) struct HokeyPokeyPlugin;
impl Plugin for HokeyPokeyPlugin {
    fn build(&self, _: &mut App) {}
}

fn compile(schedules: Schedules, world: &mut World) -> CompiledSchedules {
    #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
    {
        schedules.compile::<MultiThreadedExecutor>(world)
    }
    #[cfg(not(all(feature = "multithreaded", not(target_arch = "wasm32"))))]
    {
        schedules.compile::<SingleThreadedExecutor>(world)
    }
}

/// The top-level container for the engine.
///
/// An `App` owns the main and render [`SubApp`]s, their schedules and the registered
/// [`Plugin`]s. Call [`run`](App::run) to hand control to the configured runner,
/// typically the window event loop.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, plugins::TimePlugin};
///
/// let mut app = App::new();
/// app.register_plugin(TimePlugin);
/// app.run();
/// ```
pub struct App {
    runner: runner::RunnerFn,
    subapps: SubApps,
    plugins: Vec<Box<dyn Plugin>>,
    plugin_state: PluginsState,
}

impl App {
    /// Creates an app with no plugins and a runner that returns immediately.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let app = App::new();
    /// ```
    pub fn new() -> App {
        Self {
            runner: Box::new(runner::run_once),
            subapps: SubApps::default(),
            plugins: Vec::new(),
            plugin_state: PluginsState::Building,
        }
    }

    /// Builds and registers a [`Plugin`].
    ///
    /// Calls [`Plugin::build`] immediately, then keeps the plugin so that
    /// [`Plugin::ready`] and [`Plugin::finish`] can be called later.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, plugins::TimePlugin};
    ///
    /// let mut app = App::new();
    /// app.register_plugin(TimePlugin);
    /// ```
    pub fn register_plugin(&mut self, plugin: impl Plugin + 'static) -> &mut Self {
        info!("Registering plugin: {}", plugin.name());
        plugin.build(self);
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Registers an asset type, creating its [`AssetStore`] and the system that tracks its
    /// handles.
    ///
    /// # Panics
    ///
    /// Panics if [`AssetManagerPlugin`](plugins::AssetManagerPlugin) is not registered, or
    /// if called after [`finish_plugin_build`](App::finish_plugin_build).
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, plugins::AssetManagerPlugin};
    /// use concerto_foundation::assets::Asset;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct Level {
    ///     name: String,
    /// }
    ///
    /// impl Asset for Level {
    ///     fn name() -> &'static str {
    ///         "Level"
    ///     }
    /// }
    ///
    /// let mut app = App::new();
    /// app.register_plugin(AssetManagerPlugin).register_asset::<Level>();
    /// ```
    pub fn register_asset<A: Asset>(&mut self) -> &mut Self {
        let asset_store = AssetStore::<A>::new();
        let asset_server = self
            .get_resource_mut::<AssetServer>()
            .expect("Asset Server not found");

        asset_server.register_asset::<A>(&asset_store);

        self.add_system(
            Update,
            |mut asset_store: ResMut<AssetStore<A>>,
             asset_server: ResMut<AssetServer>,
             events: EventWriter<AssetLifetimeEvent>| {
                asset_store.track_assets(asset_server, events);
            },
        );

        self.main_mut().insert_resource(asset_store);
        self
    }

    /// Hands control to the configured runner, consuming the app.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// App::new().run();
    /// ```
    pub fn run(mut self) {
        let runner = std::mem::replace(&mut self.runner, Box::new(run_once));
        (runner)(self);
    }

    /// Replaces the runner, for example with a window event loop.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, runner::AppExit};
    ///
    /// let mut app = App::new();
    /// app.set_runner(|mut app| {
    ///     app.finish_plugin_build();
    ///     app.update();
    ///     AppExit::Success
    /// });
    /// app.run();
    /// ```
    pub fn set_runner(&mut self, f: impl FnOnce(App) -> AppExit + 'static) -> &mut Self {
        self.runner = Box::new(f);
        self
    }

    /// Adds a system to the main world's schedule labelled `update_group`.
    ///
    /// See [`Schedule::add_system`](concerto_ecs::Schedule::add_system).
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Update};
    ///
    /// fn tick() {}
    ///
    /// let mut app = App::new();
    /// app.add_system(Update, tick);
    /// ```
    pub fn add_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M> + 'static,
    ) -> &mut Self {
        self.get_resource_mut::<Schedules>()
            .expect("Schedules resource not found!")
            .add_system(update_group, system);

        self
    }

    /// Adds a system to the render world's schedule labelled `update_group`, such as
    /// [`Extract`](schedule_groups::Extract).
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Render};
    ///
    /// fn draw() {}
    ///
    /// let mut app = App::new();
    /// app.add_render_system(Render, draw);
    /// ```
    pub fn add_render_system<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        system: impl IntoSystemConfig<M> + 'static,
    ) -> &mut Self {
        self.subapps
            .render_mut()
            .get_resource_mut::<Schedules>()
            .expect("Schedules resource not found on render subapp!")
            .add_system(update_group, system);

        self
    }

    /// Adds several systems to the main world's schedule labelled `update_group`.
    ///
    /// See [`Schedule::add_systems`](concerto_ecs::Schedule::add_systems).
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Update};
    /// use concerto_ecs::IntoSystemConfigs;
    ///
    /// fn read_input() {}
    /// fn move_player() {}
    ///
    /// let mut app = App::new();
    /// app.add_systems(Update, (read_input, move_player).chain());
    /// ```
    pub fn add_systems<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        systems: impl IntoSystemConfigs<M>,
    ) -> &mut Self {
        self.main_mut().add_systems(update_group, systems);
        self
    }

    /// Adds several systems to the render world's schedule labelled `update_group`.
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Render};
    /// use concerto_ecs::IntoSystemConfigs;
    ///
    /// fn prepare() {}
    /// fn draw() {}
    ///
    /// let mut app = App::new();
    /// app.add_render_systems(Render, (prepare, draw).chain());
    /// ```
    pub fn add_render_systems<M>(
        &mut self,
        update_group: impl ScheduleLabel,
        systems: impl IntoSystemConfigs<M>,
    ) -> &mut Self {
        self.subapps.render_mut().add_systems(update_group, systems);
        self
    }

    /// Adds ordering constraints between sets in the main world's schedule labelled
    /// `update_group`.
    ///
    /// See [`Schedule::configure_sets`](concerto_ecs::Schedule::configure_sets).
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Update};
    /// use concerto_ecs::{IntoSetConfigs, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Phase {
    ///     Prepare,
    ///     Draw,
    /// }
    ///
    /// let mut app = App::new();
    /// app.configure_sets(Update, (Phase::Prepare, Phase::Draw).chain());
    /// ```
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfigs,
    ) -> &mut Self {
        self.main_mut().configure_sets(update_group, configs);
        self
    }

    /// Adds ordering constraints between sets in the render world's schedule labelled
    /// `update_group`.
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, schedule_groups::Render};
    /// use concerto_ecs::{IntoSetConfigs, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Phase {
    ///     Prepare,
    ///     Draw,
    /// }
    ///
    /// let mut app = App::new();
    /// app.configure_render_sets(Render, (Phase::Prepare, Phase::Draw).chain());
    /// ```
    pub fn configure_render_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfigs,
    ) -> &mut Self {
        self.subapps
            .render_mut()
            .configure_sets(update_group, configs);
        self
    }

    /// Registers an event type, creating its [`EventChannel`] and the system that swaps its
    /// buffers once per frame.
    ///
    /// Call this once per event type before any system uses [`EventWriter`] or
    /// [`EventReader`](concerto_ecs::events::event_reader::EventReader); without it the
    /// buffers are never swapped and grow forever.
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish_plugin_build`](App::finish_plugin_build), once the
    /// schedules have been compiled.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Event;
    ///
    /// #[derive(Event)]
    /// struct Jumped;
    ///
    /// let mut app = App::new();
    /// app.register_event::<Jumped>();
    /// ```
    pub fn register_event<T: Event + 'static>(&mut self) -> &mut Self {
        let event_channel = EventChannel::<T>::new();

        self.insert_resource(event_channel);
        self.add_system(First, update_event_channel::<T>);
        self
    }

    /// Inserts a resource into the main world, replacing any of the same type.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut app = App::new();
    /// app.insert_resource(Score(0));
    /// ```
    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.main_mut().insert_resource(value);
        self
    }

    /// Registers a [`SceneComponent`] so serialized scenes and glTF `extras` can spawn it
    /// from JSON by type name.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_foundation::transform::Transform;
    ///
    /// let mut app = App::new();
    /// app.register_scene_component::<Transform>();
    /// ```
    pub fn register_scene_component<T: SceneComponent>(&mut self) -> &mut Self {
        self.main_mut().register_scene_component::<T>();
        self
    }

    /// Removes a resource from the main world and returns it, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut app = App::new();
    /// app.insert_resource(Score(0));
    ///
    /// assert_eq!(app.remove_resource::<Score>().map(|score| score.0), Some(0));
    /// assert!(app.get_resource::<Score>().is_none());
    /// ```
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.main_mut().remove_resource()
    }

    /// Returns a resource of the main world, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut app = App::new();
    /// app.insert_resource(Score(0));
    ///
    /// assert_eq!(app.get_resource::<Score>().map(|score| score.0), Some(0));
    /// ```
    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.main().get_resource()
    }

    /// Returns a resource of the main world mutably, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Score(u64);
    ///
    /// let mut app = App::new();
    /// app.insert_resource(Score(0));
    ///
    /// if let Some(score) = app.get_resource_mut::<Score>() {
    ///     score.0 += 10;
    /// }
    /// ```
    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.main_mut().get_resource_mut()
    }

    /// Replaces resource `R` of the main world with the resource `f` builds from it.
    ///
    /// Does nothing if `R` is not present.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    /// use concerto_ecs::Resource;
    ///
    /// #[derive(Resource)]
    /// struct Settings(u32);
    ///
    /// #[derive(Resource)]
    /// struct Resolved(u32);
    ///
    /// let mut app = App::new();
    /// app.insert_resource(Settings(2));
    /// app.with_resource(|settings: Settings| Resolved(settings.0 * 2));
    /// assert_eq!(app.get_resource::<Resolved>().map(|resolved| resolved.0), Some(4));
    /// ```
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

    /// Runs one frame: the main world's update schedule, extraction into the render
    /// world, then the render world's update schedule.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    ///
    /// let mut app = App::new();
    /// app.register_plugin(MainSchedulePlugin).register_plugin(TimePlugin);
    /// app.finish_plugin_build();
    /// app.update();
    /// ```
    pub fn update(&mut self) {
        profiling::scope!("App::update");

        self.subapps.update();

        profiling::finish_frame!();
    }

    /// Returns the main sub-application, where gameplay runs.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// let world = app.main().world();
    /// ```
    pub fn main(&self) -> &SubApp {
        self.subapps.main()
    }

    /// Returns the main sub-application mutably.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// let world = app.main_mut().world_mut();
    /// ```
    pub fn main_mut(&mut self) -> &mut SubApp {
        self.subapps.main_mut()
    }

    /// Returns the render sub-application, which data is extracted into every frame.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// let world = app.render().world();
    /// ```
    pub fn render(&self) -> &SubApp {
        self.subapps.render()
    }

    /// Returns the render sub-application mutably.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// let world = app.render_mut().world_mut();
    /// ```
    pub fn render_mut(&mut self) -> &mut SubApp {
        self.subapps.render_mut()
    }

    /// Polls every plugin's [`ready`](Plugin::ready) and returns the resulting
    /// [`PluginsState`].
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::{App, plugins::PluginsState};
    ///
    /// let mut app = App::new();
    /// assert_eq!(app.plugin_state(), PluginsState::Ready);
    /// ```
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

    /// Calls [`Plugin::finish`] on every plugin, compiles every schedule, then runs the
    /// [`Startup`](schedule_groups::Startup) schedule in both worlds.
    ///
    /// Call it once, after every plugin is registered and
    /// [`plugin_state`](App::plugin_state) reports [`PluginsState::Ready`].
    ///
    /// # Panics
    ///
    /// Panics if called more than once, or if any schedule's explicit ordering constraints
    /// contain a cycle.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// app.finish_plugin_build();
    /// ```
    pub fn finish_plugin_build(&mut self) {
        let mut hokeypokey: Box<dyn Plugin> = Box::new(HokeyPokeyPlugin);
        let mut i = 0;
        while i < self.plugins.len() {
            core::mem::swap(&mut self.plugins[i], &mut hokeypokey);
            hokeypokey.finish(self);
            core::mem::swap(&mut self.plugins[i], &mut hokeypokey);
            i += 1;
        }

        self.plugin_state = PluginsState::Finished;

        self.compile_schedules();
        self.compile_render_schedules();

        self.subapps.startup();
    }

    fn compile_schedules(&mut self) {
        let schedules = self
            .remove_resource::<Schedules>()
            .expect("Schedules resource not found!");

        let world = self.main_mut().world_mut();
        let compiled_schedules = compile(schedules, world);
        self.insert_resource(compiled_schedules);
    }

    fn compile_render_schedules(&mut self) {
        let schedules = self
            .subapps
            .render_mut()
            .remove_resource::<Schedules>()
            .expect("Schedules resource not found on render subapp!");

        let world = self.render_mut().world_mut();
        let compiled_schedules = compile(schedules, world);
        self.subapps
            .render_mut()
            .insert_resource(compiled_schedules);
    }

    /// Sets the function that copies data from the main world into the render world
    /// before the render world updates.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_app::App;
    ///
    /// let mut app = App::new();
    /// app.set_extract_fn(|_main, _render| {});
    /// ```
    pub fn set_extract_fn(&mut self, extract_fn: impl FnMut(&mut World, &mut World) + 'static) {
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
    use concerto_ecs::events::{event_channel::EventChannel, event_reader::EventReader};

    #[derive(Event)]
    struct ExternalEvent;

    #[derive(Resource, Default)]
    struct Seen(usize);

    fn observe(mut events: EventReader<ExternalEvent>, mut seen: ResMut<Seen>) {
        seen.0 += events.read().count();
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
