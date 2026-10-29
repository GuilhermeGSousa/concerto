use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, PoisonError, mpsc},
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::EditorCrate;

#[cfg(unix)]
const OUTSIDE_TARGET_DIR: &str =
    "Reload needs the editor to run from its cargo target directory (target/<profile>/)";
#[cfg(unix)]
const TARGET_TRIPLE: &str = "Reload does not support an editor built with --target";
#[cfg(not(unix))]
const UNSUPPORTED: &str = "Reloading game code is not supported on this platform yet";
const NO_BINARY: &str = "cargo did not report the editor binary";
const STDERR_TAIL: usize = 40;
#[cfg(unix)]
const GRACE: Duration = Duration::from_secs(2);

/// Where and how the running editor's crate is built again.
#[derive(Clone, Debug, PartialEq)]
pub struct ReloadTarget {
    cargo: PathBuf,
    manifest_path: PathBuf,
    package: String,
    bin: String,
    profile: String,
    target_dir: PathBuf,
}

impl ReloadTarget {
    /// Finds the profile and target directory `exe` was built into, or says why the editor cannot reload.
    pub fn locate(editor: &EditorCrate, exe: &Path) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = (editor, exe);
            Err(UNSUPPORTED.to_string())
        }
        #[cfg(unix)]
        {
            let profile_dir = exe
                .parent()
                .filter(|dir| dir.join(".cargo-lock").is_file())
                .ok_or(OUTSIDE_TARGET_DIR)?;
            let target_dir = profile_dir
                .parent()
                .filter(|dir| dir.join("CACHEDIR.TAG").is_file())
                .ok_or(OUTSIDE_TARGET_DIR)?;
            if target_dir
                .parent()
                .is_some_and(|dir| dir.join("CACHEDIR.TAG").is_file())
            {
                return Err(TARGET_TRIPLE.to_string());
            }
            let profile = profile_dir
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(OUTSIDE_TARGET_DIR)?;
            let cargo = Path::new(editor.cargo);
            Ok(Self {
                cargo: if cargo.is_file() {
                    cargo.to_path_buf()
                } else {
                    PathBuf::from("cargo")
                },
                manifest_path: Path::new(editor.manifest_dir).join("Cargo.toml"),
                package: editor.package.to_owned(),
                bin: editor.bin.to_owned(),
                profile: if profile == "debug" { "dev" } else { profile }.to_owned(),
                target_dir: target_dir.to_path_buf(),
            })
        }
    }

    /// The `cargo build` that builds the editor binary again, in a process group of its own.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.cargo);
        command
            .arg("build")
            .arg("--manifest-path")
            .arg(&self.manifest_path)
            .arg("--package")
            .arg(&self.package)
            .arg("--bin")
            .arg(&self.bin)
            .arg("--profile")
            .arg(&self.profile)
            .arg("--target-dir")
            .arg(&self.target_dir)
            .arg("--message-format=json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        cargo: &Path,
        manifest_dir: &Path,
        bin: &str,
        target_dir: &Path,
    ) -> Self {
        Self {
            cargo: cargo.to_path_buf(),
            manifest_path: manifest_dir.join("Cargo.toml"),
            package: bin.to_owned(),
            bin: bin.to_owned(),
            profile: "dev".to_owned(),
            target_dir: target_dir.to_path_buf(),
        }
    }
}

/// One compiler error of a build: its message, its primary location, and the text rustc prints.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildDiagnostic {
    /// The error's one-line message.
    pub message: String,
    /// The file, line and column of the error's primary span.
    pub location: Option<(String, u32, u32)>,
    /// The error as rustc prints it, location and snippet included.
    pub rendered: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CargoEvent {
    Compiled {
        target_name: String,
        fresh: bool,
        executable: Option<PathBuf>,
    },
    Error(BuildDiagnostic),
    Warning,
    Finished {
        success: bool,
    },
}

pub(crate) fn parse_line(line: &str) -> Option<CargoEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value.get("reason")?.as_str()? {
        "compiler-artifact" => Some(CargoEvent::Compiled {
            target_name: value.pointer("/target/name")?.as_str()?.to_owned(),
            fresh: value.get("fresh").and_then(Value::as_bool).unwrap_or(false),
            executable: value
                .get("executable")
                .and_then(Value::as_str)
                .map(PathBuf::from),
        }),
        "compiler-message" => compiler_message(value.get("message")?),
        "build-finished" => Some(CargoEvent::Finished {
            success: value.get("success")?.as_bool()?,
        }),
        _ => None,
    }
}

fn compiler_message(message: &Value) -> Option<CargoEvent> {
    if !message.get("level")?.as_str()?.starts_with("error") {
        return Some(CargoEvent::Warning);
    }
    let text = message.get("message")?.as_str()?.to_owned();
    let spans = message
        .get("spans")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if spans.is_empty() && text.starts_with("aborting due to") {
        return None;
    }
    let location = spans
        .iter()
        .find(|span| span.get("is_primary").and_then(Value::as_bool) == Some(true))
        .and_then(span_location);
    let rendered = message
        .get("rendered")
        .and_then(Value::as_str)
        .map_or_else(|| text.clone(), str::to_owned);
    Some(CargoEvent::Error(BuildDiagnostic {
        message: text,
        location,
        rendered,
    }))
}

fn span_location(span: &Value) -> Option<(String, u32, u32)> {
    let number = |key: &str| u32::try_from(span.get(key)?.as_u64()?).ok();
    Some((
        span.get("file_name")?.as_str()?.to_owned(),
        number("line_start")?,
        number("column_start")?,
    ))
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BuildOutcome {
    Built { executable: PathBuf },
    Failed { errors: Vec<BuildDiagnostic> },
}

#[derive(Default)]
pub(crate) struct BuildReport {
    compiled: usize,
    last_compiled: Option<String>,
    executables: Vec<(String, PathBuf)>,
    errors: Vec<BuildDiagnostic>,
    warnings: usize,
    finished: Option<bool>,
}

impl BuildReport {
    pub(crate) fn record(&mut self, event: CargoEvent) {
        match event {
            CargoEvent::Compiled {
                target_name,
                fresh,
                executable,
            } => {
                if let Some(executable) = executable {
                    self.executables.push((target_name.clone(), executable));
                }
                if !fresh {
                    self.compiled += 1;
                    self.last_compiled = Some(target_name);
                }
            }
            CargoEvent::Error(error) => self.errors.push(error),
            CargoEvent::Warning => self.warnings += 1,
            CargoEvent::Finished { success } => self.finished = Some(success),
        }
    }

    pub(crate) fn progress(&self, last_stderr: Option<&str>, elapsed: Duration) -> String {
        let seconds = elapsed.as_secs();
        match (&self.last_compiled, last_stderr.map(str::trim)) {
            (Some(last), _) => format!(
                "Building game code · {} compiled · {last} · {seconds} s",
                plural(self.compiled, "crate", "crates")
            ),
            (None, Some(line)) if !line.is_empty() => {
                format!("Building game code · {line} · {seconds} s")
            }
            (None, _) => format!("Building game code · {seconds} s"),
        }
    }

    pub(crate) fn outcome(self, success: bool, stderr_tail: &[String], bin: &str) -> BuildOutcome {
        log::info!(
            "The build ended with {}",
            plural(self.warnings, "warning", "warnings")
        );
        if success && self.finished != Some(false) {
            return match self
                .executables
                .into_iter()
                .rev()
                .find(|(name, _)| name == bin)
            {
                Some((_, executable)) => BuildOutcome::Built { executable },
                None => BuildOutcome::Failed {
                    errors: vec![BuildDiagnostic {
                        message: NO_BINARY.into(),
                        location: None,
                        rendered: NO_BINARY.into(),
                    }],
                },
            };
        }
        if !self.errors.is_empty() {
            return BuildOutcome::Failed {
                errors: self.errors,
            };
        }
        let message = stderr_tail
            .iter()
            .find(|line| line.starts_with("error"))
            .or(stderr_tail.last())
            .cloned()
            .unwrap_or_else(|| "cargo failed without saying why".into());
        BuildOutcome::Failed {
            errors: vec![BuildDiagnostic {
                rendered: if stderr_tail.is_empty() {
                    message.clone()
                } else {
                    stderr_tail.join("\n")
                },
                message,
                location: None,
            }],
        }
    }
}

pub(crate) enum BuildUpdate {
    Cargo(CargoEvent),
    Stderr(String),
}

pub(crate) struct BuildJob {
    child: Child,
    updates: Mutex<mpsc::Receiver<BuildUpdate>>,
    started: Instant,
    bin: String,
    report: BuildReport,
    stderr: VecDeque<String>,
    exited: Option<bool>,
}

impl BuildJob {
    pub(crate) fn spawn(target: &ReloadTarget) -> std::io::Result<Self> {
        let mut child = target.command().spawn()?;
        let (sender, updates) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            let sender = sender.clone();
            std::thread::spawn(move || {
                read_lines(stdout, |line| {
                    parse_line(&line)
                        .is_none_or(|event| sender.send(BuildUpdate::Cargo(event)).is_ok())
                });
            });
        }
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                read_lines(stderr, |line| {
                    log::info!("{line}");
                    sender.send(BuildUpdate::Stderr(line)).is_ok()
                });
            });
        }
        Ok(Self::new(child, updates, &target.bin))
    }

    #[cfg(test)]
    pub(crate) fn for_test(updates: mpsc::Receiver<BuildUpdate>, bin: &str) -> Self {
        let mut command = Command::new("true");
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        Self::new(command.spawn().unwrap(), updates, bin)
    }

    fn new(child: Child, updates: mpsc::Receiver<BuildUpdate>, bin: &str) -> Self {
        Self {
            child,
            updates: Mutex::new(updates),
            started: Instant::now(),
            bin: bin.to_owned(),
            report: BuildReport::default(),
            stderr: VecDeque::new(),
            exited: None,
        }
    }

    pub(crate) fn poll(&mut self) -> Option<BuildOutcome> {
        let updates = self
            .updates
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        let mut readers_done = false;
        loop {
            match updates.try_recv() {
                Ok(BuildUpdate::Cargo(event)) => self.report.record(event),
                Ok(BuildUpdate::Stderr(line)) => {
                    if self.stderr.len() == STDERR_TAIL {
                        self.stderr.pop_front();
                    }
                    self.stderr.push_back(line);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    readers_done = true;
                    break;
                }
            }
        }
        if self.exited.is_none() {
            self.exited = match self.child.try_wait() {
                Ok(status) => status.map(|status| status.success()),
                Err(_) => Some(false),
            };
        }
        let success = self.exited?;
        if !readers_done {
            return None;
        }
        let report = std::mem::take(&mut self.report);
        Some(report.outcome(success, self.stderr.make_contiguous(), &self.bin))
    }

    pub(crate) fn progress(&self) -> String {
        self.report.progress(
            self.stderr.back().map(String::as_str),
            self.started.elapsed(),
        )
    }

    pub(crate) fn stop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(None)) {
            return;
        }
        #[cfg(unix)]
        {
            signal_group(&self.child, libc::SIGTERM);
            let deadline = Instant::now() + GRACE;
            while Instant::now() < deadline {
                if !matches!(self.child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            signal_group(&self.child, libc::SIGKILL);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn signal_group(child: &Child, signal: libc::c_int) {
    let Ok(group) = libc::pid_t::try_from(child.id()) else {
        return;
    };
    // SAFETY: killpg only sends a signal, to the process group the build was spawned to lead, whose leader this job has not yet reaped.
    unsafe {
        libc::killpg(group, signal);
    }
}

fn read_lines(pipe: impl Read, mut send: impl FnMut(String) -> bool) {
    for line in BufReader::new(pipe).split(b'\n') {
        let Ok(line) = line else {
            return;
        };
        let line = String::from_utf8_lossy(&line)
            .trim_end_matches('\r')
            .to_owned();
        if !send(line) {
            return;
        }
    }
}

impl Drop for BuildJob {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{ffi::OsStr, time::Instant};
    use std::{
        path::{Path, PathBuf},
        time::Duration,
    };

    #[cfg(unix)]
    use concerto_ecs::{World, events::event_channel::EventChannel};

    use super::*;
    #[cfg(unix)]
    use crate::reload::{CancelReload, GameReload, ReloadFailed, ReloadPhase};

    const BIN_ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/work/target/debug/fixture"],"executable":"/work/target/debug/fixture","fresh":false}"#;
    const FRESH_BIN_ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/work/target/debug/fixture"],"executable":"/work/target/debug/fixture","fresh":true}"#;
    const LIB_ARTIFACT: &str = r#"{"reason":"compiler-artifact","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"fixture","src_path":"/work/fixture/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["/work/target/debug/libfixture.rlib","/work/target/debug/deps/libfixture-043c61fdbf990c12.rmeta"],"executable":null,"fresh":false}"#;
    const BUILD_SCRIPT_EXECUTED: &str = r#"{"reason":"build-script-executed","package_id":"path+file:///work/fixture#0.1.0","linked_libs":[],"linked_paths":[],"cfgs":[],"env":[],"out_dir":"/work/target/debug/build/fixture-89dfb3ec26e1962d/out"}"#;
    const MOVED_VALUE_ERROR: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"error[E0382]: borrow of moved value: `text`\n --> src/main.rs:5:16\n  |\n3 |     let text = String::new();\n  |         ---- move occurs because `text` has type `String`, which does not implement the `Copy` trait\n4 |     let moved = text;\n  |                 ---- value moved here\n5 |     println!(\"{text} {moved}\");\n  |                ^^^^ value borrowed here after move\n  |\nhelp: consider cloning the value if the performance cost is acceptable\n  |\n4 |     let moved = text.clone();\n  |                     ++++++++\n\n","$message_type":"diagnostic","children":[{"children":[],"code":null,"level":"help","message":"consider cloning the value if the performance cost is acceptable","rendered":null,"spans":[{"byte_end":82,"byte_start":82,"column_end":21,"column_start":21,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":4,"line_start":4,"suggested_replacement":".clone()","suggestion_applicability":"MachineApplicable","text":[{"highlight_end":21,"highlight_start":21,"text":"    let moved = text;"}]}]}],"level":"error","message":"borrow of moved value: `text`","spans":[{"byte_end":82,"byte_start":78,"column_end":21,"column_start":17,"expansion":null,"file_name":"src/main.rs","is_primary":false,"label":"value moved here","line_end":4,"line_start":4,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":21,"highlight_start":17,"text":"    let moved = text;"}]},{"byte_end":103,"byte_start":99,"column_end":20,"column_start":16,"expansion":{"def_site_span":{"byte_end":0,"byte_start":0,"column_end":1,"column_start":1,"expansion":null,"file_name":"src/main.rs","is_primary":false,"label":null,"line_end":1,"line_start":1,"suggested_replacement":null,"suggestion_applicability":null,"text":[]},"macro_decl_name":"desugaring of format string literal","span":{"byte_end":114,"byte_start":88,"column_end":31,"column_start":5,"expansion":null,"file_name":"src/main.rs","is_primary":false,"label":null,"line_end":5,"line_start":5,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":31,"highlight_start":5,"text":"    println!(\"{text} {moved}\");"}]}},"file_name":"src/main.rs","is_primary":true,"label":"value borrowed here after move","line_end":5,"line_start":5,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":20,"highlight_start":16,"text":"    println!(\"{text} {moved}\");"}]},{"byte_end":44,"byte_start":40,"column_end":13,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":false,"label":"move occurs because `text` has type `String`, which does not implement the `Copy` trait","line_end":3,"line_start":3,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":13,"highlight_start":9,"text":"    let text = String::new();"}]}],"code":{"code":"E0382","explanation":"A variable was used after its contents have been moved elsewhere.\n"}}}"#;
    const MISSING_X_ERROR: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"error[E0425]: cannot find value `x` in this scope\n --> src/main.rs:2:5\n  |\n2 |     x;\n  |     ^ not found in this scope\n\n","$message_type":"diagnostic","children":[],"level":"error","message":"cannot find value `x` in this scope","spans":[{"byte_end":17,"byte_start":16,"column_end":6,"column_start":5,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":"not found in this scope","line_end":2,"line_start":2,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":6,"highlight_start":5,"text":"    x;"}]}],"code":{"code":"E0425","explanation":"An unresolved name was used.\n"}}}"#;
    const MISSING_Y_ERROR: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"error[E0425]: cannot find value `y` in this scope\n --> src/main.rs:3:5\n  |\n3 |     y;\n  |     ^ not found in this scope\n\n","$message_type":"diagnostic","children":[],"level":"error","message":"cannot find value `y` in this scope","spans":[{"byte_end":24,"byte_start":23,"column_end":6,"column_start":5,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":"not found in this scope","line_end":3,"line_start":3,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":6,"highlight_start":5,"text":"    y;"}]}],"code":{"code":"E0425","explanation":"An unresolved name was used.\n"}}}"#;
    const UNUSED_WARNING: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"warning: unused variable: `unused`\n --> src/main.rs:2:9\n  |\n2 |     let unused = 1;\n  |         ^^^^^^ help: if this is intentional, prefix it with an underscore: `_unused`\n  |\n  = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default\n\n","$message_type":"diagnostic","children":[{"children":[],"code":null,"level":"note","message":"`#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default","rendered":null,"spans":[]},{"children":[],"code":null,"level":"help","message":"if this is intentional, prefix it with an underscore","rendered":null,"spans":[{"byte_end":26,"byte_start":20,"column_end":15,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":2,"line_start":2,"suggested_replacement":"_unused","suggestion_applicability":"MachineApplicable","text":[{"highlight_end":15,"highlight_start":9,"text":"    let unused = 1;"}]}]}],"level":"warning","message":"unused variable: `unused`","spans":[{"byte_end":26,"byte_start":20,"column_end":15,"column_start":9,"expansion":null,"file_name":"src/main.rs","is_primary":true,"label":null,"line_end":2,"line_start":2,"suggested_replacement":null,"suggestion_applicability":null,"text":[{"highlight_end":15,"highlight_start":9,"text":"    let unused = 1;"}]}],"code":{"code":"unused_variables","explanation":null}}}"#;
    const FAILURE_NOTE: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"For more information about this error, try `rustc --explain E0425`.\n","$message_type":"diagnostic","children":[],"level":"failure-note","message":"For more information about this error, try `rustc --explain E0425`.","spans":[],"code":null}}"#;
    const ABORTING_SUMMARY: &str = r#"{"reason":"compiler-message","package_id":"path+file:///work/fixture#0.1.0","manifest_path":"/work/fixture/Cargo.toml","target":{"kind":["bin"],"crate_types":["bin"],"name":"fixture","src_path":"/work/fixture/src/main.rs","edition":"2021","doc":true,"doctest":false,"test":true},"message":{"rendered":"error: aborting due to 2 previous errors\n\n","$message_type":"diagnostic","children":[],"level":"error","message":"aborting due to 2 previous errors","spans":[],"code":null}}"#;
    const FINISHED_OK: &str = r#"{"reason":"build-finished","success":true}"#;
    const FINISHED_FAILED: &str = r#"{"reason":"build-finished","success":false}"#;

    fn editor(manifest_dir: &'static str, cargo: &'static str) -> EditorCrate {
        EditorCrate {
            manifest_dir,
            package: "my-game-editor",
            bin: "my-game-editor",
            cargo,
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn built_into(target: &Path, profile_dir: &str) -> PathBuf {
        touch(&target.join("CACHEDIR.TAG"));
        touch(&target.join(profile_dir).join(".cargo-lock"));
        let exe = target.join(profile_dir).join("my-game-editor");
        touch(&exe);
        exe
    }

    fn parsed(line: &str) -> CargoEvent {
        parse_line(line).expect("the line is an event the editor keeps")
    }

    fn report(lines: &[&str]) -> BuildReport {
        let mut report = BuildReport::default();
        for line in lines {
            if let Some(event) = parse_line(line) {
                report.record(event);
            }
        }
        report
    }

    fn errors(outcome: BuildOutcome) -> Vec<BuildDiagnostic> {
        match outcome {
            BuildOutcome::Failed { errors } => errors,
            BuildOutcome::Built { executable } => {
                panic!("the build gave {}", executable.display())
            }
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn reloading_is_refused_on_a_platform_that_is_not_unix() {
        let directory = tempfile::tempdir().unwrap();
        let exe = built_into(&directory.path().join("target"), "debug");

        assert_eq!(
            ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe),
            Err("Reloading game code is not supported on this platform yet".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_debug_binary_builds_the_dev_profile_into_its_target_dir() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let exe = built_into(&target, "debug");

        let located =
            ReloadTarget::locate(&editor("/games/my-game/editor", env!("CARGO")), &exe).unwrap();

        assert_eq!(
            located,
            ReloadTarget {
                cargo: PathBuf::from(env!("CARGO")),
                manifest_path: PathBuf::from("/games/my-game/editor/Cargo.toml"),
                package: "my-game-editor".into(),
                bin: "my-game-editor".into(),
                profile: "dev".into(),
                target_dir: target,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_release_binary_builds_the_release_profile() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let exe = built_into(&target, "release");

        let located = ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe).unwrap();

        assert_eq!(located.profile, "release");
        assert_eq!(located.target_dir, target);
    }

    #[cfg(unix)]
    #[test]
    fn a_custom_profile_builds_under_its_own_name() {
        let directory = tempfile::tempdir().unwrap();
        let exe = built_into(&directory.path().join("target"), "profiling");

        let located = ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe).unwrap();

        assert_eq!(located.profile, "profiling");
    }

    #[cfg(unix)]
    #[test]
    fn a_cargo_that_is_gone_is_replaced_by_the_one_on_the_path() {
        let directory = tempfile::tempdir().unwrap();
        let exe = built_into(&directory.path().join("target"), "debug");

        let located = ReloadTarget::locate(&editor("/editor", "/no/such/cargo"), &exe).unwrap();

        assert_eq!(located.cargo, PathBuf::from("cargo"));
    }

    #[cfg(unix)]
    #[test]
    fn an_editor_built_with_a_target_triple_cannot_reload() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        touch(&target.join("CACHEDIR.TAG"));
        let exe = built_into(&target.join("x86_64-unknown-linux-gnu"), "debug");

        assert_eq!(
            ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe),
            Err("Reload does not support an editor built with --target".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_outside_a_cargo_target_dir_cannot_reload() {
        let refusal = Err(
            "Reload needs the editor to run from its cargo target directory (target/<profile>/)"
                .to_string(),
        );
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let exe = target.join("debug").join("my-game-editor");
        touch(&exe);
        touch(&target.join("CACHEDIR.TAG"));
        assert_eq!(
            ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe),
            refusal
        );

        std::fs::remove_file(target.join("CACHEDIR.TAG")).unwrap();
        touch(&target.join("debug").join(".cargo-lock"));
        assert_eq!(
            ReloadTarget::locate(&editor("/editor", env!("CARGO")), &exe),
            refusal
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_build_command_names_the_manifest_package_bin_profile_and_target_dir() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let exe = built_into(&target, "release");
        let located =
            ReloadTarget::locate(&editor("/games/my-game/editor", env!("CARGO")), &exe).unwrap();

        let command = located.command();

        assert_eq!(command.get_program(), OsStr::new(env!("CARGO")));
        let target = target.to_str().unwrap();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "build",
                "--manifest-path",
                "/games/my-game/editor/Cargo.toml",
                "--package",
                "my-game-editor",
                "--bin",
                "my-game-editor",
                "--profile",
                "release",
                "--target-dir",
                target,
                "--message-format=json",
            ]
            .map(OsStr::new)
        );
    }

    #[test]
    fn the_editor_binary_artifact_gives_the_executable() {
        assert_eq!(
            parsed(BIN_ARTIFACT),
            CargoEvent::Compiled {
                target_name: "fixture".into(),
                fresh: false,
                executable: Some(PathBuf::from("/work/target/debug/fixture")),
            }
        );
    }

    #[test]
    fn a_library_artifact_counts_as_progress_without_an_executable() {
        assert_eq!(
            parsed(LIB_ARTIFACT),
            CargoEvent::Compiled {
                target_name: "fixture".into(),
                fresh: false,
                executable: None,
            }
        );
        assert_eq!(
            report(&[LIB_ARTIFACT]).progress(None, Duration::from_secs(8)),
            "Building game code · 1 crate compiled · fixture · 8 s"
        );
        assert_eq!(
            report(&[LIB_ARTIFACT, LIB_ARTIFACT]).progress(Some("ignored"), Duration::ZERO),
            "Building game code · 2 crates compiled · fixture · 0 s"
        );
    }

    #[test]
    fn a_fresh_artifact_is_not_counted() {
        let fresh = report(&[FRESH_BIN_ARTIFACT]);

        assert_eq!(
            fresh.progress(None, Duration::from_secs(3)),
            "Building game code · 3 s"
        );
        assert_eq!(
            fresh.progress(
                Some("    Blocking waiting for file lock on build directory"),
                Duration::from_secs(3)
            ),
            "Building game code · Blocking waiting for file lock on build directory · 3 s"
        );
        assert_eq!(
            report(&[FRESH_BIN_ARTIFACT, FINISHED_OK]).outcome(true, &[], "fixture"),
            BuildOutcome::Built {
                executable: PathBuf::from("/work/target/debug/fixture")
            }
        );
    }

    #[test]
    fn a_compiler_error_keeps_its_rendered_text_and_primary_location() {
        let CargoEvent::Error(error) = parsed(MOVED_VALUE_ERROR) else {
            panic!("a compiler error is an error");
        };

        assert_eq!(error.message, "borrow of moved value: `text`");
        assert_eq!(error.location, Some(("src/main.rs".to_string(), 5, 16)));
        assert!(
            error.rendered.starts_with(
                "error[E0382]: borrow of moved value: `text`\n --> src/main.rs:5:16\n"
            ),
            "{}",
            error.rendered
        );
    }

    #[test]
    fn warnings_and_the_aborting_summary_are_not_listed() {
        assert_eq!(parsed(UNUSED_WARNING), CargoEvent::Warning);
        assert_eq!(parsed(FAILURE_NOTE), CargoEvent::Warning);
        assert_eq!(parse_line(ABORTING_SUMMARY), None);

        let listed = errors(
            report(&[
                UNUSED_WARNING,
                MISSING_X_ERROR,
                FAILURE_NOTE,
                ABORTING_SUMMARY,
                FINISHED_FAILED,
            ])
            .outcome(false, &[], "fixture"),
        );

        assert_eq!(
            listed
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>(),
            ["cannot find value `x` in this scope"]
        );
    }

    #[test]
    fn build_finished_reports_success_and_failure() {
        assert_eq!(parsed(FINISHED_OK), CargoEvent::Finished { success: true });
        assert_eq!(
            parsed(FINISHED_FAILED),
            CargoEvent::Finished { success: false }
        );
    }

    #[test]
    fn a_line_that_is_not_json_and_unknown_reasons_are_skipped() {
        assert_eq!(parse_line(BUILD_SCRIPT_EXECUTED), None);
        assert_eq!(
            parse_line("   Compiling fixture v0.1.0 (/work/fixture)"),
            None
        );
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line(r#"{"success":true}"#), None);
    }

    #[test]
    fn a_failed_build_lists_its_compiler_errors_in_order() {
        let listed = errors(
            report(&[
                LIB_ARTIFACT,
                MISSING_X_ERROR,
                MOVED_VALUE_ERROR,
                MISSING_Y_ERROR,
                FINISHED_FAILED,
            ])
            .outcome(
                false,
                &["error: could not compile `fixture`".into()],
                "fixture",
            ),
        );

        assert_eq!(
            listed
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>(),
            [
                "cannot find value `x` in this scope",
                "borrow of moved value: `text`",
                "cannot find value `y` in this scope",
            ]
        );
    }

    #[test]
    fn a_failed_build_without_compiler_errors_shows_the_end_of_stderr() {
        let tail = [
            "error: failed to parse manifest at `/work/fixture/Cargo.toml`".to_string(),
            String::new(),
            "Caused by:".to_string(),
            "  missing field `name`".to_string(),
        ];

        assert_eq!(
            errors(report(&[]).outcome(false, &tail, "fixture")),
            [BuildDiagnostic {
                message: "error: failed to parse manifest at `/work/fixture/Cargo.toml`".into(),
                location: None,
                rendered: tail.join("\n"),
            }]
        );
    }

    #[test]
    fn a_successful_build_that_never_names_the_binary_fails() {
        let listed = errors(report(&[LIB_ARTIFACT, FINISHED_OK]).outcome(true, &[], "fixture"));

        assert_eq!(
            listed,
            [BuildDiagnostic {
                message: "cargo did not report the editor binary".into(),
                location: None,
                rendered: "cargo did not report the editor binary".into(),
            }]
        );
        assert!(matches!(
            report(&[BIN_ARTIFACT, FINISHED_OK]).outcome(true, &[], "another-bin"),
            BuildOutcome::Failed { .. }
        ));
    }

    #[test]
    fn a_successful_build_gives_the_executable() {
        assert_eq!(
            report(&[LIB_ARTIFACT, BIN_ARTIFACT, UNUSED_WARNING, FINISHED_OK]).outcome(
                true,
                &[],
                "fixture"
            ),
            BuildOutcome::Built {
                executable: PathBuf::from("/work/target/debug/fixture")
            }
        );
    }

    #[cfg(unix)]
    struct Fixture {
        directory: tempfile::TempDir,
        target: ReloadTarget,
    }

    #[cfg(unix)]
    fn fixture(main: &str, build: Option<&str>) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let crate_dir = directory.path().join("fixture");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(crate_dir.join("src/main.rs"), main).unwrap();
        if let Some(build) = build {
            std::fs::write(crate_dir.join("build.rs"), build).unwrap();
        }
        let target = ReloadTarget::for_test(
            Path::new(env!("CARGO")),
            &crate_dir,
            "fixture",
            &directory.path().join("target"),
        );
        Fixture { directory, target }
    }

    #[cfg(unix)]
    fn finish(job: &mut BuildJob) -> BuildOutcome {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if let Some(outcome) = job.poll() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "the build never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
    #[test]
    fn building_a_fixture_crate_reports_its_executable() {
        let fixture = fixture("fn main() {}\n", None);
        let mut job = BuildJob::spawn(&fixture.target).unwrap();

        let executable = fixture.directory.path().join("target/debug/fixture");
        assert_eq!(
            finish(&mut job),
            BuildOutcome::Built {
                executable: executable.clone()
            }
        );
        assert!(executable.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn building_a_broken_fixture_crate_reports_its_error() {
        let fixture = fixture("fn main() { x }\n", None);
        let mut job = BuildJob::spawn(&fixture.target).unwrap();

        let listed = errors(finish(&mut job));

        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].message, "cannot find value `x` in this scope");
        assert_eq!(listed[0].location, Some(("src/main.rs".to_string(), 1, 13)));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_a_build_stops_cargo_and_its_children() {
        let directory = tempfile::tempdir().unwrap();
        let started = directory.path().join("build-script-started");
        let build = format!(
            "fn main() {{\n    std::fs::write({started:?}, \"\").unwrap();\n    std::thread::sleep(std::time::Duration::from_secs(10));\n}}\n"
        );
        let fixture = fixture("fn main() {}\n", Some(&build));
        let mut job = BuildJob::spawn(&fixture.target).unwrap();
        let group = libc::pid_t::try_from(job.child.id()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(300);
        while !started.exists() {
            assert!(
                job.poll().is_none(),
                "the build ended before its build script ran"
            );
            assert!(Instant::now() < deadline, "the build script never ran");
            std::thread::sleep(Duration::from_millis(10));
        }

        let mut world = World::new();
        world.insert_resource(EventChannel::<ReloadFailed>::new());
        let mut reload = GameReload::new(Ok(fixture.target.clone()));
        reload.phase = ReloadPhase::Building(Box::new(job));
        world.insert_resource(reload);
        crate::reload::install_listeners(&mut world);
        let cancelled = Instant::now();
        world.trigger(CancelReload);

        assert!(cancelled.elapsed() < Duration::from_secs(3));
        let reload = world.get_resource::<GameReload>().unwrap();
        assert!(matches!(reload.phase, ReloadPhase::Idle));
        assert_eq!(
            reload.status(),
            Some(("Reload cancelled".to_string(), false))
        );
        assert_eq!(
            world
                .get_resource::<EventChannel<ReloadFailed>>()
                .unwrap()
                .read_from(&mut Default::default())
                .cloned()
                .collect::<Vec<_>>(),
            [ReloadFailed {
                message: "Reload cancelled".into()
            }]
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            // SAFETY: signal 0 sends nothing; it only asks whether the build's group still has a process.
            let alive = unsafe { libc::killpg(group, 0) } == 0;
            if !alive {
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::ESRCH)
                );
                break;
            }
            assert!(Instant::now() < deadline, "a process of the build is left");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
