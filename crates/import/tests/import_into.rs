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

#[test]
fn an_external_gltf_brings_its_buffer_along() {
    let project = tempfile::tempdir().expect("tempdir");
    let imported = import_source_into(
        &external_gltf(),
        "assets/models/tri.gltf",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");

    assert!(project.path().join("assets/models/tri.gltf").is_file());
    assert!(
        project
            .path()
            .join("assets/models/triangle_ext.bin")
            .is_file(),
        "the buffer keeps its place beside the source"
    );
    assert_eq!(imported.siblings, ["assets/models/triangle_ext.bin"]);
    assert!(!imported.assets.is_empty());
    for asset in &imported.assets {
        assert!(project.path().join(&asset.address).is_file());
    }
}

#[test]
fn a_copied_source_can_be_imported_again_where_it_landed() {
    let project = tempfile::tempdir().expect("tempdir");
    let first = import_source_into(
        &external_gltf(),
        "assets/models/tri.gltf",
        project.path(),
        &config(),
    )
    .expect("first import");
    let second = import_source(
        &project.path().join("assets/models/tri.gltf"),
        project.path(),
        &config(),
    )
    .expect("the copied source and its buffer are self-sufficient");
    assert_eq!(first.assets, second, "identities and addresses are stable");
}

#[test]
fn an_external_obj_brings_its_material_library_along() {
    let project = tempfile::tempdir().expect("tempdir");
    let imported = import_source_into(
        &fixtures("obj").join("square.obj"),
        "assets/square.obj",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");
    assert!(project.path().join("assets/square.mtl").is_file());
    assert_eq!(imported.siblings, ["assets/square.mtl"]);
    assert!(
        imported
            .assets
            .iter()
            .any(|asset| asset.sub_asset_name == "material/square"),
        "the material library was read, got {:?}",
        imported.assets
    );
}

fn gltf_with_buffer_one_folder_up() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("models")).expect("models");
    let text = std::fs::read_to_string(external_gltf())
        .expect("fixture")
        .replace("triangle_ext.bin", "../triangle_ext.bin");
    std::fs::write(dir.path().join("models/tri.gltf"), text).expect("write gltf");
    std::fs::copy(
        fixtures("gltf").join("triangle_ext.bin"),
        dir.path().join("triangle_ext.bin"),
    )
    .expect("copy buffer");
    dir
}

#[test]
fn a_sibling_in_a_parent_folder_keeps_its_relative_position() {
    let source = gltf_with_buffer_one_folder_up();
    let project = tempfile::tempdir().expect("tempdir");
    let imported = import_source_into(
        &source.path().join("models/tri.gltf"),
        "assets/models/tri.gltf",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");
    assert_eq!(imported.siblings, ["assets/triangle_ext.bin"]);
    assert!(project.path().join("assets/triangle_ext.bin").is_file());
}

#[test]
fn a_sibling_that_would_land_outside_the_project_fails_and_writes_nothing() {
    let source = gltf_with_buffer_one_folder_up();
    let project = tempfile::tempdir().expect("tempdir");
    let error = import_source_into(
        &source.path().join("models/tri.gltf"),
        "tri.gltf",
        project.path(),
        &config(),
    )
    .expect_err("the buffer would land above the project root");
    assert!(
        format!("{error:#}").contains("outside the project"),
        "got: {error:#}"
    );
    assert!(
        files_under(project.path()).is_empty(),
        "nothing is written, got {:?}",
        files_under(project.path())
    );
}

#[test]
fn a_failed_import_copies_nothing() {
    let source = tempfile::tempdir().expect("tempdir");
    std::fs::write(source.path().join("broken.gltf"), b"not a gltf").expect("write");
    let project = tempfile::tempdir().expect("tempdir");
    import_source_into(
        &source.path().join("broken.gltf"),
        "assets/broken.gltf",
        project.path(),
        &config(),
    )
    .expect_err("a malformed source is refused");
    assert!(
        files_under(project.path()).is_empty(),
        "nothing is written, got {:?}",
        files_under(project.path())
    );
}

#[test]
fn a_failed_import_keeps_what_was_already_at_the_destination() {
    let source = tempfile::tempdir().expect("tempdir");
    std::fs::write(source.path().join("broken.gltf"), b"not a gltf").expect("write");
    let project = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("assets")).expect("assets");
    std::fs::write(project.path().join("assets/broken.gltf"), b"old").expect("seed");
    import_source_into(
        &source.path().join("broken.gltf"),
        "assets/broken.gltf",
        project.path(),
        &config(),
    )
    .expect_err("a malformed source is refused");
    assert_eq!(files_under(project.path()), ["assets/broken.gltf"]);
    assert_eq!(
        std::fs::read(project.path().join("assets/broken.gltf")).expect("read"),
        b"old"
    );
}

#[test]
fn importing_again_replaces_the_source_and_its_siblings_and_keeps_every_asset_id() {
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

#[test]
fn a_source_already_at_its_destination_is_imported_in_place() {
    let project = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(project.path().join("assets")).expect("assets");
    for name in ["triangle_ext.gltf", "triangle_ext.bin"] {
        std::fs::copy(
            fixtures("gltf").join(name),
            project.path().join("assets").join(name),
        )
        .expect("copy");
    }
    let imported = import_source_into(
        &project.path().join("assets/triangle_ext.gltf"),
        "assets/triangle_ext.gltf",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");
    assert!(imported.siblings.is_empty(), "nothing needed copying");
    assert!(!imported.assets.is_empty());
}

#[test]
fn a_destination_that_is_not_a_file_inside_the_project_is_refused() {
    let project = tempfile::tempdir().expect("tempdir");
    let outside = project.path().join("../escaped.gltf");
    for destination in [
        "../escaped.gltf",
        "assets/../../escaped.gltf",
        outside.to_str().expect("utf-8"),
        "assets/",
        "",
        "assets/tri.glb",
    ] {
        import_source_into(&external_gltf(), destination, project.path(), &config())
            .expect_err(destination);
    }
    assert!(
        files_under(project.path()).is_empty(),
        "nothing is written, got {:?}",
        files_under(project.path())
    );
    assert!(!outside.exists());
}

#[test]
fn a_destination_at_the_project_root_needs_no_parent_directory() {
    let project = tempfile::tempdir().expect("tempdir");
    import_source_into(&external_gltf(), "tri.gltf", project.path(), &config())
        .expect("a root-level destination imports");
    assert!(project.path().join("tri.gltf").is_file());
    assert!(project.path().join("triangle_ext.bin").is_file());
}

#[test]
fn copies_leave_no_staging_files_behind() {
    let project = tempfile::tempdir().expect("tempdir");
    import_source_into(
        &external_gltf(),
        "assets/tri.gltf",
        project.path(),
        &config(),
    )
    .expect("the import succeeds");
    assert_eq!(
        files_under(&project.path().join("assets")),
        ["tri.gltf", "tri.gltf.import.toml", "triangle_ext.bin"]
    );
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
