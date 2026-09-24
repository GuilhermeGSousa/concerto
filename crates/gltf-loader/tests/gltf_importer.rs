//! Covers GltfImporter splitting a single .gltf into independently-imported
//! mesh, material, and scene sub-assets (this fixture has no textures),
//! with the scene's node referencing the mesh/material by stable AssetId
//! through serialized component payloads.
use std::path::Path;

use concerto_asset_import::{ImportContext, Importer};
use concerto_core::assets::AssetId;
use concerto_ecs::component::Component;
use concerto_gltf_loader::gltf_importer::GltfImporter;
use concerto_mesh::{SkeletonComponent, mesh::MeshComponent};
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

#[test]
fn import_flattens_multi_primitive_mesh_into_child_nodes() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle_two_prims.gltf");
    let relative_source = Path::new("triangle_two_prims.gltf");
    let mut ctx = ImportContext::new(relative_source.to_path_buf());

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("importing the two-primitive fixture should succeed");
    let outputs = ctx.into_parts();

    let names: Vec<&str> = outputs.sub_assets.iter().map(|s| s.name.as_str()).collect();
    for expected in ["mesh/0", "mesh/1", "material/0", "scene"] {
        assert!(
            names.contains(&expected),
            "expected a {expected} sub-asset, got: {names:?}"
        );
    }

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let cooked_scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();
    assert_eq!(
        cooked_scene.nodes.len(),
        3,
        "one parent node plus one appended child node per primitive"
    );

    let parent = &cooked_scene.nodes[0];
    assert_eq!(parent.name, "Triangle", "parent keeps the glTF node name");
    assert!(
        mesh_handle_id(parent).is_none(),
        "a multi-primitive parent node holds no mesh of its own"
    );
    assert!(
        !has_component_ending_in(parent, "MaterialComponent"),
        "a multi-primitive parent node holds no material of its own"
    );
    assert_eq!(
        parent.children,
        vec![1, 2],
        "parent points at the two appended primitive child nodes"
    );

    for (child_index, expected_mesh_addr) in [
        (1usize, "triangle_two_prims.gltf#mesh/0"),
        (2usize, "triangle_two_prims.gltf#mesh/1"),
    ] {
        let child = &cooked_scene.nodes[child_index];
        assert!(
            child.children.is_empty(),
            "primitive child node {child_index} is a leaf"
        );
        assert!(
            has_component_ending_in(child, "MaterialComponent"),
            "primitive child node {child_index} carries a material component"
        );
        assert_eq!(
            mesh_handle_id(child),
            Some(AssetId::from_path(expected_mesh_addr)),
            "primitive child node {child_index} must reference {expected_mesh_addr} by stable AssetId"
        );
    }
}

#[test]
fn multi_primitive_skinned_mesh_binds_every_primitive_child() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skinned_two_prims.gltf");
    let mut ctx = ImportContext::new(std::path::PathBuf::from("skinned_two_prims.gltf"));

    GltfImporter
        .import(&fixture, &mut ctx)
        .expect("importing the two-primitive skinned fixture should succeed");
    let outputs = ctx.into_parts();

    let scene_entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == "scene")
        .unwrap();
    let scene: Scene = bincode::deserialize(&scene_entry.bytes).unwrap();

    let skeletons: Vec<SkeletonComponent> = scene
        .nodes
        .iter()
        .flat_map(|n| n.components.iter())
        .filter(|c| c.type_name == SkeletonComponent::name())
        .map(|c| serde_json::from_str(&c.data).unwrap())
        .collect();
    let source_skel = skeletons
        .iter()
        .find(|skeleton| skeleton.root.is_some())
        .expect("source node must carry a rooted SkeletonComponent");
    let bindings: Vec<&SkeletonComponent> = skeletons
        .iter()
        .filter(|skeleton| skeleton.root.is_none())
        .collect();

    assert_eq!(
        bindings.len(),
        2,
        "one rootless SkeletonComponent per primitive child, got {}",
        bindings.len()
    );
    for binding in &bindings {
        assert_eq!(
            binding.bone_ids, source_skel.bone_ids,
            "primitive child binding must share the source skeleton's bone ids"
        );
        assert_eq!(binding.skeleton.id(), source_skel.skeleton.id());
    }

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
        clip.target_ids()
            .any(|id| source_skel.bone_ids.contains(id)),
        "the animation clip must key at least one channel by a skeleton bone id"
    );
}
