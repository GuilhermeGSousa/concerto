//! Importing a source from outside the project brings the files it refers to along.
use std::path::{Path, PathBuf};

use concerto_import::config::ContentConfig;
use concerto_import::{import_source, import_source_into};

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
    fixtures("obj").join("square.obj")
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

fn broken(_: &Path, scratch: &Path) -> PathBuf {
    std::fs::write(scratch.join("broken.gltf"), b"not a gltf").expect("write");
    scratch.join("broken.gltf")
}

fn broken_over_an_old_copy(project: &Path, scratch: &Path) -> PathBuf {
    std::fs::create_dir_all(project.join("assets")).expect("assets");
    std::fs::write(project.join("assets/broken.gltf"), b"old").expect("seed");
    broken(project, scratch)
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
            sub_asset: "mesh/0",
        },
        Copy {
            source: external_obj,
            destination: "assets/square.obj",
            siblings: &["assets/square.mtl"],
            files: &[
                "assets/square.mtl",
                "assets/square.obj",
                "assets/square.obj.import.toml",
            ],
            sub_asset: "material/square",
        },
        Copy {
            source: one_folder_up,
            destination: "assets/models/tri.gltf",
            siblings: &["assets/triangle_ext.bin"],
            files: &[
                "assets/models/tri.gltf",
                "assets/models/tri.gltf.import.toml",
                "assets/triangle_ext.bin",
            ],
            sub_asset: "mesh/0",
        },
        Copy {
            source: external,
            destination: "tri.gltf",
            siblings: &["triangle_ext.bin"],
            files: &["tri.gltf", "tri.gltf.import.toml", "triangle_ext.bin"],
            sub_asset: "mesh/0",
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
        for asset in &imported.assets {
            assert!(project.path().join(&asset.address).is_file());
        }
    }
}

struct Refusal {
    case: &'static str,
    source: fn(&Path, &Path) -> PathBuf,
    destination: Option<&'static str>,
    config: ContentConfig,
    message: &'static str,
    left: &'static [&'static str],
}

fn refusal(case: &'static str, destination: &'static str) -> Refusal {
    Refusal {
        case,
        source: external,
        destination: Some(destination),
        config: config(),
        message: "",
        left: &[],
    }
}

#[test]
fn an_import_that_cannot_complete_writes_nothing_and_keeps_what_was_there() {
    let cases = [
        Refusal {
            source: one_folder_up,
            message: "outside the project",
            ..refusal("a sibling would land above the project", "tri.gltf")
        },
        Refusal {
            source: broken,
            ..refusal("a malformed source", "assets/broken.gltf")
        },
        Refusal {
            source: broken_over_an_old_copy,
            left: &["assets/broken.gltf"],
            ..refusal("a malformed source over an old copy", "assets/broken.gltf")
        },
        refusal("a destination above the project", "../escaped.gltf"),
        refusal(
            "a destination stepping out of the project",
            "assets/../../escaped.gltf",
        ),
        refusal("an absolute destination", "{project}/../escaped.gltf"),
        refusal("a directory destination", "assets/"),
        refusal("an empty destination", ""),
        refusal("a destination of another type", "assets/tri.glb"),
        Refusal {
            config: ContentConfig {
                root: "data".to_string(),
                ..ContentConfig::default()
            },
            ..refusal(
                "a content root that is not content",
                "assets/models/tri.gltf",
            )
        },
        Refusal {
            config: ContentConfig {
                extension: "a.b".to_string(),
                ..ContentConfig::default()
            },
            ..refusal("an extension with a dot", "assets/models/tri.gltf")
        },
        Refusal {
            destination: None,
            config: ContentConfig {
                root: "data".to_string(),
                ..ContentConfig::default()
            },
            ..refusal("a content root that is not content, in place", "")
        },
        Refusal {
            destination: None,
            config: ContentConfig {
                extension: "a.b".to_string(),
                ..ContentConfig::default()
            },
            ..refusal("an extension with a dot, in place", "")
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
        let result = match case.destination {
            Some(destination) => {
                let destination =
                    destination.replace("{project}", project.path().to_str().expect("utf-8"));
                import_source_into(&source, &destination, project.path(), &case.config).map(|_| ())
            }
            None => import_source(&source, project.path(), &case.config).map(|_| ()),
        };
        let error = result.expect_err(case.case);
        assert!(
            format!("{error:#}").contains(case.message),
            "{}: {error:#}",
            case.case
        );
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

    let in_place = import_source(
        &project.path().join("assets/tri.gltf"),
        project.path(),
        &config(),
    )
    .expect("the copied source and its buffer are self-sufficient");
    assert_eq!(
        first.assets, in_place,
        "identities and addresses are stable"
    );

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
