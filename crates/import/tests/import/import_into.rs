//! Importing a source from outside the project brings the files it refers to along.
use std::path::{Path, PathBuf};

use concerto_import::config::ContentConfig;
use concerto_import::import_source_into;

fn fixtures(krate: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../{krate}/tests/fixtures"))
        .canonicalize()
        .expect("the fixture directory exists")
}

fn external_gltf() -> PathBuf {
    fixtures("gltf").join("triangle_ext.gltf")
}

fn config() -> ContentConfig {
    ContentConfig::default()
}

fn files_under(root: &Path) -> Vec<String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(
                    path.strip_prefix(root)
                        .expect("under the root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files.sort();
    files
}

fn gltf_with_buffer_one_folder_up(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir.join("models")).expect("models");
    let text = std::fs::read_to_string(external_gltf())
        .expect("fixture")
        .replace("triangle_ext.bin", "../triangle_ext.bin");
    std::fs::write(dir.join("models/tri.gltf"), text).expect("write gltf");
    std::fs::copy(
        fixtures("gltf").join("triangle_ext.bin"),
        dir.join("triangle_ext.bin"),
    )
    .expect("copy buffer");
    dir.join("models/tri.gltf")
}

fn external_obj(_: &Path, _: &Path) -> PathBuf {
    fixtures("obj").join("unread_texture.obj")
}

fn external(_: &Path, _: &Path) -> PathBuf {
    external_gltf()
}

fn one_folder_up(_: &Path, scratch: &Path) -> PathBuf {
    gltf_with_buffer_one_folder_up(scratch)
}

fn already_in_place(project: &Path, _: &Path) -> PathBuf {
    std::fs::create_dir_all(project.join("assets")).expect("assets");
    for name in ["triangle_ext.gltf", "triangle_ext.bin"] {
        std::fs::copy(
            fixtures("gltf").join(name),
            project.join("assets").join(name),
        )
        .expect("copy");
    }
    project.join("assets/triangle_ext.gltf")
}

fn broken_over_an_old_copy(project: &Path, scratch: &Path) -> PathBuf {
    std::fs::create_dir_all(project.join("assets")).expect("assets");
    std::fs::write(project.join("assets/broken.gltf"), b"old").expect("seed");
    std::fs::write(scratch.join("broken.gltf"), b"not a gltf").expect("write");
    scratch.join("broken.gltf")
}

fn files_outside_content(project: &Path) -> Vec<String> {
    files_under(project)
        .into_iter()
        .filter(|file| !file.starts_with("content/"))
        .collect()
}

struct Copy {
    source: fn(&Path, &Path) -> PathBuf,
    destination: &'static str,
    siblings: &'static [&'static str],
    files: &'static [&'static str],
    content: &'static str,
    sub_asset: &'static str,
}

#[test]
fn a_source_imported_into_the_project_brings_the_files_it_refers_to_and_nothing_else() {
    let cases = [
        Copy {
            source: external,
            destination: "assets/models/tri.gltf",
            siblings: &["assets/models/triangle_ext.bin"],
            files: &[
                "assets/models/tri.gltf",
                "assets/models/tri.gltf.import.toml",
                "assets/models/triangle_ext.bin",
            ],
            content: "content/models/tri/scene.gasset",
            sub_asset: "mesh/0",
        },
        Copy {
            source: external_obj,
            destination: "assets/unread_texture.obj",
            siblings: &["assets/unread_texture.mtl"],
            files: &[
                "assets/unread_texture.mtl",
                "assets/unread_texture.obj",
                "assets/unread_texture.obj.import.toml",
            ],
            content: "content/unread_texture/scene.gasset",
            sub_asset: "scene",
        },
        Copy {
            source: already_in_place,
            destination: "assets/triangle_ext.gltf",
            siblings: &[],
            files: &[
                "assets/triangle_ext.bin",
                "assets/triangle_ext.gltf",
                "assets/triangle_ext.gltf.import.toml",
            ],
            content: "content/triangle_ext/scene.gasset",
            sub_asset: "mesh/0",
        },
    ];
    for case in &cases {
        let scratch = tempfile::tempdir().expect("tempdir");
        let project = tempfile::tempdir().expect("tempdir");
        let source = (case.source)(project.path(), scratch.path());
        let imported = import_source_into(&source, case.destination, project.path(), &config())
            .unwrap_or_else(|error| panic!("{}: {error:#}", case.destination));

        assert_eq!(imported.siblings, case.siblings, "{}", case.destination);
        assert_eq!(
            files_outside_content(project.path()),
            case.files,
            "{}",
            case.destination
        );
        assert!(
            imported
                .assets
                .iter()
                .any(|asset| asset.provenance.as_ref().unwrap().sub_asset == case.sub_asset),
            "{}: {:?}",
            case.destination,
            imported.assets
        );
        assert_eq!(
            files_under(&project.path().join("content")),
            [case.content.trim_start_matches("content/")],
            "{}",
            case.destination
        );
        for asset in &imported.assets {
            assert_eq!(asset.address, case.content, "{}", case.destination);
        }
    }
}

struct Refusal {
    case: &'static str,
    source: fn(&Path, &Path) -> PathBuf,
    destination: &'static str,
    left: &'static [&'static str],
}

#[test]
fn an_import_that_cannot_complete_writes_nothing_and_keeps_what_was_there() {
    let cases = [
        Refusal {
            case: "a sibling would land above the project",
            source: one_folder_up,
            destination: "tri.gltf",
            left: &[],
        },
        Refusal {
            case: "a malformed source over an old copy",
            source: broken_over_an_old_copy,
            destination: "assets/broken.gltf",
            left: &["assets/broken.gltf"],
        },
        Refusal {
            case: "a destination above the project",
            source: external,
            destination: "../escaped.gltf",
            left: &[],
        },
    ];
    for case in &cases {
        let scratch = tempfile::tempdir().expect("tempdir");
        let project = tempfile::tempdir().expect("tempdir");
        let source = (case.source)(project.path(), scratch.path());
        let before: Vec<Vec<u8>> = case
            .left
            .iter()
            .map(|file| std::fs::read(project.path().join(file)).expect("seeded"))
            .collect();
        import_source_into(&source, case.destination, project.path(), &config())
            .expect_err(case.case);
        assert_eq!(files_under(project.path()), case.left, "{}", case.case);
        for (file, bytes) in case.left.iter().zip(before) {
            assert_eq!(
                std::fs::read(project.path().join(file)).expect("read"),
                bytes,
                "{}",
                case.case
            );
        }
        assert!(
            !project.path().join("../escaped.gltf").exists(),
            "{}",
            case.case
        );
    }
}

#[test]
fn importing_again_keeps_every_asset_id_and_replaces_the_copied_files() {
    let source = tempfile::tempdir().expect("tempdir");
    for name in ["triangle_ext.gltf", "triangle_ext.bin"] {
        std::fs::copy(fixtures("gltf").join(name), source.path().join(name)).expect("copy");
    }
    let project = tempfile::tempdir().expect("tempdir");
    let gltf = source.path().join("triangle_ext.gltf");
    let first =
        import_source_into(&gltf, "assets/tri.gltf", project.path(), &config()).expect("first");

    let mut buffer = std::fs::read(source.path().join("triangle_ext.bin")).expect("read");
    buffer[0] ^= 0x01;
    std::fs::write(source.path().join("triangle_ext.bin"), &buffer).expect("write");
    let second =
        import_source_into(&gltf, "assets/tri.gltf", project.path(), &config()).expect("second");

    assert_eq!(
        std::fs::read(project.path().join("assets/triangle_ext.bin")).expect("read"),
        buffer,
        "the changed buffer replaces the one copied before"
    );
    assert_eq!(first.assets, second.assets);
}

#[cfg(unix)]
#[test]
fn copies_keep_the_permissions_of_their_sources() {
    use std::os::unix::fs::PermissionsExt;
    let source = tempfile::tempdir().expect("tempdir");
    for name in ["triangle_ext.gltf", "triangle_ext.bin"] {
        let path = source.path().join(name);
        std::fs::copy(fixtures("gltf").join(name), &path).expect("copy");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    }
    let project = tempfile::tempdir().expect("tempdir");
    import_source_into(
        &source.path().join("triangle_ext.gltf"),
        "assets/tri.gltf",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");
    for name in ["tri.gltf", "triangle_ext.bin"] {
        let mode = std::fs::metadata(project.path().join("assets").join(name))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644, "{name}");
    }
}
