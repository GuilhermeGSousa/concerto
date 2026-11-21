//! Covers ObjImporter splitting a single .obj/.mtl pair into mesh, material,
//! and scene sub-assets, reusing the same Scene shape as glTF.
use std::path::{Path, PathBuf};

use concerto_asset_import::{ImportContext, ImportError, Importer, NoSettings};
use concerto_obj::obj_importer::ObjImporter;
use concerto_scene::scene::Scene;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct Case {
    source: &'static str,
    dependency: &'static str,
    unreadable: bool,
}

#[test]
fn a_source_tracks_the_files_it_names_or_reports_the_one_it_cannot_read() {
    let cases = [
        Case {
            source: "square.obj",
            dependency: "square.mtl",
            unreadable: false,
        },
        Case {
            source: "unread_texture.obj",
            dependency: "absent.png",
            unreadable: false,
        },
        Case {
            source: "absent_library.obj",
            dependency: "absent.mtl",
            unreadable: true,
        },
    ];
    for case in &cases {
        let path = fixture(case.source);
        let expected = path.with_file_name(case.dependency);
        let mut ctx = ImportContext::new(PathBuf::from(case.source));
        match (
            ObjImporter.import(&path, &NoSettings::default(), None, &mut ctx),
            case.unreadable,
        ) {
            (Ok(()), false) => {
                let dependencies = ctx.into_parts().dependencies;
                assert!(
                    dependencies.iter().any(|d| d.path == expected),
                    "{}: got {:?}",
                    case.source,
                    dependencies.iter().map(|d| &d.path).collect::<Vec<_>>()
                );
            }
            (Err(ImportError::SourceUnreadable { source_path, .. }), true) => {
                assert_eq!(source_path, expected, "{}", case.source);
            }
            (result, _) => panic!("{}: got {result:?}", case.source),
        }
    }
}

#[test]
fn import_emits_mesh_material_and_flat_scene() {
    let fixture = fixture("square.obj");
    let mut ctx = ImportContext::new(PathBuf::from("square.obj"));

    ObjImporter
        .import(&fixture, &NoSettings::default(), None, &mut ctx)
        .expect("importing the square fixture should succeed");
    let outputs = ctx.into_parts();

    let names: Vec<&str> = outputs.sub_assets.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.starts_with("mesh/")),
        "expected a mesh sub-asset, got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("material/")),
        "expected a material sub-asset, got: {names:?}"
    );
    assert!(
        names.contains(&"scene"),
        "expected a scene sub-asset, got: {names:?}"
    );

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let cooked_scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();
    assert_eq!(
        cooked_scene.nodes.len(),
        1,
        "the fixture has one mesh, so one flat scene node"
    );
    assert!(
        cooked_scene.nodes[0].children.is_empty(),
        "OBJ has no hierarchy"
    );

    let component_names: Vec<&str> = cooked_scene.nodes[0]
        .components
        .iter()
        .map(|c| c.type_name.as_str())
        .collect();
    assert!(
        component_names.iter().any(|n| n.ends_with("MeshComponent")),
        "the flat scene node must carry a MeshComponent payload, got: {component_names:?}"
    );
    assert!(
        component_names
            .iter()
            .any(|n| n.ends_with("MaterialComponent")),
        "the fixture ships an .mtl, so the node must carry a MaterialComponent payload, got: {component_names:?}"
    );
    assert!(
        !cooked_scene.referenced_assets.is_empty(),
        "the mesh/material ids the node references must be recorded in referenced_assets"
    );
}
