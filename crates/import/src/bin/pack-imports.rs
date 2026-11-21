//! Converts a project's imports in place: `pack-imports <project directory>`.
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let Some(project) = arguments.next().filter(|_| arguments.next().is_none()) else {
        eprintln!("usage: pack-imports <project directory>");
        return ExitCode::FAILURE;
    };
    match concerto_import::pack_imports(&PathBuf::from(project)) {
        Ok(report) => {
            for source in &report.sources {
                println!("{}: embedded {}", source.source, source.embedded);
            }
            for skipped in &report.skipped {
                println!("{skipped}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error:?}");
            ExitCode::FAILURE
        }
    }
}
