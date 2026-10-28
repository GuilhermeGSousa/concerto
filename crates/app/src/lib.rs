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
    events::{
        event_channel::{update_event_channel, EventChannel},
        event_writer::EventWriter,
        Event,
    },
    resource::{ResMut, Resource},
    signal::Signal,
    system::schedule::{CompiledSchedules, ScheduleLabel, Schedules},
    IntoSetConfig, IntoSystemConfig, SystemTag, TagFilter, World,
};
use log::info;

use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, handle::AssetLifetimeEvent, Asset, AssetId,
};

use crate::{
    edit_time::{AppListener, AtEditTime, EditTime},
    plugins::PluginsState,
    schedule_groups::{First, Update},
    subapp::{SubApp, SubApps},
};

pub mod edit_time;
pub mod extractor;
pub mod main_schedule;
pub mod plugins;
pub mod schedule_groups;
pub mod subapp;

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

/// The top-level container: the main and render [`SubApp`]s and the registered [`Plugin`]s.
pub struct App {
    subapps: SubApps,
    plugins: Vec<Box<dyn Plugin>>,
    plugin_types: std::collections::HashSet<std::any::TypeId>,
    plugin_state: PluginsState,
    editing: bool,
}

impl App {
    pub fn new() -> App {
        Self {
            subapps: SubApps::default(),
            plugins: Vec::new(),
            plugin_types: Default::default(),
            plugin_state: PluginsState::Building,
            editing: false,
        }
    }

    /// Creates an edit `App`: only main-world systems and listeners marked `at_edit_time` run in it.
    pub fn for_editing() -> App {
        App {
            editing: true,
            ..App::new()
        }
    }

    /// Declares the scene a game starts in; `ScenePlugin` spawns it in a game `App`, never in an edit `App`.
    pub fn set_startup_scene(&mut self, id: AssetId) -> &mut Self {
        self.insert_resource(StartupScene(id))
    }

    /// Whether this `App` was created by [`App::for_editing`].
    pub fn is_editing(&self) -> bool {
        self.editing
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

    /// Registers an asset type and the system that tracks its handles; requires [`AssetManagerPlugin`](plugins::AssetManagerPlugin).
    pub fn register_asset<A: Asset>(&mut self) -> &mut Self {
        let asset_store = AssetStore::<A>::new();
        let asset_server = self
            .get_resource_mut::<AssetServer>()
            .expect("Asset Server not found");

        asset_server.register_asset::<A>(&asset_store);

        self.add_system(
            Update,
            (|mut asset_store: ResMut<AssetStore<A>>,
              asset_server: ResMut<AssetServer>,
              events: EventWriter<AssetLifetimeEvent>| {
                asset_store.track_assets(asset_server, events);
            })
            .at_edit_time(),
        );

        self.main_mut().insert_resource(asset_store);
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

    /// Registers an event type; without this its `EventChannel` resource never exists, so systems taking `EventWriter` or `EventReader` for it are skipped.
    pub fn register_event<T: Event + 'static>(&mut self) -> &mut Self {
        let event_channel = EventChannel::<T>::new();

        self.insert_resource(event_channel);
        self.add_system(First, update_event_channel::<T>.at_edit_time());
        self
    }

    pub fn insert_resource<R: Resource>(&mut self, value: R) -> &mut Self {
        self.main_mut().insert_resource(value);
        self
    }

    /// Runs `listener` every time a `T` is triggered in the main world; an edit `App` keeps only marked ones.
    pub fn add_listener<T: Signal, M>(&mut self, listener: impl AppListener<T, M>) -> &mut Self {
        let editing = self.editing;
        listener.add_to(self.main_mut().world_mut(), editing);
        self
    }

    /// Registers `T` in the main world's type registry with the type data its derive declared.
    pub fn register_type<T: concerto_reflect::GetTypeRegistration>(&mut self) -> &mut Self {
        self.main_mut().register_type::<T>();
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
    }

    /// Runs one main-world schedule between frames, for a host.
    pub fn serve(&mut self, label: impl ScheduleLabel) {
        self.main_mut().world_mut().run_schedule(label);
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

        let filter = if self.editing {
            TagFilter {
                include: vec![EditTime.intern()],
                exclude: Vec::new(),
            }
        } else {
            TagFilter::default()
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
    use concerto_ecs::events::{event_channel::EventChannel, event_reader::EventReader};

    #[derive(Event)]
    struct ExternalEvent;

    #[derive(Resource, Default)]
    struct Seen(usize);

    fn observe(mut events: EventReader<ExternalEvent>, mut seen: ResMut<Seen>) {
        seen.0 += events.read().count();
    }

    use crate::edit_time::AtEditTime;
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

    fn counted_after_one_frame(mut app: App, marked: bool) -> u32 {
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .insert_resource(Count::default());
        if marked {
            app.add_system(Update, count.at_edit_time());
        } else {
            app.add_system(Update, count);
        }
        app.finish_plugin_build();
        app.update();
        app.get_resource::<Count>().unwrap().0
    }

    fn heard_after_one_ping(mut app: App, marked: bool) -> u32 {
        app.insert_resource(Count::default());
        if marked {
            app.add_listener(hear.at_edit_time());
        } else {
            app.add_listener(hear);
        }
        app.main_mut().world_mut().trigger(Ping);
        app.get_resource::<Count>().unwrap().0
    }

    #[test]
    fn an_app_for_editing_reports_its_role() {
        assert!(App::for_editing().is_editing());
        assert!(!App::new().is_editing());
    }

    #[test]
    fn only_marked_systems_run_in_an_edit_app() {
        assert_eq!(counted_after_one_frame(App::new(), false), 1);
        assert_eq!(counted_after_one_frame(App::new(), true), 1);
        assert_eq!(counted_after_one_frame(App::for_editing(), false), 0);
        assert_eq!(counted_after_one_frame(App::for_editing(), true), 1);
    }

    fn count_ten(mut count: ResMut<Count>) {
        count.0 += 10;
    }

    #[test]
    fn marking_a_tuple_marks_every_system_in_it() {
        let mut app = App::for_editing();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .insert_resource(Count::default())
            .add_system(Update, (count, count_ten).at_edit_time());
        app.finish_plugin_build();
        app.update();
        assert_eq!(app.get_resource::<Count>().unwrap().0, 11);
    }

    #[test]
    fn only_marked_listeners_are_added_to_an_edit_app() {
        assert_eq!(heard_after_one_ping(App::new(), false), 1);
        assert_eq!(heard_after_one_ping(App::new(), true), 1);
        assert_eq!(heard_after_one_ping(App::for_editing(), false), 0);
        assert_eq!(heard_after_one_ping(App::for_editing(), true), 1);
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
mod thread_safety_tests {
    use super::*;

    struct Marked;
    impl Plugin for Marked {
        fn build(&self, _: &mut App) {}
    }

    struct Absent;
    impl Plugin for Absent {
        fn build(&self, _: &mut App) {}
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn an_app_can_cross_threads() {
        assert_send_sync::<App>();
    }

    #[test]
    fn a_registered_plugin_is_reported_as_added() {
        let mut app = App::new();
        app.register_plugin(Marked);

        assert!(app.is_plugin_added::<Marked>());
        assert!(!app.is_plugin_added::<Absent>());
    }

    #[test]
    fn a_plugin_registered_by_another_plugin_is_reported_as_added() {
        struct Outer;
        impl Plugin for Outer {
            fn build(&self, app: &mut App) {
                app.register_plugin(Marked);
            }
        }
        let mut app = App::new();
        app.register_plugin(Outer);

        assert!(app.is_plugin_added::<Marked>());
        assert!(app.is_plugin_added::<Outer>());
    }
}

#[cfg(test)]
mod type_registration_tests {
    use super::*;
    use concerto_ecs::{
        reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
        Component,
    };
    use concerto_foundation::transform::Transform;
    use concerto_reflect::{Reflect, ReflectDefault};
    use std::any::TypeId;

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct Health {
        points: f32,
    }

    fn registry(app: &App) -> &AppTypeRegistry {
        app.get_resource::<AppTypeRegistry>().unwrap()
    }

    #[test]
    fn the_transform_plugin_registers_transform_with_everything_it_declares() {
        let mut app = App::new();
        app.register_plugin(main_schedule::MainSchedulePlugin)
            .register_plugin(plugins::TimePlugin)
            .register_plugin(plugins::TransformPlugin);

        let registration = registry(&app).get(TypeId::of::<Transform>()).unwrap();
        assert_eq!(registration.short_name(), "Transform");
        assert!(registration.data::<ReflectComponent>().is_some());
        assert!(registration.data::<ReflectDefault>().is_some());
        assert!(registration.data::<ReflectSceneComponent>().is_some());
    }

    #[test]
    fn a_recipe_registers_its_own_types() {
        let mut app = App::new();

        app.register_type::<Health>();

        let registration = registry(&app).get(TypeId::of::<Health>()).unwrap();
        assert!(registration.data::<ReflectComponent>().is_some());
        assert!(registration.data::<ReflectDefault>().is_some());
        assert!(registration.data::<ReflectSceneComponent>().is_none());
        let _ = Health::default().points;
    }
}
