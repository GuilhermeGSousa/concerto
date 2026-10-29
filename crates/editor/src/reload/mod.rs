//! Reloading game code: building the game's editor crate again from inside the editor.
mod cargo;
#[cfg(test)]
mod tests;

pub use cargo::{BuildDiagnostic, ReloadTarget};

use concerto_app::{App, Plugin, schedule_groups::Update};
use concerto_ecs::{
    Event, Res, ResMut, Resource, World,
    command::CommandQueue,
    events::{event_reader::EventReader, event_writer::EventWriter},
    signal::{On, Signal},
};
use concerto_window::input::actions::ActionFired;

use crate::project::ProjectState;
use cargo::{BuildJob, BuildOutcome};

const NO_PROJECT: &str = "Open a project before reloading";
const BUILT: &str = "Game code built";
const CANCELLED: &str = "Reload cancelled";

/// What the editor knows at startup about reloading its own crate.
#[derive(Clone, Debug)]
pub struct ReloadSetup {
    /// The build that reloads the editor, or why it cannot reload.
    pub target: Result<ReloadTarget, String>,
}

/// Builds the game's editor crate on `ReloadGameCode` and reports the build in Chatter.
pub struct ReloadPlugin(pub ReloadSetup);

impl Plugin for ReloadPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GameReload::new(self.0.target.clone()))
            .register_event::<ReloadFailed>()
            .add_system(Update, reload_on_shortcut)
            .add_system(Update, update_reload_build);
        install_listeners(app.main_mut().world_mut());
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(start_reload);
    world.add_listener(cancel_reload);
}

enum ReloadPhase {
    Idle,
    Building(Box<BuildJob>),
    Failed,
}

/// Where a reload of game code is, and what Chatter says about it.
#[derive(Resource)]
pub struct GameReload {
    phase: ReloadPhase,
    target: Result<ReloadTarget, String>,
    status: Option<(String, bool)>,
}

impl GameReload {
    pub(crate) fn new(target: Result<ReloadTarget, String>) -> Self {
        Self {
            phase: ReloadPhase::Idle,
            target,
            status: None,
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
}

/// Builds the game's editor crate again, unless Chatter gives a reason not to.
pub struct ReloadGameCode;
impl Signal for ReloadGameCode {}

/// Stops the build of the game's editor crate.
pub struct CancelReload;
impl Signal for CancelReload {}

/// A reload ended without restarting the editor; `message` is what Chatter says.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct ReloadFailed {
    pub message: String,
}

fn start_reload(
    _: On<ReloadGameCode>,
    mut reload: ResMut<GameReload>,
    project: Option<Res<ProjectState>>,
    mut failed: EventWriter<ReloadFailed>,
) {
    let target = match &reload.target {
        Err(message) => {
            let message = message.clone();
            reload.fail(message, &mut failed);
            return;
        }
        Ok(target) => target.clone(),
    };
    if project.as_ref().is_none_or(|state| state.project.is_none()) {
        reload.fail(NO_PROJECT.into(), &mut failed);
        return;
    }
    if matches!(reload.phase, ReloadPhase::Building(_)) {
        return;
    }
    match BuildJob::spawn(&target) {
        Ok(job) => {
            let progress = job.progress();
            reload.phase = ReloadPhase::Building(Box::new(job));
            reload.show(progress, false);
        }
        Err(error) => {
            reload.phase = ReloadPhase::Failed;
            reload.fail(
                format!("Build failed: could not run cargo: {error}"),
                &mut failed,
            );
        }
    }
}

fn cancel_reload(
    _: On<CancelReload>,
    mut reload: ResMut<GameReload>,
    mut failed: EventWriter<ReloadFailed>,
) {
    if !matches!(reload.phase, ReloadPhase::Building(_)) {
        return;
    }
    if let ReloadPhase::Building(mut job) = std::mem::replace(&mut reload.phase, ReloadPhase::Idle)
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
            reload.phase = ReloadPhase::Idle;
            reload.show(BUILT, false);
        }
        BuildOutcome::Failed { errors } => {
            for error in &errors {
                log::error!("{}", error.rendered);
            }
            let count = errors.len();
            reload.phase = ReloadPhase::Failed;
            reload.fail(
                format!(
                    "Build failed: {count} {}",
                    if count == 1 { "error" } else { "errors" }
                ),
                &mut failed,
            );
        }
    }
}
