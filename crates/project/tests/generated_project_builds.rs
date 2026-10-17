use std::path::Path;
use std::process::Command;

use concerto_project::{new_project, Engine};

#[test]
#[ignore = "builds a whole generated project against this checkout; run by hand"]
fn a_new_project_builds_against_this_checkout() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("built-game");

    new_project(&dir, Engine::Checkout(repository.clone())).unwrap();
    std::fs::copy(repository.join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();

    let status = Command::new(env!("CARGO"))
        .args(["build", "--workspace"])
        .current_dir(&dir)
        .env(
            "CARGO_TARGET_DIR",
            repository.join("target/generated-project"),
        )
        .status()
        .unwrap();

    assert!(
        status.success(),
        "cargo build --workspace failed in {}",
        dir.display()
    );
}
