//! Importing a source from outside the project brings the files it refers to along.
use std::path::{Path, PathBuf};

use concerto_foundation::assets::content::{
    decode_content_asset, read_asset_record, read_content_asset, resave_content_asset,
    write_content_asset,
};
use concerto_foundation::assets::registry::{AssetEntry, AssetRegistry};
use concerto_foundation::assets::Asset;
use concerto_import::config::ContentConfig;
use concerto_import::{import_source_into, ImportedSource};
use concerto_render::assets::texture::{Compression, MaxSize, Texture, TextureRole};

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
        let imported = import_source_into(
            &source,
            case.destination,
            project.path(),
            &config(),
            Default::default(),
        )
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
        import_source_into(
            &source,
            case.destination,
            project.path(),
            &config(),
            Default::default(),
        )
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
    let first = import_source_into(
        &gltf,
        "assets/tri.gltf",
        project.path(),
        &config(),
        Default::default(),
    )
    .expect("first");

    let mut buffer = std::fs::read(source.path().join("triangle_ext.bin")).expect("read");
    buffer[0] ^= 0x01;
    std::fs::write(source.path().join("triangle_ext.bin"), &buffer).expect("write");
    let second = import_source_into(
        &gltf,
        "assets/tri.gltf",
        project.path(),
        &config(),
        Default::default(),
    )
    .expect("second");

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
        Default::default(),
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

fn texture_options(text: &str) -> toml::Table {
    format!("texture_settings = {{ {text} }}")
        .parse()
        .expect("options parse")
}

fn write_png(path: &Path, shade: u8) -> Vec<u8> {
    image::RgbaImage::from_pixel(8, 8, image::Rgba([shade, 255 - shade, 64, 255]))
        .save(path)
        .expect("write png");
    std::fs::read(path).expect("read png")
}

fn write_material_gltf(dir: &Path, images: usize) {
    let mut materials = vec![
        serde_json::json!({
            "pbrMetallicRoughness": {
                "baseColorTexture": { "index": 0 },
                "metallicRoughnessTexture": { "index": 2 }
            },
            "normalTexture": { "index": 1 }
        }),
        serde_json::json!({ "normalTexture": { "index": 2 } }),
    ];
    if images > 3 {
        materials.push(serde_json::json!({
            "pbrMetallicRoughness": { "baseColorTexture": { "index": 3 } }
        }));
    }
    let gltf = serde_json::json!({
        "asset": { "version": "2.0" },
        "images": (0..images).map(|i| serde_json::json!({ "uri": format!("image{i}.png") })).collect::<Vec<_>>(),
        "textures": (0..images).map(|i| serde_json::json!({ "source": i })).collect::<Vec<_>>(),
        "materials": materials,
    });
    std::fs::write(dir.join("materials.gltf"), gltf.to_string()).expect("write gltf");
}

struct ImportedTexture {
    entry: AssetEntry,
    texture: Texture,
}

fn imported_texture(project: &Path, imported: &ImportedSource, name: &str) -> ImportedTexture {
    let entry = imported
        .assets
        .iter()
        .find(|asset| asset.provenance.as_ref().unwrap().sub_asset == name)
        .unwrap_or_else(|| panic!("{name} is imported: {:?}", imported.assets))
        .clone();
    let record = read_asset_record(project, &entry).expect("record");
    let (_, texture) = decode_content_asset::<Texture>(&record)
        .unwrap_or_else(|error| panic!("{name}: {error:#}"));
    ImportedTexture { entry, texture }
}

#[test]
fn texture_sources_keep_their_bytes_and_their_settings_across_reimports() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let project = tempfile::tempdir().expect("tempdir");
    let root = project.path();
    let images: Vec<Vec<u8>> = (0..4)
        .map(|i| write_png(&scratch.path().join(format!("image{i}.png")), 40 * i as u8))
        .collect();
    let picture = write_png(&scratch.path().join("picture.png"), 200);
    write_material_gltf(scratch.path(), 3);
    let gltf = scratch.path().join("materials.gltf");

    let first = import_source_into(
        &gltf,
        "assets/materials.gltf",
        root,
        &config(),
        texture_options("max_size = 1024"),
    )
    .expect("the glTF imports");
    let image = import_source_into(
        &scratch.path().join("picture.png"),
        "assets/picture.png",
        root,
        &config(),
        texture_options("max_size = 1024"),
    )
    .expect("the image imports");
    let main = imported_texture(root, &image, "main").texture;
    assert_eq!(main.source.as_ref().unwrap().bytes, picture);
    assert_eq!(main.role, TextureRole::Color);
    assert_eq!(main.settings.max_size, MaxSize::S1024);
    for (name, image, role) in [
        ("texture/0", 0, TextureRole::Color),
        ("texture/1_linear", 1, TextureRole::Normal),
        ("texture/2_linear", 2, TextureRole::Data),
    ] {
        let texture = imported_texture(root, &first, name).texture;
        assert_eq!(
            texture.source.as_ref().unwrap().bytes,
            images[image],
            "{name}"
        );
        assert_eq!(texture.role, role, "{name}");
        assert_eq!(texture.settings.max_size, MaxSize::S1024, "{name}");
    }
    for sidecar in [
        "assets/materials.gltf.import.toml",
        "assets/picture.png.import.toml",
    ] {
        let text = std::fs::read_to_string(root.join(sidecar)).expect("sidecar");
        assert!(!text.contains("max_size"), "{sidecar}: {text}");
    }

    let ImportedTexture {
        entry: normal_entry,
        texture: normal,
    } = imported_texture(root, &first, "texture/1_linear");
    let edited = Texture::imported(
        normal.role,
        normal.settings.with_compression(Compression::Bc7),
        normal.source.clone().unwrap(),
    );
    let mut registry =
        AssetRegistry::from_content_tree(root, "content", "gasset").expect("registry");
    resave_content_asset(&edited, root, &normal_entry.address, &mut registry).expect("resave");
    write_material_gltf(scratch.path(), 4);
    let second = import_source_into(
        &gltf,
        "assets/materials.gltf",
        root,
        &config(),
        texture_options("max_size = 512"),
    )
    .expect("the glTF imports again");
    let normal = imported_texture(root, &second, "texture/1_linear").texture;
    assert_eq!(normal.settings.compression, Compression::Bc7);
    assert_eq!(normal.settings.max_size, MaxSize::S1024);
    let ImportedTexture {
        entry: color_entry,
        texture: color,
    } = imported_texture(root, &second, "texture/0");
    assert_eq!(color.settings.max_size, MaxSize::S1024);
    let added = imported_texture(root, &second, "texture/3").texture;
    assert_eq!(added.settings.max_size, MaxSize::S512);
    assert_eq!(added.source.as_ref().unwrap().bytes, images[3]);

    let path = root.join(&color_entry.address);
    let file = std::fs::read(&path).expect("read");
    let (mut header, payload) = read_content_asset(&file).expect("content asset");
    header.kind_version = Texture::FORMAT_VERSION - 1;
    std::fs::write(
        &path,
        write_content_asset(&header, payload).expect("encode"),
    )
    .expect("write");
    let third = import_source_into(
        &gltf,
        "assets/materials.gltf",
        root,
        &config(),
        texture_options("max_size = 512"),
    )
    .expect("an old texture version re-imports");
    let color = imported_texture(root, &third, "texture/0").texture;
    assert_eq!(color.settings.max_size, MaxSize::S512);

    let refused = import_source_into(
        &gltf,
        "assets/materials.gltf",
        root,
        &config(),
        texture_options("compression = \"bc9\""),
    )
    .expect_err("an unknown compression is refused");
    assert!(format!("{refused:#}").contains("bc9"), "{refused:#}");
}
