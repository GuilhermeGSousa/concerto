use std::process::ExitCode;

fn main() -> ExitCode {
    let command = match concerto_project::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error:#}");
            eprintln!("{}", concerto_project::USAGE);
            return ExitCode::FAILURE;
        }
    };
    match concerto_project::run(command) {
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
