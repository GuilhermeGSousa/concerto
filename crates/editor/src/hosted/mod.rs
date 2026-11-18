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
    AssetId, asset_server::AssetServer, derived::DerivedDataCache, registry::AssetRegistry,
};
use concerto_render::{
    draw_target::{DrawTarget, SharedTexture},
    gpu::Gpu,
    render_asset::{RenderAssetBytesPerFrame, render_assets_settled},
    wgpu,
};
use concerto_scene::{
    link::{SceneLink, SceneLinkState},
    scene::Scene,
};

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

/// Replaces `WARM_UP_LIMIT` for the edit and play `App`s this editor starts.
#[doc(hidden)]
#[derive(Resource, Clone, Copy)]
pub struct WarmUpLimit(pub Duration);

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
        let spawning = world
            .get_resource::<AssetServer>()
            .cloned()
            .is_some_and(|server| {
                world
                    .query::<(&SceneLink, &SceneLinkState), ()>()
                    .iter(world)
                    .any(|(link, state)| {
                        *state == SceneLinkState::Waiting
                            && server.is_loading_or_loaded(link.scene().id())
                    })
            });
        !loading && !spawning && render_assets_settled(&self.app)
    }

    fn cancel_loads(&self) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.cancel_pending();
        }
    }

    fn serve_project(&mut self, root: &Path, registry: &AssetRegistry) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            match server.publish_project_content(root, registry.clone()) {
                Ok(()) => server.set_derived_cache(DerivedDataCache::in_project(root)),
                Err(error) => log::error!("Could not activate project assets: {error:#}"),
            }
        }
        self.app.insert_resource(registry.clone());
    }

    fn receive_registry(&mut self, registry: &AssetRegistry) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.publish_registry(registry.clone());
        }
        self.app.insert_resource(registry.clone());
    }

    pub(crate) fn reload_files(&self, files: &[AssetId], registry: &AssetRegistry) {
        if let Some(server) = self.app.get_resource::<AssetServer>() {
            server.publish_registry(registry.clone());
            for &file in files {
                server.reload_file(file);
            }
        }
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

    pub(crate) fn held(&self) -> Option<&HostedApp> {
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

    pub(crate) fn with_warm_up_limit(mut self, limit: Duration) -> Self {
        self.warm_up_limit = limit;
        self
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
    use concerto_ecs::{ResMut, Resource};
    use concerto_window::{
        input::{Input, KeyCode, MouseButton, PhysicalKey},
        plugin::WindowCommand,
    };
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

    fn stepping_world() -> (World, Entity) {
        let HostWorld {
            mut world,
            document,
        } = host_world(plain_recipe);
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
    fn the_edit_target_is_stepped_at_the_viewports_size_and_the_editors_scale() {
        let (mut world, document) = stepping_world();
        let mut step = stepper(&mut world);

        step(&mut world);
        step(&mut world);
        assert_eq!(frames(&world, document), 2);
        assert_eq!(hosted_window(&world, document).size(), (640, 480));
        assert_eq!(hosted_window(&world, document).scale_factor(), 1.0);

        world.insert_resource(Window::new((1280, 720), 2.0));
        send(&mut world, WindowEvent::ScaleFactorChanged(2.0));
        step(&mut world);
        assert_eq!(hosted_window(&world, document).scale_factor(), 2.0);
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
        let HostWorld {
            mut world,
            document,
        } = host_world(plain_recipe);
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

    #[test]
    fn play_parks_the_edit_app_behind_a_game_app_and_stop_brings_it_back_and_drops_the_game() {
        let (dropped, insert_canary) = canary();
        let HostWorld {
            mut world,
            document,
        } = host_world(plain_recipe);
        let edit = hosted_id(&world, document);
        play_now(&mut world, document, plain_recipe, insert_canary);
        assert!(!dropped.load(Ordering::SeqCst));
        assert_eq!(play_state(&world, document), Some(PlayState::Playing));
        assert_eq!(parked(&world, document).unwrap().world().id(), edit);
        assert!(
            !world
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
                .app
                .is_editing()
        );

        world.trigger(StopPlay(document));

        assert!(dropped.load(Ordering::SeqCst));
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

    struct WaitsInFinish(Arc<std::sync::Barrier>);

    impl Plugin for WaitsInFinish {
        fn build(&self, _: &mut App) {}

        fn finish(&self, _: &mut App) {
            self.0.wait();
        }
    }

    fn waiting_in_finish(barrier: &Arc<std::sync::Barrier>) -> impl FnOnce(&mut App) + use<> {
        let held = Arc::clone(barrier);
        move |app: &mut App| {
            app.register_plugin(WaitsInFinish(held));
        }
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
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let wait = waiting_in_finish(&barrier);
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

    #[derive(Resource)]
    struct HiddenSteps(Arc<AtomicUsize>);

    fn count_hidden_step(steps: concerto_ecs::Res<HiddenSteps>) {
        steps.0.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn a_play_app_still_loading_is_stepped_hidden_until_it_settles() {
        let HostWorld {
            mut world,
            document,
        } = host_world(plain_recipe);
        world.insert_resource(FinishBuildsOffThread);
        let edit = hosted_id(&world, document);
        let server = loading_server();
        let play_server = server.clone();
        let steps = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&steps);
        start_play(&mut world, document, plain_recipe, move |app| {
            app.insert_resource(play_server)
                .insert_resource(HiddenSteps(counted))
                .add_system(Update, count_hidden_step);
        });
        frames_until(&mut world, finish_play, |world| {
            assert_eq!(play_state(world, document), Some(PlayState::Starting));
            steps.load(Ordering::SeqCst) >= 3
        });
        assert_eq!(hosted_id(&world, document), edit);

        server.cancel_pending();
        frames_until(&mut world, finish_play, |world| {
            play_state(world, document) == Some(PlayState::Playing)
        });

        assert_ne!(hosted_id(&world, document), edit);
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
}
