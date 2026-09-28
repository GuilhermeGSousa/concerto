//! Covers ObjImporter splitting a single .obj/.mtl pair into one mesh, a
//! material, and a scene sub-asset, reusing the same Scene shape as glTF.
use std::path::Path;

use concerto_asset_import::{ImportContext, Importer};
use concerto_ecs::component::Component;
use concerto_foundation::assets::Asset;
use concerto_mesh::mesh::Mesh;
use concerto_obj::obj_importer::ObjImporter;
use concerto_render::assets::material::StandardMaterial;
use concerto_render::components::material::{MaterialComponent, SlotBinding};
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

#[test]
fn an_obj_emits_one_mesh_holding_every_model_as_a_primitive() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two_squares.obj");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("two_squares.obj"));

    ObjImporter
        .import(&fixture, &mut ctx)
        .expect("importing the two-model fixture should succeed");
    let outputs = ctx.into_parts();

    let meshes: Vec<_> = outputs
        .sub_assets
        .iter()
        .filter(|sub| sub.type_name == Mesh::name())
        .collect();
    assert_eq!(meshes.len(), 1, "one Mesh asset per OBJ file");
    assert_eq!(meshes[0].name, "mesh/0");

    let mesh: Mesh = bincode::deserialize(&meshes[0].bytes).expect("payload is a Mesh");
    assert_eq!(mesh.primitives.len(), 2, "one primitive per OBJ model");
    assert_eq!(
        mesh.primitives[1].vertices[0].pos_coords,
        [2.0, 0.0, 0.0],
        "primitives keep the OBJ's model order"
    );

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();
    assert_eq!(scene.nodes.len(), 1, "one scene node draws every model");
    assert_eq!(scene.nodes[0].name, "two_squares");

    let material = scene.nodes[0]
        .components
        .iter()
        .find(|c| c.type_name == MaterialComponent::<StandardMaterial>::name())
        .expect("the node carries the .mtl material");
    let material: MaterialComponent<StandardMaterial> =
        serde_json::from_str(&material.data).unwrap();
    assert!(
        matches!(material.binding, SlotBinding::All(_)),
        "an OBJ's one material covers every model's slot"
    );
}
