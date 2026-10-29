//! Reloading game code: building the game's editor crate again from inside the editor, then restarting into it with the session.
mod cargo;
mod restart;
mod session;
#[cfg(test)]
mod tests;

pub use cargo::{BuildDiagnostic, ReloadTarget};
pub(crate) use restart::replace_process;
pub use restart::{Restart, RestartSlot};
pub use session::{
    DocumentSession, SESSION_FORMAT, SceneSession, Session, SessionRestore, WindowLayout,
};
pub(crate) use session::{finish_restore, restore_documents};

use std::{path::PathBuf, time::SystemTime};

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Entity, Event, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    events::{event_reader::EventReader, event_writer::EventWriter},
    signal::{On, Signal},
};
use concerto_foundation::assets::Asset;
use concerto_scene::scene::Scene;
use concerto_window::{
    input::actions::ActionFired,
    plugin::{Window, WindowCommand},
};

use crate::{
    asset_editor::{EditorDocument, Unsaved},
    guard::UnsavedGuard,
    hosted::{PendingApp, PlayState, StopPlay},
    import::{ImportPicker, ImportQueue},
    inspector::{ComponentEdits, PropertyCommits},
    project::ProjectState,
};
use cargo::{BuildJob, BuildOutcome};

const NO_PROJECT: &str = "Open a project before reloading";
const CANCELLED: &str = "Reload cancelled";
const SESSION_FAILED: &str = "Reload failed: could not save the session";

/// What the editor knows at startup about reloading its own crate and restoring a session.
#[derive(Clone, Debug)]
pub struct ReloadSetup {
    /// The build that reloads the editor, or why it cannot reload.
    pub target: Result<ReloadTarget, String>,
    /// Where a reload leaves the restart for `run` once the window has closed.
    pub restart: RestartSlot,
    /// The session `--restore-session` named, to restore once its project opens.
    pub restore: Option<SessionRestore>,
    /// Why the session `--restore-session` named could not be read.
    pub restore_error: Option<String>,
}

impl ReloadSetup {
    /// A setup that refuses every reload with `reason` and restores nothing.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            target: Err(reason.into()),
            restart: RestartSlot::default(),
            restore: None,
            restore_error: None,
        }
    }
}

/// Builds the game's editor crate on `ReloadGameCode`, restarts into it with the session, and restores a session at startup.
pub struct ReloadPlugin(pub ReloadSetup);

impl Plugin for ReloadPlugin {
    fn build(&self, app: &mut App) {
        let mut reload = GameReload::new(self.0.target.clone());
        if let Some(error) = &self.0.restore_error {
            reload.show(error.clone(), true);
        }
        app.insert_resource(reload)
            .insert_resource(self.0.restart.clone());
        if let Some(restore) = &self.0.restore {
            app.insert_resource(restore.clone());
        }
        app.register_event::<ReloadFailed>()
            .add_system(Update, reload_on_shortcut)
            .add_system(Update, update_reload_build)
            .add_system(Update, restart_when_ready)
            .add_system(Update, restore_documents)
            .add_system(Update, finish_restore);
        install_listeners(app.main_mut().world_mut());
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(start_reload);
    world.add_listener(cancel_reload);
    world.add_listener(save_session);
    session::install_listeners(world);
}

enum ReloadPhase {
    Idle,
    Building(Box<BuildJob>),
    Waiting { executable: PathBuf },
    Confirming(Box<BuildJob>),
    Closing,
    Failed,
}

/// Where a reload of game code is, and what Chatter says about it.
#[derive(Resource)]
pub struct GameReload {
    phase: ReloadPhase,
    target: Result<ReloadTarget, String>,
    status: Option<(String, bool)>,
    started_at: Option<SystemTime>,
    #[cfg(test)]
    fake_builds: std::collections::VecDeque<BuildJob>,
}

impl GameReload {
    pub(crate) fn new(target: Result<ReloadTarget, String>) -> Self {
        Self {
            phase: ReloadPhase::Idle,
            target,
            status: None,
            started_at: None,
            #[cfg(test)]
            fake_builds: std::collections::VecDeque::new(),
        }
    }

    /// What Chatter shows about reloading, and whether it is a warning.
    pub fn status(&self) -> Option<(String, bool)> {
        self.status.clone()
    }

    fn show(&mut self, text: impl Into<String>, warning: bool) {
        let status = Some((text.into(), warning));
        if self.status != status {
            self.status = status;
        }
    }

    fn fail(&mut self, message: String, failed: &mut EventWriter<ReloadFailed>) {
        self.show(message.clone(), true);
        failed.write(ReloadFailed { message });
    }

    fn spawn_build(&mut self) -> Result<BuildJob, String> {
        #[cfg(test)]
        if let Some(job) = self.fake_builds.pop_front() {
            return Ok(job);
        }
        let target = self.target.as_ref().map_err(String::clone)?;
        BuildJob::spawn(target)
            .map_err(|error| format!("Build failed: could not run cargo: {error}"))
    }

    fn build_failed(
        &mut self,
        errors: Vec<BuildDiagnostic>,
        failed: &mut EventWriter<ReloadFailed>,
    ) {
        for error in &errors {
            log::error!("{}", error.rendered);
        }
        let count = errors.len();
        self.phase = ReloadPhase::Failed;
        self.fail(
            format!(
                "Build failed: {count} {}",
                if count == 1 { "error" } else { "errors" }
            ),
            failed,
        );
    }
}

/// Builds the game's editor crate again, unless Chatter gives a reason not to.
pub struct ReloadGameCode;
impl Signal for ReloadGameCode {}

/// Stops a reload that is building, waiting for in-flight work, or confirming its build.
pub struct CancelReload;
impl Signal for CancelReload {}

/// A reload ended without restarting the editor; `message` is what Chatter says.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ReloadFailed {
    pub message: String,
}

struct SaveSession {
    executable: PathBuf,
}
impl Signal for SaveSession {}

fn start_reload(
    _: On<ReloadGameCode>,
    mut reload: ResMut<GameReload>,
    project: Option<Res<ProjectState>>,
    mut failed: EventWriter<ReloadFailed>,
) {
    if let Err(message) = &reload.target {
        let message = message.clone();
        reload.fail(message, &mut failed);
        return;
    }
    if project.as_ref().is_none_or(|state| state.project.is_none()) {
        reload.fail(NO_PROJECT.into(), &mut failed);
        return;
    }
    if !matches!(reload.phase, ReloadPhase::Idle | ReloadPhase::Failed) {
        return;
    }
    match reload.spawn_build() {
        Ok(job) => {
            let progress = job.progress();
            reload.phase = ReloadPhase::Building(Box::new(job));
            reload.started_at = Some(SystemTime::now());
            reload.show(progress, false);
        }
        Err(message) => {
            reload.phase = ReloadPhase::Failed;
            reload.fail(message, &mut failed);
        }
    }
}

fn cancel_reload(
    _: On<CancelReload>,
    mut reload: ResMut<GameReload>,
    mut failed: EventWriter<ReloadFailed>,
) {
    if !matches!(
        reload.phase,
        ReloadPhase::Building(_) | ReloadPhase::Waiting { .. } | ReloadPhase::Confirming(_)
    ) {
        return;
    }
    if let ReloadPhase::Building(mut job) | ReloadPhase::Confirming(mut job) =
        std::mem::replace(&mut reload.phase, ReloadPhase::Idle)
    {
        job.stop();
    }
    reload.show(CANCELLED, false);
    failed.write(ReloadFailed {
        message: CANCELLED.into(),
    });
}

fn reload_on_shortcut(mut fired: EventReader<ActionFired>, mut cmd: CommandQueue) {
    let pressed = fired
        .read()
        .filter(|action| action.is(crate::actions::ReloadGameCode))
        .count()
        > 0;
    if pressed {
        cmd.trigger(ReloadGameCode);
    }
}

pub(crate) fn update_reload_build(
    mut reload: ResMut<GameReload>,
    mut failed: EventWriter<ReloadFailed>,
) {
    let ReloadPhase::Building(job) = &mut reload.phase else {
        return;
    };
    let Some(outcome) = job.poll() else {
        let progress = job.progress();
        reload.show(progress, false);
        return;
    };
    match outcome {
        BuildOutcome::Built { executable } => {
            log::info!("Game code built into {}", executable.display());
            reload.phase = ReloadPhase::Waiting { executable };
            reload.show("Game code built", false);
        }
        BuildOutcome::Failed { errors } => reload.build_failed(errors, &mut failed),
    }
}

enum Readiness {
    StopGames(Vec<Entity>),
    Wait(&'static str),
    Refuse(String),
    Ready,
}

type Documents<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static EditorDocument,
        Option<&'static PlayState>,
        Option<&'static Unsaved>,
        Option<&'static PendingApp>,
    ),
>;

type InFlight<'w> = (
    Res<'w, UnsavedGuard>,
    Res<'w, ProjectState>,
    Res<'w, ImportQueue>,
    Res<'w, ImportPicker>,
);

type QueuedEdits<'w> = (Res<'w, PropertyCommits>, Res<'w, ComponentEdits>);

fn readiness(documents: &Documents, in_flight: &InFlight, edits: &QueuedEdits) -> Readiness {
    let (guard, project, imports, picker) = in_flight;
    let (commits, component_edits) = edits;
    let games: Vec<Entity> = documents
        .iter()
        .filter(|(_, _, state, ..)| state.is_some_and(|state| state.is_on()))
        .map(|(document, ..)| document)
        .collect();
    if !games.is_empty() {
        return Readiness::StopGames(games);
    }
    let loading = documents.iter().any(|(_, document, _, _, pending)| {
        document.asset_type == Scene::name() && (document.pending.is_some() || pending.is_some())
    });
    let reason = if guard.asking() {
        Some("the question about unsaved changes")
    } else if project.busy() {
        Some("the project to open")
    } else if imports.is_running() || imports.remaining() > 0 {
        Some("imports to finish")
    } else if picker.is_picking() {
        Some("the file picker to close")
    } else if !commits.0.is_empty() || !component_edits.0.is_empty() {
        Some("edits to apply")
    } else if loading {
        Some("the scene to load")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Readiness::Wait(reason);
    }
    if project.project.is_none() {
        return Readiness::Refuse("Reload failed: no project is open".into());
    }
    if let Some((_, document, ..)) = documents.iter().find(|(_, document, _, unsaved, _)| {
        unsaved.is_some() && document.asset_type != Scene::name()
    }) {
        return Readiness::Refuse(format!(
            "Reload failed: save {} first; the reload can only save scenes",
            document.title
        ));
    }
    Readiness::Ready
}

pub(crate) fn restart_when_ready(
    mut reload: ResMut<GameReload>,
    mut failed: EventWriter<ReloadFailed>,
    documents: Documents,
    in_flight: InFlight,
    edits: QueuedEdits,
    mut cmd: CommandQueue,
) {
    match &mut reload.phase {
        ReloadPhase::Waiting { executable } => match readiness(&documents, &in_flight, &edits) {
            Readiness::StopGames(games) => {
                for document in games {
                    cmd.trigger(StopPlay(document));
                }
                reload.show("Built · waiting for the game to stop to restart", false);
            }
            Readiness::Wait(reason) => {
                reload.show(format!("Built · waiting for {reason} to restart"), false);
            }
            Readiness::Refuse(message) => {
                reload.phase = ReloadPhase::Idle;
                reload.fail(message, &mut failed);
            }
            Readiness::Ready => {
                log::info!(
                    "Confirming the build of {} before restarting",
                    executable.display()
                );
                match reload.spawn_build() {
                    Ok(job) => {
                        let progress = job.progress();
                        reload.phase = ReloadPhase::Confirming(Box::new(job));
                        reload.show(progress, false);
                    }
                    Err(message) => {
                        reload.phase = ReloadPhase::Failed;
                        reload.fail(message, &mut failed);
                    }
                }
            }
        },
        ReloadPhase::Confirming(job) => match job.poll() {
            None => {
                let progress = job.progress();
                reload.show(progress, false);
            }
            Some(BuildOutcome::Failed { errors }) => reload.build_failed(errors, &mut failed),
            Some(BuildOutcome::Built { executable }) => {
                match readiness(&documents, &in_flight, &edits) {
                    Readiness::Ready => {
                        cmd.trigger(SaveSession { executable });
                        reload.phase = ReloadPhase::Closing;
                        reload.show("Restarting", false);
                    }
                    Readiness::Refuse(message) => {
                        reload.phase = ReloadPhase::Idle;
                        reload.fail(message, &mut failed);
                    }
                    Readiness::StopGames(_) | Readiness::Wait(_) => {
                        reload.phase = ReloadPhase::Waiting { executable };
                    }
                }
            }
        },
        _ => {}
    }
}

fn save_session(on: On<SaveSession>, world: &mut World) {
    let executable = on.signal().executable.clone();
    if let Err(message) = save_and_close(world, executable) {
        if let Some(reload) = world.get_resource_mut::<GameReload>() {
            reload.phase = ReloadPhase::Idle;
            reload.show(message.clone(), true);
        }
        crate::tools::scene::write(world, ReloadFailed { message });
    }
}

fn save_and_close(world: &mut World, executable: PathBuf) -> Result<(), String> {
    let mut scenes: Vec<(u64, Entity, String)> = world
        .query::<(Entity, &EditorDocument), ()>()
        .iter(world)
        .filter(|(_, document)| document.asset_type == Scene::name())
        .map(|(entity, document)| (document.order, entity, document.title.clone()))
        .collect();
    scenes.sort_by_key(|(order, ..)| *order);
    for (_, document, title) in scenes {
        if crate::scene::has_unsaved_edits(world, document) {
            crate::scene::save(world, document)
                .map_err(|error| format!("Reload failed: could not save {title}: {error}"))?;
        }
    }
    let session = session::capture(world).map_err(|error| format!("{SESSION_FAILED}: {error}"))?;
    let dir = world
        .get_resource::<GameReload>()
        .and_then(|reload| reload.target.as_ref().ok())
        .map(|target| target.session_dir(SystemTime::now()))
        .ok_or_else(|| format!("{SESSION_FAILED}: the editor has no build to restart into"))?;
    session
        .write(&dir)
        .map_err(|error| format!("{SESSION_FAILED}: {error:#}"))?;
    if !executable.is_file() {
        return Err(format!(
            "Reload failed: the built editor {} is missing",
            executable.display()
        ));
    }
    world
        .get_resource::<RestartSlot>()
        .ok_or_else(|| "Reload failed: the editor has nowhere to restart from".to_string())?
        .fill(Restart {
            executable,
            session_dir: dir,
        });
    if let Some(window) = world.get_resource_mut::<Window>() {
        window.send(WindowCommand::Close);
    }
    Ok(())
}
