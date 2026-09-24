//! Covers ObjImporter splitting a single .obj/.mtl pair into mesh, material,
//! and scene sub-assets, reusing the same Scene shape as glTF.
use std::path::Path;

use concerto_asset_import::{ImportContext, Importer};
use concerto_obj_loader::obj_importer::ObjImporter;
use concerto_scene::scene::Scene;

#[test]
fn import_emits_mesh_material_and_flat_scene() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/square.obj");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("square.obj"));

    ObjImporter
        .import(&fixture, &mut ctx)
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

    assert!(
        outputs
            .dependencies
            .iter()
            .any(|d| d.path.file_name().unwrap() == "square.mtl"),
        "the referenced .mtl file must be tracked as a dependency for incremental rebuilds"
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
