use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use concerto_ecs::Resource;

/// The binary a reload restarts into, and the session it restores.
#[derive(Clone, Debug, PartialEq)]
pub struct Restart {
    /// The binary of the confirming build.
    pub executable: PathBuf,
    /// The directory the session was written to.
    pub session_dir: PathBuf,
}

/// Where a reload leaves its restart for `run` to carry out once the window has closed.
#[derive(Resource, Clone, Debug, Default)]
pub struct RestartSlot(Arc<Mutex<Option<Restart>>>);

impl RestartSlot {
    pub(crate) fn fill(&self, restart: Restart) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(restart);
    }

    /// Takes the restart a reload left, if any.
    pub fn take(&self) -> Option<Restart> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

const RESTORE_SESSION: &str = "--restore-session";

pub(crate) fn restart_arguments(original: &[OsString], session_dir: &Path) -> Vec<OsString> {
    let mut arguments = Vec::with_capacity(original.len() + 2);
    let mut original = original.iter();
    while let Some(argument) = original.next() {
        if argument == RESTORE_SESSION {
            original.next();
        } else {
            arguments.push(argument.clone());
        }
    }
    arguments.push(RESTORE_SESSION.into());
    arguments.push(session_dir.as_os_str().to_owned());
    arguments
}

fn command_line(executable: &Path, arguments: &[OsString]) -> String {
    std::iter::once(executable.as_os_str())
        .chain(arguments.iter().map(OsString::as_os_str))
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn replace_process(restart: Restart, original: &[OsString]) -> ! {
    let arguments = restart_arguments(original, &restart.session_dir);
    let command = command_line(&restart.executable, &arguments);
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::process::CommandExt};
        log::info!(
            "Restarting into {}; session saved in {}. If the editor does not come back, run: {command}",
            restart.executable.display(),
            restart.session_dir.display()
        );
        log::logger().flush();
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        let error = std::process::Command::new(&restart.executable)
            .args(&arguments)
            .exec();
        eprintln!("Could not restart the editor: {error}\nTo restore the session, run: {command}");
    }
    #[cfg(not(unix))]
    eprintln!(
        "Restarting the editor is not supported on this platform\nTo restore the session, run: {command}"
    );
    std::process::exit(1)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use super::{Restart, RestartSlot, restart_arguments};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn restart_arguments_keep_the_original_ones_and_name_the_new_session() {
        assert_eq!(
            restart_arguments(&args(&["--project", "x", "--decorated"]), Path::new("d")),
            args(&["--project", "x", "--decorated", "--restore-session", "d"])
        );
    }

    #[test]
    fn restart_arguments_replace_an_earlier_session() {
        assert_eq!(
            restart_arguments(
                &args(&["--restore-session", "old", "--project", "x"]),
                Path::new("new")
            ),
            args(&["--project", "x", "--restore-session", "new"])
        );
    }

    #[test]
    fn a_restart_slot_is_taken_once_from_any_of_its_clones() {
        let slot = RestartSlot::default();
        let kept = slot.clone();
        let restart = Restart {
            executable: "/work/target/debug/game-editor".into(),
            session_dir: "/work/target/concerto-editor/sessions/game-editor-1".into(),
        };

        slot.fill(restart.clone());

        assert_eq!(kept.take(), Some(restart));
        assert_eq!(slot.take(), None);
    }
}
