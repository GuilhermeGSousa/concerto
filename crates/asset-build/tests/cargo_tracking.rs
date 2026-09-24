//! Exercise real Cargo invalidation without touching the fixture's Rust source.
#![cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::process::{Command, Output};

use concerto_asset_format::{
    write_content_asset, AssetId, ContentAssetHeader, CONTENT_FORMAT_VERSION,
};

fn cargo(project: &Path) -> Output {
    Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--offline"])
        .current_dir(project)
        // A separate target avoids contention with the parent cargo test.
        .env("CARGO_TARGET_DIR", project.join("target"))
        .env_remove("CONCERTO_ASSET_ROOT")
        .output()
        .expect("run Cargo fixture")
}

fn write_asset(project: &Path, id: AssetId) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: vec![],
        kind: "Fixture".into(),
        provenance: None,
    };
    // This recognizable large payload must never be embedded by the macro.
    let payload = b"ASSET_PAYLOAD_SHOULD_NOT_APPEAR_IN_THE_EXECUTABLE".repeat(100_000);
    std::fs::write(
        project.join("content/fixture.gasset"),
        write_content_asset(&header, &payload).unwrap(),
    )
    .unwrap();
}

fn assert_id(output: Output, id: AssetId) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        id.simple_hex()
    );
}

fn assert_error(output: Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "fixture should fail compilation");
    assert!(
        stderr.contains(expected),
        "expected {expected:?}, got:\n{stderr}"
    );
}

#[test]
fn cargo_rebuilds_uuid_constants_and_reports_invalid_asset_inputs() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::create_dir(root.join("content")).unwrap();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    // Debug string escaping is also TOML basic-string escaping for these paths,
    // including Windows backslashes. Cargo receives this as a file, not a shell.
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "asset-macro-fixture"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
concerto-asset-format = {{ path = {:?} }}
concerto-foundation-macros = {{ path = {:?} }}
[build-dependencies]
concerto-asset-build = {{ path = {:?} }}
"#,
            crates.join("asset-format"),
            crates.join("foundation/macros"),
            crates.join("asset-build"),
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("build.rs"),
        "fn main() { concerto_asset_build::track_assets(\"content\").unwrap(); }",
    )
    .unwrap();
    std::fs::write(root.join("src/main.rs"), r#"
use concerto_asset_format::AssetId;
const ID: AssetId = AssetId::from_bytes(concerto_foundation_macros::asset_id_bytes!("content/fixture.gasset"));
fn main() { println!("{}", ID.simple_hex()); }
"#).unwrap();

    let first = AssetId::new();
    write_asset(root, first);
    assert_id(cargo(root), first);
    let executable = root.join("target/debug").join(format!(
        "asset-macro-fixture{}",
        std::env::consts::EXE_SUFFIX
    ));
    let binary = std::fs::read(executable).unwrap();
    let marker = b"ASSET_PAYLOAD_SHOULD_NOT_APPEAR_IN_THE_EXECUTABLE";
    assert!(!binary.windows(marker.len()).any(|window| window == marker));

    let second = AssetId::new();
    write_asset(root, second);
    assert_id(cargo(root), second);

    std::fs::write(root.join("content/fixture.gasset"), b"broken header").unwrap();
    assert_error(cargo(root), "missing the GRDY magic prefix");
    std::fs::remove_file(root.join("content/fixture.gasset")).unwrap();
    assert_error(cargo(root), "cannot open");

    write_asset(root, second);
    std::fs::write(root.join("build.rs"), "fn main() {}").unwrap();
    assert_error(cargo(root), "requires Cargo dependency tracking");
}
