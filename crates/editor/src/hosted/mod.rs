//! `App`s the editor holds as values: building, stepping, and the messages in and out.
pub mod events;
pub mod input;
pub mod seam;
pub mod target;
#[cfg(test)]
pub(crate) mod testing;

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
};

use concerto_app::{App, Plugin, plugins::PluginsState, schedule_groups::First};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, World,
    command::CommandQueue,
    events::event_channel::EventChannel,
    signal::{On, Signal},
    system::NonSendMarker,
};
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_render::{
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    wgpu,
};
use concerto_scene::scene::Scene;
use concerto_window::{
    events::WindowEvent,
    plugin::{Window, WindowCommand},
};

pub use events::{Hosted, HostedAppEvents, HostedEventsAppExt};
pub use seam::{EditWorld, EditWorldSignals};
pub use target::{EditTarget, SetEditTarget};

pub const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Whether a host builds an `App` to edit a scene in or to play the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRole {
    Edit,
    Game,
}

/// What a host supplies when it builds an `App`.
pub struct HostRequest<'a> {
    pub gpu: Option<&'a Gpu>,
    pub recipe: fn(&mut App),
    pub size: (u32, u32),
    pub scale_factor: f64,
    pub role: HostRole,
}

/// An `App` held on a document. Dropping the component drops the `App`.
#[derive(Component)]
pub struct HostedApp {
    app: App,
    picture: SharedTexture,
}

impl HostedApp {
    /// The texture this `App` draws its window into.
    pub fn picture(&self) -> &SharedTexture {
        &self.picture
    }

    pub(crate) fn capture_scene(&mut self) -> Result<Scene, String> {
        crate::tools::scene::capture_open_scene(self.app.main_mut().world_mut())
    }

    pub(crate) fn mark_saved(&mut self) {
        crate::tools::scene::mark_saved(self.app.main_mut().world_mut());
    }

    pub(crate) fn scene_dirty(&self) -> bool {
        self.world()
            .get_resource::<crate::tools::scene::SceneDirty>()
            .is_some_and(|dirty| dirty.0)
    }

    fn world(&self) -> &World {
        self.app.main().world()
    }

    #[cfg(test)]
    pub(crate) fn world_for_test(&self) -> &World {
        self.world()
    }

    pub(crate) fn push(&mut self, event: WindowEvent) {
        if let Some(channel) = self.app.get_resource_mut::<EventChannel<WindowEvent>>() {
            channel.push_event(event);
        }
    }

    pub(crate) fn step(&mut self) {
        self.app.update();
    }

    pub(crate) fn trigger<S: Signal>(&mut self, signal: S) {
        self.app.main_mut().world_mut().trigger(signal);
    }

    pub(crate) fn window(&self) -> Option<&Window> {
        self.app.get_resource::<Window>()
    }

    pub(crate) fn drop_window_commands(&mut self) -> bool {
        self.app
            .get_resource_mut::<Window>()
            .is_some_and(|window| window.drain_commands().contains(&WindowCommand::Close))
    }

    fn cancel_loads(&self) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.cancel_pending();
        }
    }
}

/// An `App` whose plugins are not all ready yet.
#[derive(Component)]
pub struct PendingApp {
    hosted: Option<HostedApp>,
    on_ready: Option<Box<dyn FnOnce(&mut HostedApp) + Send + Sync>>,
    started: std::time::Instant,
    cancelled: bool,
    failure: Option<String>,
}

impl PendingApp {
    pub(crate) fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// The panic message of a recipe, plugin step, or ready step that failed; such an `App` never becomes ready.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn finish_if_ready(&mut self) -> Option<HostedApp> {
        if self.cancelled || self.failure.is_some() {
            return None;
        }
        let mut hosted = self.hosted.take()?;
        let on_ready = &mut self.on_ready;
        let mut step = "a plugin's ready";
        let finished = catch_unwind(AssertUnwindSafe(|| {
            if hosted.app.plugin_state() != PluginsState::Ready {
                return false;
            }
            step = "finishing the build";
            hosted.app.finish_plugin_build();
            step = "the ready step";
            if let Some(on_ready) = on_ready.take() {
                on_ready(&mut hosted);
            }
            true
        }));
        match finished {
            Ok(true) => {
                log::info!("Hosted App ready in {:?}", self.started.elapsed());
                Some(hosted)
            }
            Ok(false) => {
                self.hosted = Some(hosted);
                None
            }
            Err(payload) => {
                self.failure = Some(panic_message(payload, step));
                None
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn finish_now(mut self) -> Option<HostedApp> {
        self.finish_if_ready()
    }
}

/// Starts building an `App`; `on_ready` runs once on it before it becomes a [`HostedApp`].
pub fn begin(
    request: HostRequest<'_>,
    extra: impl FnOnce(&mut App),
    on_ready: impl FnOnce(&mut HostedApp) + Send + Sync + 'static,
) -> PendingApp {
    let started = std::time::Instant::now();
    let picture = SharedTexture::new(PICTURE_FORMAT);
    let mut app = match request.role {
        HostRole::Edit => App::for_editing(),
        HostRole::Game => App::new(),
    };
    if let Some(gpu) = request.gpu {
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::Texture(picture.clone()));
    }
    app.insert_resource(Window::new(request.size, request.scale_factor));
    let built = catch_unwind(AssertUnwindSafe(|| {
        (request.recipe)(&mut app);
        extra(&mut app);
    }));
    let (hosted, failure) = match built {
        Ok(()) => (Some(HostedApp { app, picture }), None),
        Err(payload) => (None, Some(panic_message(payload, "the recipe"))),
    };
    PendingApp {
        hosted,
        on_ready: Some(Box::new(on_ready)),
        started,
        cancelled: false,
        failure,
    }
}

fn panic_message(payload: Box<dyn Any + Send>, step: &str) -> String {
    let message = match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload
            .downcast_ref::<&str>()
            .map_or_else(|| format!("{step} panicked"), |message| (*message).into()),
    };
    log::error!("Building a hosted App failed in {step}: {message}");
    message
}

#[doc(hidden)]
pub fn finish_blocking(mut pending: PendingApp) -> HostedApp {
    loop {
        if let Some(hosted) = pending.finish_if_ready() {
            return hosted;
        }
        if let Some(failure) = pending.failure() {
            panic!("{failure}");
        }
        std::thread::yield_now();
    }
}

impl HostedApp {
    #[doc(hidden)]
    pub fn step_for_test(&mut self) {
        self.step();
    }

    #[doc(hidden)]
    pub fn push_for_test(&mut self, event: WindowEvent) {
        self.push(event);
    }

    #[doc(hidden)]
    pub fn trigger_for_test<S: Signal>(&mut self, signal: S) {
        self.trigger(signal);
    }
}

/// Triggers `signal` in the `App` hosted on `document`; what its listeners write is forwarded in the next `First`.
pub fn trigger_on<S: Signal>(world: &mut World, document: Entity, signal: S) {
    if let Some(hosted) = world.get_component_for_entity_mut::<HostedApp>(document) {
        hosted.trigger(signal);
    }
}

pub(crate) fn finish_pending(
    _: NonSendMarker,
    pending: Query<(Entity, &mut PendingApp)>,
    parked: Query<&ParkedEditApp>,
    starting: Query<&PendingPlayApp>,
    mut cmd: CommandQueue,
) {
    for (document, mut pending) in pending.iter() {
        if parked.get_entity(document).is_some() || starting.get_entity(document).is_some() {
            continue;
        }
        if let Some(hosted) = pending.finish_if_ready() {
            cmd.remove::<PendingApp>(document);
            cmd.insert(hosted, document);
            cmd.trigger(SetEditTarget(Some(document)));
        }
    }
}

/// Whether a Scene document is being edited, is starting its game, or is playing it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Editing,
    Starting,
    Playing,
}

impl PlayState {
    /// Whether the game is starting or playing.
    pub fn is_on(self) -> bool {
        self != PlayState::Editing
    }
}

/// Triggers `StopPlay` when the document's game is on and `Play` otherwise.
pub(crate) fn toggle_play(
    document: Entity,
    state: Option<PlayState>,
    cmd: &mut concerto_ecs::command::CommandQueue,
) {
    if state.is_some_and(PlayState::is_on) {
        cmd.trigger(StopPlay(document));
    } else {
        cmd.trigger(crate::scene::Play(document));
    }
}

/// A play `App` building beside its document's edit `App`; `finish_pending` never takes it.
#[derive(Component)]
pub struct PendingPlayApp(PendingApp);

impl PendingPlayApp {
    /// Holds `pending`, begun in the [`HostRole::Game`] role, until it is ready to play.
    pub fn new(pending: PendingApp) -> Self {
        Self(pending)
    }

    /// The panic message of a play build that failed.
    pub fn failure(&self) -> Option<&str> {
        self.0.failure()
    }

    fn cancel_loads(&self) {
        if let Some(hosted) = &self.0.hosted {
            hosted.cancel_loads();
        }
    }
}

/// The edit `App` set aside, not stepped, while its document's play `App` is the `HostedApp`.
#[derive(Component)]
pub struct ParkedEditApp(pub(crate) HostedApp);

/// Why the document's last play `App` could not start.
#[derive(Component, Debug)]
pub struct PlayFailure(pub String);

/// Stops the play `App` on a document, starting or playing, and returns to its edit `App`.
pub struct StopPlay(pub Entity);
impl Signal for StopPlay {}

pub(crate) fn finish_pending_play(
    _: NonSendMarker,
    pending: Query<(Entity, &mut PendingPlayApp)>,
    hosted: Query<&mut HostedApp>,
    failures: Query<&PlayFailure>,
    mut cmd: CommandQueue,
) {
    for (document, mut pending) in pending.iter() {
        let Some(mut slot) = hosted.get_entity(document) else {
            continue;
        };
        if let Some(play) = pending.0.finish_if_ready() {
            let mut edit = std::mem::replace(&mut **slot, play);
            edit.push(WindowEvent::Focused(false));
            cmd.remove::<PendingPlayApp>(document);
            cmd.insert(
                (
                    ParkedEditApp(edit),
                    PlayState::Playing,
                    input::InputOwner::Game,
                ),
                document,
            );
            if failures.get_entity(document).is_some() {
                cmd.remove::<PlayFailure>(document);
            }
            cmd.trigger(SetEditTarget(Some(document)));
        } else if let Some(failure) = pending.failure() {
            let failure = PlayFailure(failure.to_owned());
            cmd.remove::<PendingPlayApp>(document);
            cmd.insert((failure, PlayState::Editing), document);
            cmd.trigger(SetEditTarget(Some(document)));
        }
    }
}

pub(crate) fn stop_play(
    on: On<StopPlay>,
    pending: Query<&PendingPlayApp>,
    playing: Query<(&mut HostedApp, &mut ParkedEditApp)>,
    owners: Query<&input::InputOwner>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let mut stopped = false;
    if let Some(pending) = pending.get_entity(document) {
        pending.cancel_loads();
        cmd.remove::<PendingPlayApp>(document);
        stopped = true;
    }
    if let Some((mut hosted, mut parked)) = playing.get_entity(document) {
        std::mem::swap(&mut **hosted, &mut parked.0);
        parked.0.cancel_loads();
        cmd.remove::<ParkedEditApp>(document);
        stopped = true;
    }
    if owners.get_entity(document).is_some() {
        cmd.remove::<input::InputOwner>(document);
    }
    if stopped {
        cmd.insert(PlayState::Editing, document);
        cmd.trigger(SetEditTarget(Some(document)));
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(target::move_edit_target);
    world.add_listener(input::focus_shown_app);
    world.add_listener(input::blur_hidden_app);
    world.add_listener(input::focus_new_target);
    world.add_listener(input::blur_old_target);
    world.add_listener(stop_play);
    world.add_listener(input::release_input);
}

pub struct HostedPlugin;

impl Plugin for HostedPlugin {
    fn build(&self, app: &mut App) {
        install_listeners(app.main_mut().world_mut());
        app.insert_resource(input::Routing::default())
            .insert_resource(input::HostCursor::default())
            .insert_resource(input::ViewportShown::default())
            .add_system(
                First,
                (
                    input::route_play_input.before(concerto_window::FoldInput),
                    finish_pending,
                    finish_pending_play,
                    input::sync_in_viewport,
                    input::step_edit_target,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use concerto_app::{edit_time::AtEditTime, schedule_groups::Update};
    use concerto_ecs::{Res, ResMut, Resource, events::event_writer::EventWriter, signal::On};
    use concerto_window::{
        input::{Input, KeyCode, MouseButton, PhysicalKey},
        plugin::WindowCommand,
    };
    use glam::Vec2;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyW);

    #[derive(Resource, Default)]
    struct Heard(Vec<(Entity, u32)>);

    fn record(mut events: HostedAppEvents<Echo>, mut heard: ResMut<Heard>) {
        heard.0.extend(
            events
                .read()
                .into_iter()
                .map(|(document, echo)| (document, echo.0)),
        );
    }

    #[derive(Resource)]
    struct Hearing(Option<concerto_ecs::system::BoxedSystem>);

    fn listen(world: &mut World) {
        let mut system: concerto_ecs::system::BoxedSystem =
            Box::new(concerto_ecs::IntoSystem::into_system(record));
        concerto_ecs::System::initialize(&mut system, world);
        world.insert_resource(Heard::default());
        world.insert_resource(Hearing(Some(system)));
    }

    fn hear(world: &mut World) -> Vec<(Entity, u32)> {
        let mut system = world
            .get_resource_mut::<Hearing>()
            .unwrap()
            .0
            .take()
            .unwrap();
        concerto_ecs::System::run_and_apply(&mut system, (), world);
        world.get_resource_mut::<Hearing>().unwrap().0 = Some(system);
        world.get_resource::<Heard>().unwrap().0.clone()
    }

    #[derive(Resource)]
    struct Canary(Arc<AtomicBool>);

    impl Drop for Canary {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn canary() -> (Arc<AtomicBool>, impl FnOnce(&mut App)) {
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&dropped);
        (dropped, move |app: &mut App| {
            app.insert_resource(Canary(flag));
        })
    }

    fn listening_world() -> (World, Entity) {
        let (mut world, document) = host_world(plain_recipe);
        listen(&mut world);
        (world, document)
    }

    fn frames(world: &World, document: Entity) -> usize {
        world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
            .get_resource::<Frames>()
            .unwrap()
            .0
    }

    fn set_active(world: &mut World, document: Option<Entity>) {
        world
            .get_resource_mut::<crate::asset_editor::ActiveEditor>()
            .unwrap()
            .0 = document;
        world.trigger(SetEditTarget(document));
    }

    fn run_step(world: &mut World) {
        stepper(world)(world);
    }

    fn stepper(world: &mut World) -> impl FnMut(&mut World) + use<> {
        let mut sync = concerto_ecs::IntoSystem::into_system(input::sync_in_viewport);
        let mut step = concerto_ecs::IntoSystem::into_system(input::step_edit_target);
        concerto_ecs::System::initialize(&mut sync, world);
        concerto_ecs::System::initialize(&mut step, world);
        move |world| {
            concerto_ecs::System::run_and_apply(&mut sync, (), world);
            concerto_ecs::System::run_and_apply(&mut step, (), world);
        }
    }

    fn send(world: &mut World, event: WindowEvent) {
        world
            .get_resource_mut::<EventChannel<WindowEvent>>()
            .unwrap()
            .push_event(event);
    }

    fn press_button(world: &mut World) {
        send(
            world,
            WindowEvent::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            },
        );
    }

    fn move_pointer_outside_the_viewport(world: &mut World) {
        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.over = false;
        }
        send(
            world,
            WindowEvent::CursorMoved {
                position: Vec2::new(900.0, 600.0),
            },
        );
    }

    fn stepping_world() -> (World, Entity) {
        let (mut world, document) = host_world(plain_recipe);
        add_stepping(&mut world);
        (world, document)
    }

    fn add_stepping(world: &mut World) {
        world.insert_resource(Input::new());
        world.insert_resource(EventChannel::<WindowEvent>::new());
        world.insert_resource(input::Routing::default());
        world.insert_resource(input::HostCursor::default());
        world.insert_resource(Window::new((1280, 720), 1.0));
        world.insert_resource(input::ViewportShown(Some(input::ViewportView {
            origin: glam::Vec2::ZERO,
            size: (640, 480),
            over: true,
            typing: false,
        })));
    }

    fn hosted_window(world: &World, document: Entity) -> &Window {
        world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .window()
            .unwrap()
    }

    fn hosted_input(world: &World, document: Entity) -> &Input {
        world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
            .get_resource::<Input>()
            .unwrap()
    }

    #[test]
    fn an_app_that_becomes_ready_is_stepped_in_the_same_frame() {
        let mut app = App::new();
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .insert_resource(crate::asset_editor::ActiveEditor::default())
            .register_plugin(HostedPlugin);
        let document =
            app.main_mut()
                .world_mut()
                .spawn(begin(request(plain_recipe), |_| {}, |_| {}));
        app.get_resource_mut::<crate::asset_editor::ActiveEditor>()
            .unwrap()
            .0 = Some(document);
        app.get_resource_mut::<input::ViewportShown>().unwrap().0 = Some(input::ViewportView {
            origin: glam::Vec2::ZERO,
            size: (640, 480),
            over: true,
            typing: false,
        });
        app.finish_plugin_build();

        app.update();

        assert_eq!(frames(app.main().world(), document), 1);
    }

    #[test]
    fn the_edit_target_is_stepped_and_resized_to_the_viewport() {
        let (mut world, document) = stepping_world();

        run_step(&mut world);
        run_step(&mut world);

        assert_eq!(frames(&world, document), 2);
        let hosted = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert_eq!(hosted.window().unwrap().size(), (640, 480));
    }

    #[test]
    fn a_change_of_the_editor_windows_scale_reaches_the_edit_target() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        step(&mut world);
        assert_eq!(hosted_window(&world, document).scale_factor(), 1.0);

        world.insert_resource(Window::new((1280, 720), 2.0));
        send(&mut world, WindowEvent::ScaleFactorChanged(2.0));
        step(&mut world);

        assert_eq!(hosted_window(&world, document).scale_factor(), 2.0);
    }

    #[test]
    fn an_app_that_is_not_the_edit_target_is_not_stepped() {
        let (mut world, document) = stepping_world();
        set_active(&mut world, None);

        run_step(&mut world);

        assert_eq!(frames(&world, document), 0);
    }

    #[test]
    fn a_hidden_viewport_stops_the_stepping() {
        let (mut world, document) = stepping_world();
        world.get_resource_mut::<input::ViewportShown>().unwrap().0 = None;

        run_step(&mut world);

        assert_eq!(frames(&world, document), 0);
    }

    #[test]
    fn leaving_the_front_releases_what_was_held_with_one_last_step() {
        let (mut world, document) = stepping_world();
        world
            .get_resource_mut::<EventChannel<WindowEvent>>()
            .unwrap()
            .push_event(WindowEvent::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            });
        run_step(&mut world);
        assert!(hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));

        set_active(&mut world, None);
        run_step(&mut world);
        let after_leaving = frames(&world, document);
        run_step(&mut world);

        assert_eq!(after_leaving, 2);
        assert_eq!(frames(&world, document), 2);
        assert!(!hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));
    }

    #[test]
    fn a_key_released_while_the_pointer_is_outside_the_viewport_is_no_longer_held() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        send(
            &mut world,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        step(&mut world);
        assert!(hosted_input(&world, document).is_held(KEY));

        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.over = false;
        }
        send(
            &mut world,
            WindowEvent::Key {
                key: KEY,
                pressed: false,
                text: None,
            },
        );
        step(&mut world);

        assert_eq!(frames(&world, document), 2);
        assert!(!hosted_input(&world, document).is_held(KEY));
    }

    fn hold_a_key_outside_the_viewport_then_drag_into_it(typing: bool) -> (World, Entity) {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.over = false;
            view.typing = typing;
        }
        world
            .get_resource_mut::<Input>()
            .unwrap()
            .update_key_input(KEY, true);
        send(
            &mut world,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        step(&mut world);
        assert!(!hosted_input(&world, document).is_held(KEY));

        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.over = true;
        }
        step(&mut world);
        press_button(&mut world);
        step(&mut world);
        (world, document)
    }

    #[test]
    fn a_key_held_before_a_drag_starts_on_the_viewport_is_held_in_the_hosted_app() {
        let (world, document) = hold_a_key_outside_the_viewport_then_drag_into_it(false);

        assert!(hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));
        assert!(hosted_input(&world, document).is_held(KEY));
    }

    #[test]
    fn a_key_held_while_a_text_field_has_focus_is_not_passed_to_a_drag() {
        let (world, document) = hold_a_key_outside_the_viewport_then_drag_into_it(true);

        assert!(hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));
        assert!(!hosted_input(&world, document).is_held(KEY));
    }

    fn grab(mut window: ResMut<Window>) {
        window.settings.cursor.grabbed = true;
    }

    fn grabbing_recipe(app: &mut App) {
        plain_recipe(app);
        app.add_system(Update, grab.at_edit_time());
    }

    fn grabbing_world() -> World {
        let (mut world, _) = stepping_world();
        let document = world.spawn(hosted_now(grabbing_recipe, |_| {}));
        set_active(&mut world, Some(document));
        world
    }

    #[test]
    fn the_editor_window_is_not_marked_changed_while_the_cursor_capture_stays_the_same() {
        let mut world = grabbing_world();
        run_step(&mut world);
        let written = world.resource_changed_tick::<Window>();
        world.tick();

        run_step(&mut world);

        assert_eq!(world.resource_changed_tick::<Window>(), written);
        assert_ne!(Some(world.current_tick()), written);
    }

    #[test]
    fn a_cursor_capture_is_copied_out_and_released_when_nothing_is_stepped() {
        let mut world = grabbing_world();

        run_step(&mut world);
        assert!(
            world
                .get_resource::<Window>()
                .unwrap()
                .settings
                .cursor
                .grabbed
        );

        set_active(&mut world, None);
        run_step(&mut world);
        assert!(
            !world
                .get_resource::<Window>()
                .unwrap()
                .settings
                .cursor
                .grabbed
        );
    }

    #[test]
    fn replacing_the_app_on_the_active_document_ends_the_drag_that_was_held() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        press_button(&mut world);
        step(&mut world);

        world.insert(hosted_now(plain_recipe, |_| {}), document);
        set_active(&mut world, Some(document));
        move_pointer_outside_the_viewport(&mut world);
        step(&mut world);

        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            input::POINTER_GONE
        );
    }

    fn move_pointer(world: &mut World, position: Vec2) {
        send(world, WindowEvent::CursorMoved { position });
    }

    #[test]
    fn leaving_the_viewport_with_no_button_held_takes_the_hosted_pointer_off_the_picture() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        move_pointer(&mut world, Vec2::new(30.0, 20.0));
        step(&mut world);
        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            Vec2::new(30.0, 20.0)
        );

        move_pointer_outside_the_viewport(&mut world);
        step(&mut world);

        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            input::POINTER_GONE
        );
    }

    #[test]
    fn coming_back_over_the_viewport_tells_the_hosted_app_where_the_pointer_is() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.origin = Vec2::new(10.0, 20.0);
            view.over = false;
        }
        step(&mut world);
        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            input::POINTER_GONE
        );

        world
            .get_resource_mut::<Input>()
            .unwrap()
            .update_mouse_position(100.0, 80.0);
        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.over = true;
        }
        step(&mut world);

        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            Vec2::new(90.0, 60.0)
        );
    }

    #[test]
    fn a_new_app_entered_with_the_pointer_over_the_viewport_is_told_where_it_is() {
        let (mut world, document) = stepping_world();
        world
            .get_resource_mut::<Input>()
            .unwrap()
            .update_mouse_position(40.0, 30.0);
        let mut step = stepper(&mut world);

        step(&mut world);

        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            Vec2::new(40.0, 30.0)
        );
    }

    #[test]
    fn a_drag_that_leaves_the_viewport_keeps_following_the_pointer() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        press_button(&mut world);
        step(&mut world);

        move_pointer_outside_the_viewport(&mut world);
        step(&mut world);

        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            Vec2::new(900.0, 600.0)
        );
    }

    #[derive(Resource)]
    struct Steps(Arc<AtomicUsize>);

    fn count_step(steps: Res<Steps>) {
        steps.0.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn a_replacement_app_is_stepped_once_and_the_app_it_replaced_is_not_stepped_again() {
        let (mut world, document) = stepping_world();
        let old_steps = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&old_steps);
        world.insert(
            hosted_now(plain_recipe, move |app| {
                app.insert_resource(Steps(counter))
                    .add_system(Update, count_step.at_edit_time());
            }),
            document,
        );
        let mut step = stepper(&mut world);
        step(&mut world);
        assert_eq!(old_steps.load(Ordering::SeqCst), 1);

        world.insert(hosted_now(plain_recipe, |_| {}), document);
        step(&mut world);

        assert_eq!(frames(&world, document), 1);
        assert_eq!(old_steps.load(Ordering::SeqCst), 1);
        assert!(hosted_window(&world, document).is_focused());
    }

    #[test]
    fn the_app_on_a_document_leaving_the_front_ends_unfocused() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        step(&mut world);

        world.insert(hosted_now(plain_recipe, |_| {}), document);
        set_active(&mut world, None);
        step(&mut world);

        assert!(!hosted_window(&world, document).is_focused());
    }

    #[test]
    fn losing_the_editor_windows_focus_releases_what_the_hosted_app_held() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        send(
            &mut world,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        press_button(&mut world);
        step(&mut world);
        assert!(hosted_input(&world, document).is_held(KEY));
        assert!(hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));

        send(&mut world, WindowEvent::Focused(false));
        step(&mut world);
        assert!(!hosted_input(&world, document).is_held(KEY));
        assert!(!hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));

        move_pointer_outside_the_viewport(&mut world);
        step(&mut world);
        assert_eq!(
            hosted_input(&world, document).mouse_position(),
            input::POINTER_GONE
        );
    }

    #[derive(Resource, Default)]
    struct Said(u32);

    fn echo_each_frame(mut said: ResMut<Said>, mut echo: EventWriter<Echo>) {
        said.0 += 1;
        echo.write(Echo(said.0));
    }

    #[test]
    fn an_event_written_during_a_step_arrives_as_a_hosted_event() {
        let (mut world, document) = listening_world();
        add_stepping(&mut world);
        world.insert(
            hosted_now(plain_recipe, |app| {
                app.insert_resource(Said::default())
                    .add_system(Update, echo_each_frame.at_edit_time());
            }),
            document,
        );
        let mut step = stepper(&mut world);

        step(&mut world);
        assert_eq!(hear(&mut world), vec![(document, 1)]);

        set_active(&mut world, None);
        step(&mut world);
        assert_eq!(hear(&mut world), vec![(document, 1), (document, 2)]);
    }

    #[test]
    fn an_app_that_becomes_the_target_is_told_it_has_focus() {
        let (mut world, document) = stepping_world();
        let hosted = world
            .get_component_for_entity_mut::<HostedApp>(document)
            .unwrap();
        hosted.push(WindowEvent::Focused(false));
        hosted.step();
        assert!(!hosted_window(&world, document).is_focused());

        run_step(&mut world);

        assert!(hosted_window(&world, document).is_focused());
    }

    #[test]
    fn hiding_the_viewport_releases_what_was_held_with_one_last_step() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        press_button(&mut world);
        step(&mut world);
        assert!(hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));

        world.get_resource_mut::<input::ViewportShown>().unwrap().0 = None;
        step(&mut world);
        let after_hiding = frames(&world, document);
        step(&mut world);

        assert_eq!(after_hiding, 2);
        assert_eq!(frames(&world, document), 2);
        assert!(!hosted_window(&world, document).is_focused());
        assert!(!hosted_input(&world, document).is_mouse_button_held(MouseButton::Right));
    }

    fn ask_for_focus(mut window: ResMut<Window>) {
        window.send(WindowCommand::Focus);
    }

    #[test]
    fn the_window_commands_of_a_hosted_app_are_dropped_after_each_step() {
        let (mut world, document) = stepping_world();
        world.insert(
            hosted_now(plain_recipe, |app| {
                app.add_system(Update, ask_for_focus.at_edit_time());
            }),
            document,
        );
        let mut step = stepper(&mut world);

        step(&mut world);
        let after_a_step = queued_commands(&mut world, document);
        world.get_resource_mut::<input::ViewportShown>().unwrap().0 = None;
        step(&mut world);

        assert_eq!(frames(&world, document), 2);
        assert_eq!(after_a_step, 0);
        assert_eq!(queued_commands(&mut world, document), 0);
    }

    fn queued_commands(world: &mut World, document: Entity) -> usize {
        world
            .get_component_for_entity_mut::<HostedApp>(document)
            .unwrap()
            .app
            .get_resource_mut::<Window>()
            .unwrap()
            .drain_commands()
            .len()
    }

    #[test]
    fn a_pushed_event_is_folded_into_the_hosted_input() {
        let mut hosted = hosted_now(plain_recipe, |_| {});

        hosted.push(WindowEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        });
        hosted.step();

        assert!(
            hosted
                .world()
                .get_resource::<Input>()
                .unwrap()
                .is_mouse_button_held(MouseButton::Left)
        );
    }

    #[test]
    fn what_a_host_triggered_signal_writes_comes_back_as_a_hosted_event_with_its_document() {
        let (mut world, document) = listening_world();

        trigger_on(&mut world, document, Say(7));

        assert_eq!(hear(&mut world), vec![(document, 7)]);
    }

    #[test]
    fn hosted_events_arrive_once_and_in_the_order_they_were_written() {
        let (mut world, document) = listening_world();

        trigger_on(&mut world, document, Say(1));
        trigger_on(&mut world, document, Say(2));
        hear(&mut world);

        assert_eq!(hear(&mut world), vec![(document, 1), (document, 2)]);
    }

    #[test]
    fn an_app_that_is_not_stepped_still_takes_a_signal() {
        let (mut world, document) = listening_world();
        let before = frames(&world, document);

        trigger_on(&mut world, document, Say(3));

        assert_eq!(frames(&world, document), before);
        assert_eq!(hear(&mut world), vec![(document, 3)]);
    }

    #[test]
    fn despawning_the_document_drops_the_app() {
        let (dropped, insert_canary) = canary();
        let mut world = World::new();
        let document = world.spawn(hosted_now(plain_recipe, insert_canary));
        assert!(!dropped.load(Ordering::SeqCst));

        world.despawn(document);

        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn a_pending_app_becomes_hosted_and_runs_its_ready_step_once() {
        let mut world = World::new();
        install_listeners(&mut world);
        listen(&mut world);
        let document = world.spawn(begin(
            request(plain_recipe),
            |_| {},
            |hosted| {
                hosted.trigger(Say(9));
            },
        ));
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);
        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert_eq!(hear(&mut world), vec![(document, 9)]);
        assert!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        let hosted = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert_eq!(hosted.window().unwrap().size(), (320, 240));
    }

    #[test]
    fn a_replacement_keeps_the_old_app_until_the_new_one_is_ready() {
        let (mut world, document) = host_world(plain_recipe);
        let old = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
            .id();
        world.insert(begin(request(plain_recipe), |_| {}, |_| {}), document);
        assert_eq!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .world()
                .id(),
            old
        );
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert_ne!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .world()
                .id(),
            old
        );
    }

    #[test]
    fn the_replaced_app_is_dropped_when_its_replacement_is_swapped_in() {
        let (dropped, insert_canary) = canary();
        let mut world = World::new();
        install_listeners(&mut world);
        let document = world.spawn(hosted_now(plain_recipe, insert_canary));
        world.insert(begin(request(plain_recipe), |_| {}, |_| {}), document);
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, &mut world);
        assert!(!dropped.load(Ordering::SeqCst));

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert!(dropped.load(Ordering::SeqCst));
    }

    fn panicking_recipe(_: &mut App) {
        panic!("the recipe broke");
    }

    #[test]
    fn a_recipe_that_panics_leaves_a_failed_pending_app() {
        let pending = begin(request(panicking_recipe), |_| {}, |_| {});
        assert_eq!(pending.failure(), Some("the recipe broke"));
    }

    struct BreaksOnFinish;

    impl Plugin for BreaksOnFinish {
        fn build(&self, _: &mut App) {}

        fn finish(&self, _: &mut App) {
            panic!("finish broke after {} plugins", 3);
        }
    }

    fn recipe_breaking_on_finish(app: &mut App) {
        plain_recipe(app);
        app.register_plugin(BreaksOnFinish);
    }

    #[test]
    fn a_plugin_that_panics_in_finish_leaves_a_failed_pending_app_that_never_becomes_ready() {
        let mut pending = begin(request(recipe_breaking_on_finish), |_| {}, |_| {});

        assert!(pending.finish_if_ready().is_none());
        assert_eq!(pending.failure(), Some("finish broke after 3 plugins"));
        assert!(pending.finish_if_ready().is_none());
    }

    #[test]
    fn a_ready_step_that_panics_leaves_a_failed_pending_app() {
        let mut pending = begin(
            request(plain_recipe),
            |_| {},
            |_| panic!("the ready step broke"),
        );

        assert!(pending.finish_if_ready().is_none());
        assert_eq!(pending.failure(), Some("the ready step broke"));
    }

    struct BreaksWhenAskedIfReady;

    impl Plugin for BreaksWhenAskedIfReady {
        fn build(&self, _: &mut App) {}

        fn ready(&self, _: &App) -> bool {
            panic!("ready broke");
        }
    }

    fn recipe_breaking_when_asked_if_ready(app: &mut App) {
        plain_recipe(app);
        app.register_plugin(BreaksWhenAskedIfReady);
    }

    #[test]
    fn a_plugin_that_panics_in_ready_leaves_a_failed_pending_app() {
        let mut pending = begin(request(recipe_breaking_when_asked_if_ready), |_| {}, |_| {});

        assert!(pending.finish_if_ready().is_none());
        assert_eq!(pending.failure(), Some("ready broke"));
    }

    fn recipe_panicking_without_a_message(_: &mut App) {
        std::panic::panic_any(7_u32);
    }

    struct BreaksOnFinishWithoutAMessage;

    impl Plugin for BreaksOnFinishWithoutAMessage {
        fn build(&self, _: &mut App) {}

        fn finish(&self, _: &mut App) {
            std::panic::panic_any(7_u32);
        }
    }

    fn recipe_breaking_on_finish_without_a_message(app: &mut App) {
        plain_recipe(app);
        app.register_plugin(BreaksOnFinishWithoutAMessage);
    }

    #[test]
    fn a_panic_without_a_message_is_reported_by_the_step_that_panicked() {
        let pending = begin(request(recipe_panicking_without_a_message), |_| {}, |_| {});
        assert_eq!(pending.failure(), Some("the recipe panicked"));

        let mut pending = begin(
            request(recipe_breaking_on_finish_without_a_message),
            |_| {},
            |_| {},
        );
        pending.finish_if_ready();
        assert_eq!(pending.failure(), Some("finishing the build panicked"));

        let mut pending = begin(
            request(plain_recipe),
            |_| {},
            |_| std::panic::panic_any(7_u32),
        );
        pending.finish_if_ready();
        assert_eq!(pending.failure(), Some("the ready step panicked"));
    }

    #[derive(Resource)]
    struct SiblingFinished(Arc<AtomicBool>);

    fn startup_breaks(_: Res<SiblingFinished>) {
        panic!("startup broke");
    }

    fn startup_sleeps(finished: Res<SiblingFinished>) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        finished.0.store(true, Ordering::SeqCst);
    }

    fn recipe_breaking_in_startup(app: &mut App) {
        plain_recipe(app);
        app.add_system(
            concerto_app::schedule_groups::Startup,
            startup_breaks.at_edit_time(),
        )
        .add_system(
            concerto_app::schedule_groups::Startup,
            startup_sleeps.at_edit_time(),
        );
    }

    #[test]
    fn a_startup_system_that_panics_fails_the_build_after_its_running_sibling_finishes() {
        for _ in 0..5 {
            let finished = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&finished);
            let mut pending = begin(
                request(recipe_breaking_in_startup),
                move |app| {
                    app.insert_resource(SiblingFinished(flag));
                },
                |_| {},
            );

            assert!(pending.finish_if_ready().is_none());

            assert!(finished.load(Ordering::SeqCst));
            assert_eq!(pending.failure(), Some("startup broke"));
        }
    }

    #[test]
    fn a_failed_pending_app_is_left_in_place_and_never_hosted() {
        let mut world = World::new();
        install_listeners(&mut world);
        let document = world.spawn(begin(request(recipe_breaking_on_finish), |_| {}, |_| {}));
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);
        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
        assert_eq!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .unwrap()
                .failure(),
            Some("finish broke after 3 plugins")
        );
    }

    #[test]
    fn a_pending_app_that_is_removed_before_it_is_ready_is_dropped() {
        let (dropped, insert_canary) = canary();
        let mut world = World::new();
        let document = world.spawn(begin(request(plain_recipe), insert_canary, |_| {}));
        assert!(!dropped.load(Ordering::SeqCst));

        world.remove_component::<PendingApp>(document);

        assert!(dropped.load(Ordering::SeqCst));
        assert!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
    }

    fn hosted_id(world: &World, document: Entity) -> concerto_ecs::world_id::WorldId {
        world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
            .id()
    }

    fn parked(world: &World, document: Entity) -> Option<&HostedApp> {
        world
            .get_component_for_entity::<ParkedEditApp>(document)
            .map(|parked| &parked.0)
    }

    fn starting(world: &World, document: Entity) -> bool {
        world
            .get_component_for_entity::<PendingPlayApp>(document)
            .is_some()
    }

    #[test]
    fn the_game_role_builds_a_game_app_and_the_edit_role_an_edit_app() {
        let game = begin(play_request(plain_recipe), |_| {}, |_| {})
            .finish_now()
            .unwrap();
        let edit = hosted_now(plain_recipe, |_| {});

        assert!(!game.app.is_editing());
        assert!(edit.app.is_editing());
    }

    #[test]
    fn a_starting_play_app_is_left_alone_by_the_finishing_of_edit_apps() {
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        start_play(&mut world, document, plain_recipe, |_| {});
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, &mut world);

        concerto_ecs::System::run_and_apply(&mut system, (), &mut world);

        assert_eq!(hosted_id(&world, document), edit);
        assert!(starting(&world, document));
        assert_eq!(play_state(&world, document), Some(PlayState::Starting));
    }

    #[test]
    fn a_ready_play_app_takes_the_place_of_the_edit_app_which_is_parked() {
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);

        play_now(&mut world, document, plain_recipe, |_| {});

        assert_eq!(parked(&world, document).unwrap().world().id(), edit);
        assert_ne!(hosted_id(&world, document), edit);
        assert!(
            !world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .app
                .is_editing()
        );
        assert!(!starting(&world, document));
        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
    }

    #[test]
    fn the_parked_edit_app_is_not_stepped() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        step(&mut world);

        play_now(&mut world, document, plain_recipe, |_| {});
        step(&mut world);
        step(&mut world);

        let parked_frames = parked(&world, document)
            .unwrap()
            .world()
            .get_resource::<Frames>()
            .unwrap()
            .0;
        assert_eq!(parked_frames, 1);
        assert_eq!(frames(&world, document), 2);
    }

    #[test]
    fn stopping_a_playing_document_returns_its_edit_app_and_drops_the_play_app() {
        let (dropped, insert_canary) = canary();
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        play_now(&mut world, document, plain_recipe, insert_canary);
        assert!(!dropped.load(Ordering::SeqCst));
        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
        assert!(parked(&world, document).is_some());

        world.trigger(StopPlay(document));

        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(hosted_id(&world, document), edit);
        assert!(parked(&world, document).is_none());
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
    }

    #[test]
    fn stopping_a_starting_document_drops_the_play_app_and_leaves_the_edit_app() {
        let (dropped, insert_canary) = canary();
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        start_play(&mut world, document, plain_recipe, insert_canary);

        world.trigger(StopPlay(document));

        assert!(dropped.load(Ordering::SeqCst));
        assert!(!starting(&world, document));
        assert_eq!(hosted_id(&world, document), edit);
        assert!(parked(&world, document).is_none());
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
    }

    fn ask_to_close(mut window: ResMut<Window>) {
        window.send(WindowCommand::Close);
    }

    fn closing(app: &mut App) {
        app.add_system(Update, ask_to_close.at_edit_time());
    }

    #[test]
    fn a_play_app_asking_to_close_is_stopped() {
        let (mut world, document) = stepping_world();
        let edit = hosted_id(&world, document);
        let mut step = stepper(&mut world);
        play_now(&mut world, document, plain_recipe, closing);

        step(&mut world);

        assert_eq!(hosted_id(&world, document), edit);
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
    }

    #[derive(Resource, Default)]
    struct Stops(usize);

    fn count_stop(_: On<StopPlay>, mut stops: ResMut<Stops>) {
        stops.0 += 1;
    }

    #[test]
    fn an_edit_app_asking_to_close_is_not_stopped() {
        let (mut world, document) = stepping_world();
        world.insert(hosted_now(plain_recipe, closing), document);
        world.insert_resource(Stops::default());
        world.add_listener(count_stop);
        let mut step = stepper(&mut world);

        step(&mut world);

        assert_eq!(world.get_resource::<Stops>().unwrap().0, 0);
        assert_eq!(queued_commands(&mut world, document), 0);
    }

    #[test]
    fn a_play_app_that_fails_to_build_returns_the_document_to_editing_with_its_message() {
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        start_play(&mut world, document, panicking_recipe, |_| {});

        finish_play(&mut world);

        assert!(!starting(&world, document));
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
        assert_eq!(
            world
                .get_component_for_entity::<PlayFailure>(document)
                .map(|failure| failure.0.as_str()),
            Some("the recipe broke")
        );
        assert_eq!(hosted_id(&world, document), edit);
        assert!(parked(&world, document).is_none());
    }

    #[test]
    fn a_resize_while_playing_reaches_the_play_app() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        play_now(&mut world, document, plain_recipe, |_| {});
        step(&mut world);

        if let Some(view) = &mut world.get_resource_mut::<input::ViewportShown>().unwrap().0 {
            view.size = (800, 600);
        }
        step(&mut world);

        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
        assert!(
            !world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .app
                .is_editing()
        );
        assert_eq!(hosted_window(&world, document).size(), (800, 600));
        assert_eq!(
            parked(&world, document).unwrap().window().unwrap().size(),
            (320, 240)
        );
    }

    #[test]
    fn a_play_app_that_starts_clears_the_failure_of_the_one_before() {
        let (mut world, document) = host_world(plain_recipe);
        start_play(&mut world, document, panicking_recipe, |_| {});
        finish_play(&mut world);
        assert!(
            world
                .get_component_for_entity::<PlayFailure>(document)
                .is_some()
        );

        play_now(&mut world, document, plain_recipe, |_| {});

        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
        assert!(
            world
                .get_component_for_entity::<PlayFailure>(document)
                .is_none()
        );
    }

    fn finish_edit_apps(world: &mut World) {
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, world);
        concerto_ecs::System::run_and_apply(&mut system, (), world);
    }

    #[test]
    fn an_edit_app_that_becomes_ready_while_playing_does_not_replace_the_play_app() {
        let (mut world, document) = host_world(plain_recipe);
        play_now(&mut world, document, plain_recipe, |_| {});
        let play = hosted_id(&world, document);
        world.insert(begin(request(plain_recipe), |_| {}, |_| {}), document);

        finish_edit_apps(&mut world);

        assert_eq!(hosted_id(&world, document), play);
        assert!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .is_some()
        );
    }

    #[test]
    fn an_edit_app_that_becomes_ready_while_play_starts_does_not_replace_the_edit_app() {
        let (mut world, document) = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        start_play(&mut world, document, plain_recipe, |_| {});
        world.insert(begin(request(plain_recipe), |_| {}, |_| {}), document);

        finish_edit_apps(&mut world);

        assert_eq!(hosted_id(&world, document), edit);
        assert!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .is_some()
        );
    }

    fn stop_drops_a_load_that_is_running_in_the_play_app(playing: bool) {
        let _gate = lock_gate();
        let directory = tempfile::tempdir().unwrap();
        let mut registry = concerto_foundation::assets::content::AssetRegistry::new();
        let id = concerto_foundation::assets::content::save_content_asset_into(
            &GatedAsset,
            directory.path(),
            "content/gated.gasset",
            &mut registry,
        )
        .unwrap();
        let mut server = AssetServer::new();
        server.publish_content_source(
            concerto_foundation::assets::ContentAssetRoot::Directory(directory.path().into()),
            registry,
        );
        let store = concerto_foundation::assets::asset_store::AssetStore::<GatedAsset>::new();
        server.register_asset(&store);
        let (mut world, document) = host_world(plain_recipe);
        let play_server = server.clone();
        let extra = move |app: &mut App| {
            app.insert_resource(play_server);
        };
        if playing {
            play_now(&mut world, document, plain_recipe, extra);
        } else {
            start_play(&mut world, document, plain_recipe, extra);
        }
        let dropped_before = GATED_DROPS.load(Ordering::SeqCst);
        let _handle = server.load::<GatedAsset>(id);
        wait_for("the load never started", || ENTERED.load(Ordering::SeqCst));

        world.trigger(StopPlay(document));
        RELEASED.store(true, Ordering::SeqCst);

        wait_for("the load was delivered after Stop", || {
            GATED_DROPS.load(Ordering::SeqCst) > dropped_before
        });
    }

    #[test]
    fn stop_cancels_the_loads_of_the_play_app_while_starting_and_while_playing() {
        stop_drops_a_load_that_is_running_in_the_play_app(false);
        stop_drops_a_load_that_is_running_in_the_play_app(true);
    }

    #[test]
    fn a_key_held_in_the_edit_app_when_play_starts_is_not_held_when_it_returns() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);
        send(
            &mut world,
            WindowEvent::Key {
                key: KEY,
                pressed: true,
                text: None,
            },
        );
        step(&mut world);
        assert!(hosted_input(&world, document).is_held(KEY));

        play_now(&mut world, document, plain_recipe, |_| {});
        step(&mut world);
        world.trigger(StopPlay(document));
        step(&mut world);

        assert!(!hosted_input(&world, document).is_held(KEY));
    }
}
