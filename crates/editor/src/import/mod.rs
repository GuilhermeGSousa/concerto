//! Importing source files into the open project.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, bail};
use concerto_import::ImportedAsset;
use concerto_import::config::ContentConfig;

/// Copies a source into the project at `destination`, then imports it.
pub fn import_source_into(
    source: &Path,
    destination: &str,
    project_root: &Path,
) -> anyhow::Result<Vec<ImportedAsset>> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !concerto_import::supported_extension(extension) {
        bail!("no importer handles '.{extension}'");
    }
    let config = ContentConfig::load_or_default(project_root)?;
    let target = project_root.join(destination);
    let parent = match target.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => project_root,
    };
    std::fs::create_dir_all(parent).with_context(|| format!("creating '{}'", parent.display()))?;
    let same = source
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .is_some_and(|(source, target)| source == target);
    if !same {
        copy_atomically(source, &target, parent)?;
    }
    concerto_import::import_source(&target, project_root, &config)
}

fn copy_atomically(source: &Path, target: &Path, parent: &Path) -> anyhow::Result<()> {
    let context = || format!("copying '{}' to '{}'", source.display(), target.display());
    let mut reader = std::fs::File::open(source).with_context(context)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).with_context(context)?;
    std::io::copy(&mut reader, &mut staged).with_context(context)?;
    staged.flush().with_context(context)?;
    staged
        .persist(target)
        .map_err(|error| error.error)
        .with_context(context)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_dir() -> tempfile::TempDir {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures/square.obj");
        let text = std::fs::read_to_string(fixture).expect("the obj fixture exists");
        let dir = tempfile::tempdir().expect("tempdir");
        let without_materials: String = text
            .lines()
            .filter(|line| !line.starts_with("mtllib") && !line.starts_with("usemtl"))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(dir.path().join("square.obj"), without_materials).expect("write source");
        dir
    }

    #[test]
    fn an_external_source_is_copied_to_its_destination_and_imported() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let written = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import succeeds");
        assert!(
            project.path().join("assets/square.obj").is_file(),
            "the source is copied into the project"
        );
        assert!(!written.is_empty(), "the import emits at least one asset");
        assert!(
            project.path().join(&written[0].address).is_file(),
            "the content asset is written at its registered address"
        );
    }

    #[test]
    fn a_destination_at_the_project_root_needs_no_parent_directory() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        import_source_into(
            &source.path().join("square.obj"),
            "square.obj",
            project.path(),
        )
        .expect("a destination with no parent directory still imports");
        assert!(project.path().join("square.obj").is_file());
    }

    #[test]
    fn a_source_already_at_its_destination_is_left_in_place() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(project.path().join("assets")).expect("assets");
        let inside = project.path().join("assets/square.obj");
        std::fs::copy(source.path().join("square.obj"), &inside).expect("seed the source");
        let before = std::fs::metadata(&inside).expect("metadata").len();
        import_source_into(&inside, "assets/square.obj", project.path()).expect("import succeeds");
        assert_eq!(
            std::fs::metadata(&inside).expect("metadata").len(),
            before,
            "an in-project source at its own destination is not copied over itself"
        );
    }

    #[test]
    fn an_unsupported_extension_errors_before_copying_anything() {
        let project = tempfile::tempdir().expect("tempdir");
        let source = project.path().join("notes.txt");
        std::fs::write(&source, b"not an asset").expect("write");
        let error = import_source_into(&source, "assets/notes.txt", project.path())
            .expect_err("an unsupported extension is refused");
        assert!(
            format!("{error:#}").contains("txt"),
            "the error names the extension, got: {error:#}"
        );
        assert!(
            !project.path().join("assets/notes.txt").exists(),
            "nothing is copied when the extension is refused"
        );
    }

    #[test]
    fn importing_the_same_source_twice_keeps_every_asset_id() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        let first = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("first import");
        let second = import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("second import");
        let ids = |assets: &[concerto_import::ImportedAsset]| {
            assets.iter().map(|a| a.asset_id).collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&first),
            ids(&second),
            "re-importing reuses the sidecar, so asset identities are stable"
        );
    }

    #[test]
    fn a_copy_leaves_no_staging_files_behind() {
        let source = source_dir();
        let project = tempfile::tempdir().expect("tempdir");
        import_source_into(
            &source.path().join("square.obj"),
            "assets/square.obj",
            project.path(),
        )
        .expect("import");
        let names: Vec<_> = std::fs::read_dir(project.path().join("assets"))
            .expect("assets")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().starts_with(".tmp")),
            "no staging file remains, got: {names:?}"
        );
    }
}
