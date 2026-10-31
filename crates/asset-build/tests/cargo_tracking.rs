//! Exercise real Cargo invalidation without touching the fixture's Rust source.
#![cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::process::{Command, Output};

use concerto_asset_format::{write_content_asset, AssetId, ContentAssetHeader};

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
        asset_id: id,
        references: vec![],
        kind: "Fixture".into(),
        kind_version: 1,
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
    let missing = cargo(root);
    let stderr = String::from_utf8_lossy(&missing.stderr).into_owned();
    assert!(
        stderr.contains("warning") && stderr.contains("no asset at"),
        "expected a missing-asset warning, got:\n{stderr}"
    );
    assert_id(missing, AssetId::nil());

    write_asset(root, second);
    std::fs::write(root.join("build.rs"), "fn main() {}").unwrap();
    assert_error(cargo(root), "requires Cargo dependency tracking");
}

fn write_registry_fixture(project: &Path) {
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("content")).unwrap();
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        format!(
            r#"[package]
name = "asset-registry-fixture"
version = "0.0.0"
edition = "2021"
[workspace]
[build-dependencies]
concerto-asset-build = {{ path = {:?} }}
"#,
            crates.join("asset-build"),
        ),
    )
    .unwrap();
    std::fs::write(
        project.join("build.rs"),
        "fn main() { concerto_asset_build::track_assets(\"content\").unwrap(); }",
    )
    .unwrap();
    std::fs::write(
        project.join("src/main.rs"),
        r#"fn main() { print!("{}", include_str!(concat!(env!("OUT_DIR"), "/concerto_registry.toml"))); }"#,
    )
    .unwrap();
}

fn write_asset_at(project: &Path, address: &str, id: AssetId) {
    let header = ContentAssetHeader {
        asset_id: id,
        references: vec![],
        kind: "Fixture".into(),
        kind_version: 1,
        provenance: None,
    };
    let path = project.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, b"payload").unwrap()).unwrap();
}

fn registry(output: Output) -> Vec<(AssetId, String)> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut entries: Vec<_> =
        concerto_asset_format::parse_registry(&String::from_utf8(output.stdout).unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| (entry.id, entry.address))
            .collect();
    entries.sort();
    entries
}

fn rebuilds(project: &Path) -> bool {
    let output = Command::new(env!("CARGO"))
        .args(["build", "--offline"])
        .current_dir(project)
        .env("CARGO_TARGET_DIR", project.join("target"))
        .env_remove("CONCERTO_ASSET_ROOT")
        .output()
        .expect("build Cargo fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).contains("Compiling asset-registry-fixture")
}

fn sorted(mut entries: Vec<(AssetId, String)>) -> Vec<(AssetId, String)> {
    entries.sort();
    entries
}

#[test]
fn cargo_rebuilds_the_registry_from_the_root_content_toml_names() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_registry_fixture(root);
    std::fs::write(root.join("content.toml"), "extension = \"blob\"\n").unwrap();
    let first = AssetId::new();
    let other = AssetId::new();
    let third = AssetId::new();
    write_asset_at(root, "content/first.blob", first);
    write_asset_at(root, "content/other.gasset", other);
    write_asset_at(root, "content/third.bin", third);

    assert_eq!(
        registry(cargo(root)),
        vec![(first, "content/first.blob".to_string())]
    );
    assert!(!rebuilds(root));

    let second = AssetId::new();
    write_asset_at(root, "content/second.blob", second);
    assert_eq!(
        registry(cargo(root)),
        sorted(vec![
            (first, "content/first.blob".to_string()),
            (second, "content/second.blob".to_string()),
        ])
    );

    std::fs::write(root.join("content.toml"), "extension = \"bin\"\n").unwrap();
    assert_eq!(
        registry(cargo(root)),
        vec![(third, "content/third.bin".to_string())]
    );

    std::fs::remove_file(root.join("content.toml")).unwrap();
    assert_eq!(
        registry(cargo(root)),
        vec![(other, "content/other.gasset".to_string())]
    );
    assert!(!rebuilds(root));
}

#[test]
fn cargo_fails_the_build_on_an_invalid_content_toml() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    write_registry_fixture(root);
    write_asset_at(root, "content/first.gasset", AssetId::new());
    std::fs::write(root.join("content.toml"), "root = \"data\"\n").unwrap();

    assert_error(cargo(root), "content root must be 'content'");

    std::fs::write(root.join("content.toml"), "extension = \"a.b\"\n").unwrap();
    assert_error(
        cargo(root),
        "content extension must be a nonempty extension",
    );
}
