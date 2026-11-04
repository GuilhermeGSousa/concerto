//! `App`s the editor holds as values: building, stepping, and the messages in and out.
pub mod events;
pub mod input;
pub mod seam;
pub mod target;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use concerto_app::{App, Plugin, plugins::PluginsState, schedule_groups::First};
use concerto_ecs::{
    Component, Entity, Event, IntoSystemConfig, Query, Res, Resource, World,
    command::CommandQueue,
    events::{event_channel::EventChannel, event_writer::EventWriter},
    query::change_detection::DetectChanges,
    reflect::AppTypeRegistry,
    signal::{On, Signal},
    system::{NonSendMarker, executor::multi_thread::MainThreadExecutor},
};
use concerto_foundation::assets::{
    asset_server::AssetServer, asset_store::AssetStore, registry::AssetRegistry,
};
use concerto_render::{
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    render_asset::{RenderAssetBytesPerFrame, render_assets_settled},
    wgpu,
};
use concerto_scene::{scene::Scene, spawner::SceneSpawnerComponent};

use crate::project::ProjectState;
use concerto_window::{
    clipboard::WindowClipboard,
    events::WindowEvent,
    plugin::{Window, WindowCommand},
};

pub use events::{Hosted, HostedAppEvents, HostedEventsAppExt};
pub use seam::{EditWorld, EditWorldSignals};
pub use target::{EditTarget, SetEditTarget, ViewportTarget};

pub const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The bytes of render assets a hosted `App` uploads in one frame, whatever its game sets.
pub const HOSTED_UPLOAD_BYTES_PER_FRAME: usize = 32 * 1024 * 1024;

/// How long an edit or play `App` is stepped hidden, waiting for its loads and uploads to settle, before it is shown anyway.
pub const WARM_UP_LIMIT: Duration = Duration::from_secs(10);

#[derive(Resource)]
pub(crate) struct FinishBuildsOffThread;

/// Whether a host builds an `App` to edit a scene in, to play the game, or to preview an asset in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRole {
    Edit,
    Game,
    Preview,
}

/// What a host supplies when it builds an `App`.
pub struct HostRequest<'a> {
    pub gpu: Option<&'a Gpu>,
    pub recipe: fn(&mut App),
    pub size: (u32, u32),
    pub scale_factor: f64,
    pub role: HostRole,
    pub clipboard: Option<fn(&mut WindowClipboard)>,
}

/// An `App` held on a document. Dropping the component drops the `App`.
#[derive(Component)]
pub struct HostedApp {
    app: App,
    picture: SharedTexture,
    role: HostRole,
}

impl HostedApp {
    /// The role this `App` was built for.
    pub fn role(&self) -> HostRole {
        self.role
    }

    /// The texture this `App` draws its window into.
    pub fn picture(&self) -> &SharedTexture {
        &self.picture
    }

    /// The type registry of this `App`'s world.
    pub fn type_registry(&self) -> Option<AppTypeRegistry> {
        self.world().get_resource::<AppTypeRegistry>().cloned()
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

    pub(crate) fn world(&self) -> &World {
        self.app.main().world()
    }

    #[doc(hidden)]
    pub fn world_for_test(&self) -> &World {
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

    fn settled(&mut self) -> bool {
        let loading = self
            .app
            .get_resource::<AssetServer>()
            .is_some_and(AssetServer::is_loading);
        let world = self.app.main_mut().world_mut();
        let spawners: Vec<_> = world
            .query::<&SceneSpawnerComponent, ()>()
            .iter(world)
            .map(|spawner| spawner.0.clone())
            .collect();
        let spawning = world
            .get_resource::<AssetStore<Scene>>()
            .is_some_and(|scenes| spawners.iter().any(|scene| scenes.get(scene).is_some()));
        !loading && !spawning && render_assets_settled(&self.app)
    }

    fn cancel_loads(&self) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.cancel_pending();
        }
    }

    fn serve_project(&mut self, root: &Path, registry: &AssetRegistry) {
        if let Some(server) = self.app.get_resource::<AssetServer>()
            && let Err(error) = server.publish_project_content(root, registry.clone())
        {
            log::error!("Could not activate project assets: {error:#}");
        }
        self.app.insert_resource(registry.clone());
    }

    fn receive_registry(&mut self, registry: &AssetRegistry) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.publish_registry(registry.clone());
        }
        self.app.insert_resource(registry.clone());
    }
}

type OnReady = Box<dyn FnOnce(&mut HostedApp) + Send + Sync>;

type Finished = Result<Option<HostedApp>, String>;

enum Phase {
    Building(Box<HostedApp>),
    Finishing(JoinHandle<Finished>),
    WarmingUp(Box<WarmingUp>),
    Done,
}

struct WarmingUp {
    hosted: HostedApp,
    since: Instant,
    steps: usize,
}

/// An `App` whose plugins are not all ready yet.
#[derive(Component)]
pub struct PendingApp {
    phase: Phase,
    on_ready: Option<OnReady>,
    started: Instant,
    warm_up_limit: Duration,
    cancelled: Arc<AtomicBool>,
    failure: Option<String>,
}

impl PendingApp {
    pub(crate) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// The panic message of a recipe, plugin step, or ready step that failed; such an `App` never becomes ready.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn building_mut(&mut self) -> Option<&mut HostedApp> {
        match &mut self.phase {
            Phase::Building(hosted) => Some(hosted.as_mut()),
            _ => None,
        }
    }

    fn held(&self) -> Option<&HostedApp> {
        match &self.phase {
            Phase::Building(hosted) => Some(hosted.as_ref()),
            Phase::WarmingUp(warming) => Some(&warming.hosted),
            _ => None,
        }
    }

    fn held_mut(&mut self) -> Option<&mut HostedApp> {
        match &mut self.phase {
            Phase::Building(hosted) => Some(hosted.as_mut()),
            Phase::WarmingUp(warming) => Some(&mut warming.hosted),
            _ => None,
        }
    }

    fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.failure.is_some()
    }

    fn take_ready(&mut self) -> Option<HostedApp> {
        let hosted = self.building_mut()?;
        match catch_unwind(AssertUnwindSafe(|| hosted.app.plugin_state())) {
            Ok(PluginsState::Ready) => match std::mem::replace(&mut self.phase, Phase::Done) {
                Phase::Building(hosted) => Some(*hosted),
                _ => None,
            },
            Ok(_) => None,
            Err(payload) => {
                self.failure = Some(panic_message(payload, "a plugin's ready"));
                None
            }
        }
    }

    fn finish_if_ready(&mut self) -> Option<HostedApp> {
        let hosted = self.finish_here()?;
        log::info!("Hosted App ready in {:?}", self.started.elapsed());
        Some(hosted)
    }

    fn finish_here(&mut self) -> Option<HostedApp> {
        if self.stopped() {
            return None;
        }
        let mut hosted = self.take_ready()?;
        match finish_build(&mut hosted, self.on_ready.take(), &self.cancelled) {
            Ok(true) => Some(hosted),
            Ok(false) => None,
            Err(failure) => {
                self.failure = Some(failure);
                None
            }
        }
    }

    fn finish_off_thread(&mut self, registry: Option<&AssetRegistry>) -> Option<HostedApp> {
        if self.stopped() {
            return None;
        }
        match std::mem::replace(&mut self.phase, Phase::Done) {
            Phase::Finishing(worker) if !worker.is_finished() => {
                self.phase = Phase::Finishing(worker);
                None
            }
            Phase::Finishing(worker) => {
                let hosted = self.join(worker, registry)?;
                self.start_warm_up(hosted)
            }
            Phase::WarmingUp(warming) => self.warm_up(*warming),
            phase => {
                self.phase = phase;
                let hosted = self.take_ready()?;
                self.phase = Phase::Finishing(spawn_finishing(
                    hosted,
                    self.on_ready.take(),
                    Arc::clone(&self.cancelled),
                ));
                None
            }
        }
    }

    fn join(
        &mut self,
        worker: JoinHandle<Finished>,
        registry: Option<&AssetRegistry>,
    ) -> Option<HostedApp> {
        let finished = worker
            .join()
            .unwrap_or_else(|payload| Err(panic_message(payload, "finishing the build")));
        match finished {
            Ok(Some(mut hosted)) => {
                if let Some(registry) = registry {
                    hosted.receive_registry(registry);
                }
                Some(hosted)
            }
            Ok(None) => None,
            Err(failure) => {
                self.failure = Some(failure);
                None
            }
        }
    }

    fn start_warm_up(&mut self, hosted: HostedApp) -> Option<HostedApp> {
        if hosted.role == HostRole::Preview {
            return Some(self.ready(hosted, 0));
        }
        self.phase = Phase::WarmingUp(Box::new(WarmingUp {
            hosted,
            since: Instant::now(),
            steps: 0,
        }));
        None
    }

    fn warm_up(&mut self, mut warming: WarmingUp) -> Option<HostedApp> {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| warming.hosted.step())) {
            self.failure = Some(panic_message(payload, "warming up"));
            return None;
        }
        warming.steps += 1;
        if warming.hosted.settled() || warming.since.elapsed() >= self.warm_up_limit {
            return Some(self.ready(warming.hosted, warming.steps));
        }
        self.phase = Phase::WarmingUp(Box::new(warming));
        None
    }

    fn ready(&self, hosted: HostedApp, hidden_steps: usize) -> HostedApp {
        log::info!(
            "Hosted App ready in {:?} after {hidden_steps} hidden steps",
            self.started.elapsed()
        );
        hosted
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn finish_now(mut self) -> Option<HostedApp> {
        self.finish_if_ready()
    }

    #[cfg(test)]
    pub(crate) fn warm_up_now(&mut self) -> Option<HostedApp> {
        let hosted = self.finish_here()?;
        let mut promoted = self.start_warm_up(hosted);
        while promoted.is_none() && !self.stopped() {
            let Phase::WarmingUp(warming) = std::mem::replace(&mut self.phase, Phase::Done) else {
                return None;
            };
            promoted = self.warm_up(*warming);
        }
        promoted
    }

    #[cfg(test)]
    fn with_warm_up_limit(mut self, limit: Duration) -> Self {
        self.warm_up_limit = limit;
        self
    }

    #[cfg(test)]
    fn warming_up(&self) -> bool {
        matches!(self.phase, Phase::WarmingUp(_))
    }
}

impl Drop for PendingApp {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

fn spawn_finishing(
    mut hosted: HostedApp,
    on_ready: Option<OnReady>,
    cancelled: Arc<AtomicBool>,
) -> JoinHandle<Finished> {
    std::thread::Builder::new()
        .name("hosted-build".into())
        .spawn(move || {
            profiling::register_thread!("hosted-build");
            finish_build(&mut hosted, on_ready, &cancelled).map(|ready| ready.then_some(hosted))
        })
        .expect("the thread that finishes a hosted App's build could not be started")
}

fn finish_build(
    hosted: &mut HostedApp,
    on_ready: Option<OnReady>,
    cancelled: &AtomicBool,
) -> Result<bool, String> {
    let editor_executor = run_non_send_systems_here(&mut hosted.app);
    let built = catch_unwind(AssertUnwindSafe(|| hosted.app.finish_plugin_build()));
    if let Some(executor) = editor_executor {
        hosted.app.insert_resource(executor);
    }
    built.map_err(|payload| panic_message(payload, "finishing the build"))?;
    hosted.app.insert_resource(hosted_upload_budget());
    if cancelled.load(Ordering::SeqCst) {
        hosted.cancel_loads();
        return Ok(false);
    }
    if let Some(on_ready) = on_ready {
        catch_unwind(AssertUnwindSafe(|| on_ready(hosted)))
            .map_err(|payload| panic_message(payload, "the ready step"))?;
    }
    Ok(true)
}

fn run_non_send_systems_here(app: &mut App) -> Option<MainThreadExecutor> {
    let previous = app.remove_resource::<MainThreadExecutor>()?;
    app.insert_resource(MainThreadExecutor::new());
    Some(previous)
}

fn hosted_upload_budget() -> RenderAssetBytesPerFrame {
    RenderAssetBytesPerFrame {
        max_bytes: Some(HOSTED_UPLOAD_BYTES_PER_FRAME),
    }
}

/// Starts building an `App`; `on_ready` runs once on it before it becomes a [`HostedApp`].
pub fn begin(
    request: HostRequest<'_>,
    extra: impl FnOnce(&mut App),
    on_ready: impl FnOnce(&mut HostedApp) + Send + Sync + 'static,
) -> PendingApp {
    let started = Instant::now();
    let picture = SharedTexture::new(PICTURE_FORMAT);
    let mut app = match request.role {
        HostRole::Edit => App::for_editing(),
        HostRole::Game => App::new(),
        HostRole::Preview => App::for_previewing(),
    };
    if let Some(gpu) = request.gpu {
        app.insert_resource(gpu.clone())
            .insert_resource(DrawTarget::Texture(picture.clone()));
    }
    app.insert_resource(Window::new(request.size, request.scale_factor));
    let built = catch_unwind(AssertUnwindSafe(|| {
        (request.recipe)(&mut app);
        extra(&mut app);
        if let Some(open) = request.clipboard
            && let Some(clipboard) = app.get_resource_mut::<WindowClipboard>()
        {
            open(clipboard);
        }
        app.insert_resource(hosted_upload_budget());
    }));
    let (phase, failure) = match built {
        Ok(()) => (
            Phase::Building(Box::new(HostedApp {
                app,
                picture,
                role: request.role,
            })),
            None,
        ),
        Err(payload) => (Phase::Done, Some(panic_message(payload, "the recipe"))),
    };
    PendingApp {
        phase,
        on_ready: Some(Box::new(on_ready)),
        started,
        warm_up_limit: WARM_UP_LIMIT,
        cancelled: Arc::new(AtomicBool::new(false)),
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

/// Gives each `App` a document holds the project's root and registry when it is created, and the registry whenever it changes.
pub(crate) fn forward_registry(
    _: NonSendMarker,
    project: Res<ProjectState>,
    registry: Res<AssetRegistry>,
    pending: Query<&mut PendingApp>,
    starting: Query<&mut PendingPlayApp>,
    hosted: Query<&mut HostedApp>,
    parked: Query<&mut ParkedEditApp>,
) {
    let root = project
        .project
        .as_ref()
        .map(|project| project.root.as_path());
    let deliver = |app: Option<&mut HostedApp>, created: bool| match (app, root) {
        (Some(app), Some(root)) if created => app.serve_project(root, &registry),
        (Some(app), _) if registry.is_changed() => app.receive_registry(&registry),
        _ => {}
    };
    for mut app in pending.iter() {
        let created = app.is_added();
        deliver(app.held_mut(), created);
    }
    for mut app in starting.iter() {
        let created = app.is_added();
        deliver(app.0.held_mut(), created);
    }
    if registry.is_changed() {
        for mut app in hosted.iter() {
            deliver(Some(&mut *app), false);
        }
        for mut app in parked.iter() {
            deliver(Some(&mut app.0), false);
        }
    }
}

fn finish(
    pending: &mut PendingApp,
    off_thread: bool,
    registry: Option<&AssetRegistry>,
) -> Option<HostedApp> {
    if off_thread {
        pending.finish_off_thread(registry)
    } else {
        pending.finish_if_ready()
    }
}

pub(crate) fn finish_pending(
    _: NonSendMarker,
    pending: Query<(Entity, &mut PendingApp)>,
    parked: Query<&ParkedEditApp>,
    starting: Query<&PendingPlayApp>,
    (off_thread, registry): (
        Option<Res<FinishBuildsOffThread>>,
        Option<Res<AssetRegistry>>,
    ),
    mut cmd: CommandQueue,
) {
    for (document, mut pending) in pending.iter() {
        if parked.get_entity(document).is_some() || starting.get_entity(document).is_some() {
            continue;
        }
        if let Some(hosted) = finish(&mut pending, off_thread.is_some(), registry.as_deref()) {
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
        if let Some(hosted) = self.0.held() {
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

impl PlayFailure {
    /// The status a document shows for this failure.
    pub fn status(&self) -> String {
        format!("Could not start the game: {}", self.0)
    }
}

/// Stops the play `App` on a document, starting or playing, and returns to its edit `App`.
pub struct StopPlay(pub Entity);
impl Signal for StopPlay {}

/// A document's game finished building and is playing.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct PlayStarted {
    pub document: Entity,
}

/// A request to play a document's game ended without it playing; `message` says why.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct PlayFailed {
    pub document: Entity,
    pub message: String,
}

/// A document's game, starting or playing, was stopped and its edit `App` is back.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct PlayStopped {
    pub document: Entity,
}

pub(crate) fn finish_pending_play(
    _: NonSendMarker,
    pending: Query<(Entity, &mut PendingPlayApp)>,
    hosted: Query<&mut HostedApp>,
    failures: Query<&PlayFailure>,
    (off_thread, registry): (
        Option<Res<FinishBuildsOffThread>>,
        Option<Res<AssetRegistry>>,
    ),
    mut started: EventWriter<PlayStarted>,
    mut failed: EventWriter<PlayFailed>,
    mut cmd: CommandQueue,
) {
    for (document, mut pending) in pending.iter() {
        let Some(mut slot) = hosted.get_entity(document) else {
            continue;
        };
        if let Some(play) = finish(&mut pending.0, off_thread.is_some(), registry.as_deref()) {
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
            started.write(PlayStarted { document });
        } else if let Some(failure) = pending.failure() {
            let failure = PlayFailure(failure.to_owned());
            failed.write(PlayFailed {
                document,
                message: failure.status(),
            });
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
    mut failed: EventWriter<PlayFailed>,
    mut stops: EventWriter<PlayStopped>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().0;
    let mut stopped = false;
    if let Some(pending) = pending.get_entity(document) {
        pending.cancel_loads();
        cmd.remove::<PendingPlayApp>(document);
        failed.write(PlayFailed {
            document,
            message: "The game was stopped before it started".into(),
        });
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
        stops.write(PlayStopped { document });
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
        app.register_event::<PlayStarted>()
            .register_event::<PlayFailed>()
            .register_event::<PlayStopped>();
        app.insert_resource(input::Routing::default())
            .insert_resource(input::HostCursor::default())
            .insert_resource(input::ViewportShown::default())
            .add_system(
                First,
                (
                    input::route_play_input.before(concerto_window::FoldInput),
                    forward_registry,
                    finish_pending,
                    finish_pending_play,
                    input::sync_in_viewport,
                    input::step_viewport_target,
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
        clipboard::{ClipboardBackend, ClipboardRead, WindowClipboard},
        input::{Input, KeyCode, MouseButton, PhysicalKey},
        plugin::WindowCommand,
    };
    use glam::Vec2;
    use std::sync::{
        Arc, Mutex,
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
        let mut step = concerto_ecs::IntoSystem::into_system(input::step_viewport_target);
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
        assert_ne!(Some(world.change_tick()), written);
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

    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl ClipboardBackend for Recorder {
        fn write(&mut self, text: &str) {
            self.0.lock().unwrap().push(text.to_owned());
        }

        fn request_read(&mut self) {}

        fn poll_read(&mut self) -> Option<ClipboardRead> {
            None
        }
    }

    fn mark_opened(clipboard: &mut WindowClipboard) {
        clipboard.write("opened");
    }

    fn written_while_building(
        request: HostRequest<'static>,
        opener: Option<fn(&mut WindowClipboard)>,
    ) -> Vec<String> {
        let written = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&written);
        let pending = begin(
            HostRequest {
                clipboard: opener,
                ..request
            },
            move |app| {
                let mut clipboard = WindowClipboard::with_backend(Recorder(log));
                clipboard.write("built");
                app.insert_resource(clipboard);
            },
            |_| {},
        );
        assert!(pending.finish_now().is_some());
        written.lock().unwrap().clone()
    }

    #[test]
    fn the_clipboard_opener_is_applied_once_after_the_recipe_for_edit_and_play() {
        for request in [request(plain_recipe), play_request(plain_recipe)] {
            assert_eq!(
                written_while_building(request, Some(mark_opened)),
                ["built", "opened"]
            );
        }
    }

    #[test]
    fn without_a_clipboard_opener_the_clipboard_is_untouched() {
        for request in [request(plain_recipe), play_request(plain_recipe)] {
            assert_eq!(written_while_building(request, None), ["built"]);
        }
    }

    fn windowless_recipe(app: &mut App) {
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::TimePlugin);
    }

    #[test]
    fn a_clipboard_opener_on_an_app_without_a_window_plugin_inserts_no_clipboard() {
        let pending = begin(
            HostRequest {
                clipboard: Some(mark_opened),
                ..request(windowless_recipe)
            },
            |_| {},
            |_| {},
        );

        assert_eq!(pending.failure(), None);
        let hosted = pending.finish_now().unwrap();
        assert!(hosted.app.get_resource::<WindowClipboard>().is_none());
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
        let mut registry = AssetRegistry::new();
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

    fn marked<T: Component>(world: &mut World) -> Vec<Entity> {
        world
            .query::<(Entity, &T), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .collect()
    }

    fn a_preview_in_front_of_a_scene() -> (World, Entity, Entity) {
        let (mut world, scene) = host_world(plain_recipe);
        let preview = world.spawn(preview_now(plain_recipe));
        set_active(&mut world, Some(preview));
        (world, preview, scene)
    }

    fn assert_the_preview_alone_is_targeted(world: &mut World, preview: Entity) {
        assert_eq!(marked::<ViewportTarget>(world), [preview]);
        assert!(marked::<EditTarget>(world).is_empty());
    }

    fn start_play_in_the_background(world: &mut World, document: Entity, recipe: fn(&mut App)) {
        let pending = begin(play_request(recipe), |_| {}, |_| {});
        world.insert(
            (PendingPlayApp::new(pending), PlayState::Starting),
            document,
        );
    }

    #[test]
    fn an_app_finishing_in_the_background_leaves_the_front_documents_markers_alone() {
        let (mut world, preview, scene) = a_preview_in_front_of_a_scene();
        world.insert(begin(request(plain_recipe), |_| {}, |_| {}), scene);

        finish_edit_apps(&mut world);

        assert!(
            world
                .get_component_for_entity::<PendingApp>(scene)
                .is_none()
        );
        assert_the_preview_alone_is_targeted(&mut world, preview);
    }

    #[test]
    fn a_play_app_finishing_in_the_background_leaves_the_front_documents_markers_alone() {
        let (mut world, preview, scene) = a_preview_in_front_of_a_scene();
        start_play_in_the_background(&mut world, scene, plain_recipe);

        finish_play(&mut world);

        assert_eq!(play_state(&world, scene), Some(PlayState::Playing));
        assert_the_preview_alone_is_targeted(&mut world, preview);
    }

    #[test]
    fn a_play_app_failing_in_the_background_leaves_the_front_documents_markers_alone() {
        let (mut world, preview, scene) = a_preview_in_front_of_a_scene();
        start_play_in_the_background(&mut world, scene, panicking_recipe);

        finish_play(&mut world);

        assert!(
            world
                .get_component_for_entity::<PlayFailure>(scene)
                .is_some()
        );
        assert_the_preview_alone_is_targeted(&mut world, preview);
    }

    #[test]
    fn stopping_a_background_document_leaves_the_front_documents_markers_alone() {
        let (mut world, preview, scene) = a_preview_in_front_of_a_scene();
        start_play_in_the_background(&mut world, scene, plain_recipe);
        finish_play(&mut world);

        world.trigger(StopPlay(scene));

        assert_eq!(play_state(&world, scene), Some(PlayState::Editing));
        assert_the_preview_alone_is_targeted(&mut world, preview);
    }

    #[test]
    fn only_the_viewport_target_is_stepped() {
        let (mut world, scene) = stepping_world();
        let mut step = stepper(&mut world);
        step(&mut world);
        let preview = world.spawn(preview_now(plain_recipe));
        set_active(&mut world, Some(preview));
        step(&mut world);
        let scene_frames = frames(&world, scene);

        step(&mut world);
        step(&mut world);

        assert_eq!(frames(&world, scene), scene_frames);
        assert_eq!(frames(&world, preview), 3);
        assert_the_preview_alone_is_targeted(&mut world, preview);
    }

    struct WaitsInFinish(Arc<std::sync::Barrier>);

    impl Plugin for WaitsInFinish {
        fn build(&self, _: &mut App) {}

        fn finish(&self, _: &mut App) {
            self.0.wait();
        }
    }

    fn waiting_in_finish() -> (Arc<std::sync::Barrier>, impl FnOnce(&mut App)) {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let held = Arc::clone(&barrier);
        (barrier, move |app: &mut App| {
            app.register_plugin(WaitsInFinish(held));
        })
    }

    fn off_thread_world() -> World {
        let mut world = World::new();
        install_listeners(&mut world);
        world.insert_resource(FinishBuildsOffThread);
        world
    }

    fn finishing(world: &mut World) -> impl FnMut(&mut World) + use<> {
        let mut system = concerto_ecs::IntoSystem::into_system(finish_pending);
        concerto_ecs::System::initialize(&mut system, world);
        move |world| concerto_ecs::System::run_and_apply(&mut system, (), world)
    }

    fn frames_until(
        world: &mut World,
        mut frame: impl FnMut(&mut World),
        done: impl Fn(&World) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !done(world) {
            assert!(
                std::time::Instant::now() < deadline,
                "the build never finished"
            );
            frame(world);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn is_hosted(document: Entity) -> impl Fn(&World) -> bool {
        move |world| {
            world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
        }
    }

    #[test]
    fn a_plugin_finish_that_blocks_leaves_the_editor_frame_running_until_it_returns() {
        let mut world = off_thread_world();
        let (barrier, wait) = waiting_in_finish();
        let document = world.spawn(begin(request(plain_recipe), wait, |_| {}));
        let mut finish = finishing(&mut world);

        finish(&mut world);
        finish(&mut world);

        assert!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
        assert!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .is_some()
        );
        barrier.wait();
        frames_until(&mut world, finish, is_hosted(document));
        assert!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
    }

    #[test]
    fn a_registry_change_made_while_the_build_finishes_reaches_the_app() {
        let mut world = off_thread_world();
        world.insert_resource(AssetRegistry::default());
        let (barrier, wait) = waiting_in_finish();
        let document = world.spawn(begin(request(plain_recipe), wait, |_| {}));
        let mut finish = finishing(&mut world);
        finish(&mut world);

        let asset = scene_entry("late");
        world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(asset.clone());
        barrier.wait();
        frames_until(&mut world, finish, is_hosted(document));

        let hosted = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert!(
            hosted
                .world()
                .get_resource::<AssetRegistry>()
                .and_then(|registry| registry.get(asset.id))
                .is_some()
        );
    }

    #[test]
    fn a_plugin_that_panics_in_finish_off_the_thread_leaves_a_failed_pending_app() {
        let mut world = off_thread_world();
        let document = world.spawn(begin(request(recipe_breaking_on_finish), |_| {}, |_| {}));
        let finish = finishing(&mut world);

        frames_until(&mut world, finish, |world| {
            world
                .get_component_for_entity::<PendingApp>(document)
                .unwrap()
                .failure()
                .is_some()
        });

        assert_eq!(
            world
                .get_component_for_entity::<PendingApp>(document)
                .unwrap()
                .failure(),
            Some("finish broke after 3 plugins")
        );
        assert!(
            world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
    }

    #[test]
    fn a_play_build_that_panics_in_finish_off_the_thread_fails_to_play() {
        let (mut world, document) = host_world(plain_recipe);
        world.insert_resource(FinishBuildsOffThread);
        start_play(&mut world, document, recipe_breaking_on_finish, |_| {});

        frames_until(&mut world, finish_play, |world| {
            world
                .get_component_for_entity::<PendingPlayApp>(document)
                .is_none()
        });

        let failed: Vec<PlayFailed> = world
            .get_resource::<EventChannel<PlayFailed>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .cloned()
            .collect();
        assert_eq!(
            failed,
            vec![PlayFailed {
                document,
                message: "Could not start the game: finish broke after 3 plugins".into(),
            }]
        );
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
    }

    #[test]
    fn cancelling_while_the_build_finishes_skips_the_ready_step_and_drops_the_app() {
        removing_while_the_build_finishes_skips_the_ready_step(true);
    }

    #[test]
    fn removing_a_pending_app_while_its_build_finishes_skips_the_ready_step_and_drops_the_app() {
        removing_while_the_build_finishes_skips_the_ready_step(false);
    }

    fn removing_while_the_build_finishes_skips_the_ready_step(cancel: bool) {
        let mut world = off_thread_world();
        let (barrier, wait) = waiting_in_finish();
        let (dropped, insert_canary) = canary();
        let readied = Arc::new(AtomicBool::new(false));
        let ready_flag = Arc::clone(&readied);
        let document = world.spawn(begin(
            request(plain_recipe),
            move |app: &mut App| {
                wait(app);
                insert_canary(app);
            },
            move |_| ready_flag.store(true, Ordering::SeqCst),
        ));
        let mut finish = finishing(&mut world);
        finish(&mut world);

        if cancel {
            world
                .get_component_for_entity_mut::<PendingApp>(document)
                .unwrap()
                .cancel();
        }
        world.remove_component::<PendingApp>(document);
        barrier.wait();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !dropped.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "the cancelled App was never dropped"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!readied.load(Ordering::SeqCst));
    }

    #[derive(Resource, Default)]
    struct StartupThread(Option<(std::thread::ThreadId, Option<String>)>);

    fn note_startup_thread(_: NonSendMarker, mut noted: ResMut<StartupThread>) {
        let thread = std::thread::current();
        noted.0 = Some((thread.id(), thread.name().map(str::to_owned)));
    }

    fn recipe_with_a_non_send_startup_system(app: &mut App) {
        plain_recipe(app);
        app.insert_resource(StartupThread::default()).add_system(
            concerto_app::schedule_groups::Startup,
            note_startup_thread.at_edit_time(),
        );
    }

    #[test]
    fn a_non_send_startup_system_runs_on_the_thread_that_finishes_the_build() {
        let mut world = off_thread_world();
        let document = world.spawn(begin(
            request(recipe_with_a_non_send_startup_system),
            |_| {},
            |_| {},
        ));
        let finish = finishing(&mut world);

        frames_until(&mut world, finish, is_hosted(document));

        let (thread, name) = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
            .get_resource::<StartupThread>()
            .unwrap()
            .0
            .clone()
            .expect("Startup ran");
        assert_ne!(thread, std::thread::current().id());
        assert_eq!(name.as_deref(), Some("hosted-build"));
    }

    fn upload_budget(hosted: &HostedApp) -> Option<usize> {
        hosted
            .world()
            .get_resource::<RenderAssetBytesPerFrame>()
            .unwrap()
            .max_bytes
    }

    #[test]
    fn begin_sets_the_hosted_upload_budget_over_the_recipes() {
        let pending = begin(
            request(plain_recipe),
            |app| {
                app.insert_resource(RenderAssetBytesPerFrame { max_bytes: Some(1) });
            },
            |_| {},
        );

        assert_eq!(
            upload_budget(pending.held().unwrap()),
            Some(HOSTED_UPLOAD_BYTES_PER_FRAME)
        );
    }

    struct SetsItsOwnBudgetOnFinish;

    impl Plugin for SetsItsOwnBudgetOnFinish {
        fn build(&self, _: &mut App) {}

        fn finish(&self, app: &mut App) {
            app.insert_resource(RenderAssetBytesPerFrame { max_bytes: Some(2) });
        }
    }

    #[test]
    fn a_budget_a_plugin_sets_in_finish_gives_way_to_the_hosted_one() {
        let hosted = hosted_now(plain_recipe, |app| {
            app.register_plugin(SetsItsOwnBudgetOnFinish);
        });

        assert_eq!(upload_budget(&hosted), Some(HOSTED_UPLOAD_BYTES_PER_FRAME));
    }

    fn hosted_frames(hosted: &HostedApp) -> usize {
        hosted.world().get_resource::<Frames>().unwrap().0
    }

    fn loading_server() -> AssetServer {
        let mut server = AssetServer::new();
        server.register_asset(&concerto_foundation::assets::asset_store::AssetStore::<
            GatedAsset,
        >::new());
        std::mem::forget(server.load::<GatedAsset>(concerto_foundation::assets::AssetId::new()));
        server
    }

    fn never_settling_recipe(app: &mut App) {
        plain_recipe(app);
        app.insert_resource(loading_server());
    }

    fn breaks_each_frame() {
        panic!("the hidden step broke");
    }

    fn recipe_breaking_in_update(app: &mut App) {
        plain_recipe(app);
        app.add_system(Update, breaks_each_frame.at_edit_time());
    }

    fn is_warming_up(document: Entity) -> impl Fn(&World) -> bool {
        move |world| {
            world
                .get_component_for_entity::<PendingPlayApp>(document)
                .is_some_and(|pending| pending.0.warming_up())
        }
    }

    #[test]
    fn a_play_app_still_loading_is_stepped_hidden_until_it_settles() {
        let (mut world, document) = host_world(plain_recipe);
        world.insert_resource(FinishBuildsOffThread);
        let edit = hosted_id(&world, document);
        let server = loading_server();
        let play_server = server.clone();
        start_play(&mut world, document, plain_recipe, move |app| {
            app.insert_resource(play_server);
        });
        frames_until(&mut world, finish_play, is_warming_up(document));
        finish_play(&mut world);
        finish_play(&mut world);

        assert_eq!(hosted_id(&world, document), edit);
        assert!(starting(&world, document));
        assert_eq!(play_state(&world, document), Some(PlayState::Starting));

        server.cancel_pending();
        finish_play(&mut world);

        assert_ne!(hosted_id(&world, document), edit);
        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
        assert!(frames(&world, document) > 1);
    }

    #[test]
    fn an_app_that_never_settles_is_promoted_after_the_warm_up_limit() {
        let limit = std::time::Duration::from_millis(50);
        let mut pending =
            begin(request(never_settling_recipe), |_| {}, |_| {}).with_warm_up_limit(limit);
        let started = std::time::Instant::now();

        let hosted = pending.warm_up_now().unwrap();

        assert!(started.elapsed() >= limit);
        assert!(
            hosted
                .world()
                .get_resource::<AssetServer>()
                .unwrap()
                .is_loading()
        );
        assert!(hosted_frames(&hosted) > 1);
    }

    #[test]
    fn a_settled_app_is_promoted_after_one_hidden_step() {
        let mut pending = begin(request(plain_recipe), |_| {}, |_| {});

        let hosted = pending.warm_up_now().unwrap();

        assert_eq!(hosted_frames(&hosted), 1);
    }

    #[test]
    fn a_hidden_step_that_panics_leaves_a_failed_pending_app() {
        let mut pending = begin(request(recipe_breaking_in_update), |_| {}, |_| {});

        assert!(pending.warm_up_now().is_none());

        assert_eq!(pending.failure(), Some("the hidden step broke"));
    }

    #[test]
    fn a_play_app_that_panics_in_a_hidden_step_fails_to_play() {
        let (mut world, document) = host_world(plain_recipe);
        world.insert_resource(FinishBuildsOffThread);
        let edit = hosted_id(&world, document);
        start_play(&mut world, document, recipe_breaking_in_update, |_| {});

        frames_until(&mut world, finish_play, |world| {
            world
                .get_component_for_entity::<PendingPlayApp>(document)
                .is_none()
        });

        let failed: Vec<PlayFailed> = world
            .get_resource::<EventChannel<PlayFailed>>()
            .unwrap()
            .read_from(&mut concerto_ecs::events::event_cursor::EventCursor::default())
            .cloned()
            .collect();
        assert_eq!(
            failed,
            vec![PlayFailed {
                document,
                message: "Could not start the game: the hidden step broke".into(),
            }]
        );
        assert_eq!(play_state(&world, document), Some(PlayState::Editing));
        assert_eq!(hosted_id(&world, document), edit);
    }

    #[test]
    fn a_preview_is_promoted_without_a_warm_up() {
        let mut pending = begin(preview_request(never_settling_recipe), |_| {}, |_| {});

        let hosted = pending.warm_up_now().unwrap();

        assert_eq!(hosted_frames(&hosted), 0);
    }

    fn with_a_spawner(stored: bool) -> impl FnOnce(&mut App) {
        move |app: &mut App| {
            let id = concerto_foundation::assets::AssetId::new();
            let mut scenes = concerto_foundation::assets::asset_store::AssetStore::<Scene>::new();
            if stored {
                scenes.insert(
                    id,
                    Scene {
                        nodes: vec![],
                        referenced_assets: vec![],
                    },
                );
            }
            app.insert_resource(scenes);
            app.main_mut().world_mut().spawn(SceneSpawnerComponent(
                concerto_foundation::assets::handle::AssetHandle::weak(id),
            ));
        }
    }

    #[test]
    fn an_app_with_a_spawner_whose_scene_is_stored_is_not_settled() {
        let mut hosted = hosted_now(plain_recipe, with_a_spawner(true));

        hosted.step();

        assert!(!hosted.settled());
    }

    #[test]
    fn a_spawner_whose_scene_is_neither_stored_nor_loading_does_not_hold_the_warm_up() {
        let mut pending = begin(request(plain_recipe), with_a_spawner(false), |_| {});

        let hosted = pending.warm_up_now().unwrap();

        assert_eq!(hosted_frames(&hosted), 1);
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Upload;

    impl concerto_foundation::assets::Asset for Upload {
        fn name() -> &'static str {
            "Upload"
        }
    }

    struct Uploaded;

    impl concerto_render::render_asset::RenderAsset for Uploaded {
        type SourceAsset = Upload;
        type PreparationParams = ();

        fn prepare_asset(
            _: &Upload,
            _: &mut concerto_ecs::system::input::SystemInputData<()>,
        ) -> Result<Self, concerto_render::render_asset::AssetPreparationError> {
            Ok(Uploaded)
        }
    }

    fn uploading(app: &mut App) {
        let mut uploads = concerto_foundation::assets::asset_store::AssetStore::<Upload>::new();
        uploads.insert(concerto_foundation::assets::AssetId::new(), Upload);
        app.set_extract_fn(concerto_app::extractor::extract);
        app.render_mut().insert_resource(
            concerto_render::render_asset::RenderAssetBytesPerFrameLimiter::default(),
        );
        app.register_plugin(concerto_render::render_asset::RenderAssetPlugin::<Uploaded>::new())
            .insert_resource(uploads);
    }

    #[test]
    fn an_app_whose_last_frame_prepared_a_render_asset_is_not_settled() {
        let mut hosted = hosted_now(plain_recipe, uploading);

        hosted.step();

        assert!(!hosted.settled());
    }
}
