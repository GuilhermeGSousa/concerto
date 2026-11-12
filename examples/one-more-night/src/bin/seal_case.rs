#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::path::PathBuf;
    use std::process::ExitCode;

    use one_more_night::case::CaseFiles;
    use one_more_night::casefile::CaseData;
    use one_more_night::seal::{CaseKey, bundle_path, key_path, seal};

    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: seal-case <case directory>");
        return ExitCode::FAILURE;
    };
    let Ok(files) = CaseFiles::read_dir(&dir) else {
        eprintln!("FAIL: the case directory could not be read");
        return ExitCode::FAILURE;
    };
    if CaseData::manor(&files).is_err() {
        eprintln!("FAIL: the case does not load over the manor");
        return ExitCode::FAILURE;
    }
    let (bundle, key_file) = (bundle_path(), key_path());
    let key = std::fs::read_to_string(&key_file)
        .ok()
        .and_then(|text| CaseKey::from_hex(&text));
    let key = match key {
        Some(key) => key,
        None => {
            let key = CaseKey::generate();
            let written = key_file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&key_file, key.to_hex()));
            if written.is_err() {
                eprintln!(
                    "FAIL: the key could not be written to {}",
                    key_file.display()
                );
                return ExitCode::FAILURE;
            }
            key
        }
    };
    let written = bundle
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&bundle, seal(&files, &key)));
    if written.is_err() {
        eprintln!(
            "FAIL: the bundle could not be written to {}",
            bundle.display()
        );
        return ExitCode::FAILURE;
    }
    println!("PASS: sealed into {}", bundle.display());
    ExitCode::SUCCESS
}

#[cfg(target_arch = "wasm32")]
fn main() {}
