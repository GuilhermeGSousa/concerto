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
    component::{scene::SceneComponent, Component},
    events::{
        event_channel::{update_event_channel, EventChannel},
        event_writer::EventWriter,
        Event,
    },
    query::trait_query::ImplementedBy,
    resource::{ResMut, Resource},
    system::schedule::{CompiledSchedules, ScheduleLabel, Schedules},
    IntoSetConfig, IntoSystemConfig, World,
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

pub mod extractor;
pub mod main_schedule;
pub mod plugins;
pub mod runner;
pub mod schedule_groups;
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

/// The top-level container: the main and render [`SubApp`]s and the registered [`Plugin`]s.
pub struct App {
    runner: runner::RunnerFn,
    subapps: SubApps,
    plugins: Vec<Box<dyn Plugin>>,
    plugin_state: PluginsState,
}

impl App {
    pub fn new() -> App {
        Self {
            runner: Box::new(runner::run_once),
            subapps: SubApps::default(),
            plugins: Vec::new(),
            plugin_state: PluginsState::Building,
        }
    }

    /// Builds `plugin` immediately and keeps it for [`Plugin::ready`] and [`Plugin::finish`].
    pub fn register_plugin(&mut self, plugin: impl Plugin + 'static) -> &mut Self {
        info!("Registering plugin: {}", plugin.name());
        plugin.build(self);
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Registers an asset type and the system that tracks its handles; requires [`AssetManagerPlugin`](plugins::AssetManagerPlugin).
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

    pub fn run(mut self) {
        let runner = std::mem::replace(&mut self.runner, Box::new(run_once));
        (runner)(self);
    }

    /// Replaces the runner, for example with a window event loop.
    pub fn set_runner(&mut self, f: impl FnOnce(App) -> AppExit + 'static) -> &mut Self {
        self.runner = Box::new(f);
        self
    }

    /// Adds systems to the main world's `update_group` schedule. Panics once the schedules have been compiled.
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

    /// Adds systems to the render world's `update_group` schedule. Panics once the schedules have been compiled.
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

    /// Adds ordering constraints between sets in the main world's `update_group` schedule. Panics once the schedules have been compiled.
    pub fn configure_sets(
        &mut self,
        update_group: impl ScheduleLabel,
        configs: impl IntoSetConfig,
    ) -> &mut Self {
        self.main_mut().configure_sets(update_group, configs);
        self
    }

    /// Adds ordering constraints between sets in the render world's `update_group` schedule. Panics once the schedules have been compiled.
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

    /// Registers an event type; without this its buffers are never swapped and grow forever.
    pub fn register_event<T: Event + 'static>(&mut self) -> &mut Self {
        let event_channel = EventChannel::<T>::new();

        self.insert_resource(event_channel);
        self.add_system(First, update_event_channel::<T>);
        self
    }

    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.main_mut().insert_resource(value);
        self
    }

    /// Registers a [`SceneComponent`] so scenes can spawn it by type name.
    pub fn register_scene_component<T: SceneComponent>(&mut self) -> &mut Self {
        self.main_mut().register_scene_component::<T>();
        self
    }

    /// Registers `C` as an implementor of the `#[queryable]` trait `Dyn` on the main world,
    /// so its `All<&Dyn>` queries visit it.
    pub fn register_component_as<Dyn, C>(&mut self) -> &mut Self
    where
        Dyn: ImplementedBy<C> + ?Sized,
        C: Component,
    {
        self.main_mut()
            .world_mut()
            .register_component_as::<Dyn, C>();
        self
    }

    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.main_mut().remove_resource()
    }

    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.main().get_resource()
    }

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

        profiling::finish_frame!();
    }

    pub fn main(&self) -> &SubApp {
        self.subapps.main()
    }

    pub fn main_mut(&mut self) -> &mut SubApp {
        self.subapps.main_mut()
    }

    pub fn render(&self) -> &SubApp {
        self.subapps.render()
    }

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

    /// Sets the function that copies main-world data into the render world each frame.
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
