//! Covers GltfImporter splitting a single .gltf into independently-imported
//! mesh, material, and scene sub-assets (this fixture has no textures),
//! with the scene's node referencing the mesh/material by stable AssetId
//! through serialized component payloads.
use std::path::Path;

use concerto_asset_import::{ImportContext, Importer};
use concerto_ecs::component::Component;
use concerto_foundation::assets::Asset;
use concerto_foundation::assets::AssetId;
use concerto_gltf::gltf_importer::GltfImporter;
use concerto_mesh::{
    SkeletonComponent,
    mesh::{Mesh, MeshComponent},
};
use concerto_render::assets::material::StandardMaterial;
use concerto_render::components::material::{MaterialComponent, SlotBinding};
use concerto_scene::scene::{Scene, SceneNode};

/// The `AssetId` the node's `MeshComponent` payload points at, if it has one.
fn mesh_handle_id(node: &SceneNode) -> Option<AssetId> {
    node.components
        .iter()
        .find(|c| c.type_name == MeshComponent::name())
        .map(|c| {
            serde_json::from_str::<MeshComponent>(&c.data)
                .expect("a MeshComponent payload must deserialize")
                .handle
                .id()
        })
}

fn has_component_ending_in(node: &SceneNode, suffix: &str) -> bool {
    node.components
        .iter()
        .any(|c| c.type_name.ends_with(suffix))
}

#[test]
fn import_emits_mesh_material_and_scene_sub_assets() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle.gltf");
    let relative_source = Path::new("triangle.gltf");
    let mut ctx = ImportContext::new(relative_source.to_path_buf());

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("importing the triangle fixture should succeed");
    let outputs = ctx.into_parts();

    let names: Vec<&str> = outputs.sub_assets.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"mesh/0"),
        "expected a mesh/0 sub-asset, got: {names:?}"
    );
    assert!(
        names.contains(&"material/0"),
        "expected a material/0 sub-asset, got: {names:?}"
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
    assert_eq!(cooked_scene.nodes.len(), 1);
    assert_eq!(cooked_scene.nodes[0].name, "Triangle");
    assert_eq!(
        mesh_handle_id(&cooked_scene.nodes[0]),
        Some(AssetId::from_path("triangle.gltf#mesh/0")),
        "the scene node's MeshComponent must carry the exact same AssetId a runtime load of 'triangle.gltf#mesh/0' would compute"
    );
    assert!(
        has_component_ending_in(&cooked_scene.nodes[0], "MaterialComponent"),
        "the drawable node must also carry a MaterialComponent payload"
    );
    assert!(
        cooked_scene
            .referenced_assets
            .contains(&AssetId::from_path("triangle.gltf#mesh/0")),
        "the mesh id must be recorded in referenced_assets for import-time validation"
    );
}

#[test]
fn import_emits_light_and_extras_components() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle.gltf");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("triangle.gltf"));

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("import should succeed");
    let outputs = ctx.into_parts();

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let cooked_scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();
    let names: Vec<&str> = cooked_scene.nodes[0]
        .components
        .iter()
        .map(|c| c.type_name.as_str())
        .collect();

    // `Component::name()` is the fully-qualified path, so match on the suffix
    // the way `has_component_ending_in` does elsewhere in this file.
    assert!(
        names.iter().any(|n| n.ends_with("light::Light")),
        "the punctual light must become a Light component, got: {names:?}"
    );
    assert!(
        names.contains(&"MeshCollider"),
        "a Blender extras entry must become a component payload verbatim, got: {names:?}"
    );
}

#[test]
fn import_emits_skeleton_and_animation_sub_assets() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skinned.gltf");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("skinned.gltf"));

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("importing the skinned fixture should succeed");
    let outputs = ctx.into_parts();

    let names: Vec<&str> = outputs.sub_assets.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"skeleton/0"),
        "expected a skeleton sub-asset, got: {names:?}"
    );
    assert!(
        names.contains(&"animation/0"),
        "expected an animation sub-asset, got: {names:?}"
    );

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let cooked_scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();

    let skinned_node = cooked_scene
        .nodes
        .iter()
        .find(|node| {
            node.components
                .iter()
                .any(|c| c.type_name == SkeletonComponent::name())
        })
        .expect("a skinned node must carry a SkeletonComponent");

    let payload = skinned_node
        .components
        .iter()
        .find(|c| c.type_name == SkeletonComponent::name())
        .unwrap();
    let scene_skeleton: SkeletonComponent = serde_json::from_str(&payload.data).unwrap();

    assert_eq!(
        scene_skeleton.skeleton.id(),
        AssetId::from_path("skinned.gltf#skeleton/0"),
        "the skeleton handle must address the emitted skeleton sub-asset"
    );
    assert_eq!(
        scene_skeleton.bones.len(),
        scene_skeleton.bone_ids.len(),
        "every bone must have a matching stable id for animation channel lookup"
    );
    assert!(
        !scene_skeleton.bones.is_empty(),
        "the fixture's skin has joints, so bones must not be empty"
    );
}

#[test]
fn import_tracks_external_buffer_as_dependency() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle_ext.gltf");
    let relative_source = Path::new("triangle_ext.gltf");
    let mut ctx = ImportContext::new(relative_source.to_path_buf());

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("importing the external-buffer fixture should succeed");

    let dependencies = ctx.into_parts().dependencies;
    assert!(
        dependencies
            .iter()
            .any(|dep| dep.path.file_name().and_then(|n| n.to_str()) == Some("triangle_ext.bin")),
        "the external .bin buffer must be tracked as an import dependency so a stale \
         incremental re-import can't ship old geometry, got: {:?}",
        dependencies
            .iter()
            .map(|d| d.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

fn import_fixture(file_name: &str) -> concerto_asset_import::ImportOutputs {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(file_name);
    let mut ctx = ImportContext::new(std::path::PathBuf::from(file_name));
    GltfImporter
        .import(&fixture, &mut ctx)
        .unwrap_or_else(|err| panic!("importing {file_name} should succeed: {err:?}"));
    ctx.into_parts()
}

fn mesh_sub_asset_names(outputs: &concerto_asset_import::ImportOutputs) -> Vec<&str> {
    outputs
        .sub_assets
        .iter()
        .filter(|sub| sub.type_name == Mesh::name())
        .map(|sub| sub.name.as_str())
        .collect()
}

fn scene_of(outputs: &concerto_asset_import::ImportOutputs) -> Scene {
    let entry = outputs
        .sub_assets
        .iter()
        .find(|sub| sub.name == "scene")
        .expect("a scene sub-asset is emitted");
    bincode::deserialize(&entry.bytes).expect("the scene deserializes")
}

fn material_of(node: &SceneNode) -> MaterialComponent<StandardMaterial> {
    let payload = node
        .components
        .iter()
        .find(|c| c.type_name == MaterialComponent::<StandardMaterial>::name())
        .expect("the node carries a MaterialComponent");
    serde_json::from_str(&payload.data).expect("a MaterialComponent payload must deserialize")
}

#[test]
fn a_multi_primitive_gltf_mesh_emits_one_sub_asset_holding_every_primitive() {
    let outputs = import_fixture("triangle_two_prims.gltf");

    assert_eq!(mesh_sub_asset_names(&outputs), vec!["mesh/0"]);

    let entry = outputs
        .sub_assets
        .iter()
        .find(|sub| sub.name == "mesh/0")
        .unwrap();
    let mesh: Mesh = bincode::deserialize(&entry.bytes).expect("payload is a Mesh");
    assert_eq!(mesh.primitives.len(), 2);
}

#[test]
fn a_multi_primitive_node_draws_its_mesh_without_synthesized_children() {
    let outputs = import_fixture("triangle_two_prims.gltf");
    let scene = scene_of(&outputs);

    assert_eq!(scene.nodes.len(), 1, "primitives fan out at extract time");
    let node = &scene.nodes[0];
    assert_eq!(node.name, "Triangle");
    assert!(node.children.is_empty());
    assert_eq!(
        mesh_handle_id(node),
        Some(AssetId::from_path("triangle_two_prims.gltf#mesh/0"))
    );
    assert!(
        matches!(material_of(node).binding, SlotBinding::All(_)),
        "primitives sharing one material bind it to every slot"
    );
}

#[test]
fn a_mesh_sub_asset_is_named_after_its_source_mesh() {
    let outputs = import_fixture("barrel_per_primitive_materials.gltf");

    assert_eq!(mesh_sub_asset_names(&outputs), vec!["mesh/Barrel"]);
    assert_eq!(
        mesh_handle_id(&scene_of(&outputs).nodes[0]),
        Some(AssetId::from_path(
            "barrel_per_primitive_materials.gltf#mesh/Barrel"
        ))
    );
}

#[test]
fn primitives_with_different_materials_bind_one_per_slot() {
    let outputs = import_fixture("barrel_per_primitive_materials.gltf");
    let scene = scene_of(&outputs);

    let SlotBinding::PerSlot(slots) = material_of(&scene.nodes[0]).binding else {
        panic!("differing primitive materials must bind per slot");
    };
    let slot_ids: Vec<Option<AssetId>> = slots
        .iter()
        .map(|slot| slot.as_ref().map(|handle| handle.id()))
        .collect();
    let material = |index: usize| {
        Some(AssetId::from_path(&format!(
            "barrel_per_primitive_materials.gltf#material/{index}"
        )))
    };
    assert_eq!(
        slot_ids,
        vec![material(0), material(1), material(2)],
        "a primitive without a material draws with the default material emitted after the real ones"
    );
    for id in slot_ids.into_iter().flatten() {
        assert!(scene.referenced_assets.contains(&id));
    }
}

#[test]
fn duplicate_and_missing_mesh_names_resolve_by_mesh_index() {
    let outputs = import_fixture("duplicate_mesh_names.gltf");

    assert_eq!(
        mesh_sub_asset_names(&outputs),
        vec!["mesh/Crate", "mesh/Crate.1", "mesh/2"]
    );

    let scene = scene_of(&outputs);
    for (node, expected) in scene
        .nodes
        .iter()
        .zip(["mesh/Crate", "mesh/Crate.1", "mesh/2"])
    {
        assert_eq!(
            mesh_handle_id(node),
            Some(AssetId::from_path(&format!(
                "duplicate_mesh_names.gltf#{expected}"
            ))),
            "node {} must reference {expected}",
            node.name
        );
    }
}

#[test]
fn a_skinned_multi_primitive_node_carries_one_skeleton_component() {
    let outputs = import_fixture("skinned_two_prims.gltf");
    let scene = scene_of(&outputs);

    let skeletons: Vec<SkeletonComponent> = scene
        .nodes
        .iter()
        .flat_map(|n| n.components.iter())
        .filter(|c| c.type_name == SkeletonComponent::name())
        .map(|c| serde_json::from_str(&c.data).unwrap())
        .collect();
    assert_eq!(
        skeletons.len(),
        1,
        "the skinned node owns the only SkeletonComponent; primitives no longer get rootless copies"
    );
    let skeleton = &skeletons[0];
    assert!(skeleton.root.is_some());

    let skinned_node = scene
        .nodes
        .iter()
        .find(|node| {
            node.components
                .iter()
                .any(|c| c.type_name == SkeletonComponent::name())
        })
        .unwrap();
    assert!(
        mesh_handle_id(skinned_node).is_some(),
        "the skinned node draws its mesh itself"
    );

    // The Wiggle clip's channel key must be one of those bone ids, or animation
    // silently does nothing.
    let clip_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "animation/0")
        .unwrap();
    let clip: concerto_animation::clip::AnimationClip =
        bincode::deserialize(&clip_entry.bytes).unwrap();
    assert!(
        clip.target_ids().any(|id| skeleton.bone_ids.contains(id)),
        "the animation clip must key at least one channel by a skeleton bone id"
    );
}
