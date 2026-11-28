//! `cargo-concerto`, the Cargo subcommand `cargo concerto`; [`concerto_project::USAGE`] gives its
//! commands.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let default_engine = std::env::var_os("CONCERTO_ENGINE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let command = match concerto_project::Command::parse(std::env::args().skip(1), default_engine) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error:#}");
            eprintln!("{}", concerto_project::USAGE);
            return ExitCode::FAILURE;
        }
    };
    match command.run() {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
