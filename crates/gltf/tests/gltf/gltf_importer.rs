//! Covers GltfImporter splitting a single .gltf into independently-imported
//! mesh, material, and scene sub-assets (this fixture has no textures),
//! with the scene's node referencing the mesh/material by stable AssetId
//! through serialized component payloads.
use std::path::{Path, PathBuf};

use concerto_animation::clip::{Additive, AdditiveBase, AnimationClip, ClipRange};
use concerto_animation::event::AnimationEvent;
use concerto_asset_import::{ImportContext, ImportError, ImportOutputs, Importer};
use concerto_ecs::component::Component;
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_gltf::gltf_importer::GltfImporter;
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent, mesh::MeshComponent};
use concerto_reflect::{Reflect, apply_value};
use concerto_scene::node_id::NodeId;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};

fn decode<T: Reflect + Default>(component: &SerializedComponent) -> T {
    let mut value = T::default();
    let warnings = apply_value(&mut value, &component.value);
    assert!(warnings.is_empty(), "{warnings:?}");
    value
}

/// The `AssetId` the node's `MeshComponent` payload points at, if it has one.
fn mesh_handle_id(node: &SceneNode) -> Option<AssetId> {
    node.components
        .iter()
        .find(|c| c.type_name == MeshComponent::name())
        .map(|c| decode::<MeshComponent>(c).handle.id())
}

fn has_component_ending_in(node: &SceneNode, suffix: &str) -> bool {
    node.components
        .iter()
        .any(|c| c.type_name.ends_with(suffix))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct Imported {
    result: Result<(), ImportError>,
    outputs: ImportOutputs,
}

fn import(name: &str, settings: &str) -> Imported {
    let mut ctx = ImportContext::new(PathBuf::from(name)).with_settings(settings.parse().unwrap());
    let result = GltfImporter.import(&fixture(name), &mut ctx);
    Imported {
        result,
        outputs: ctx.into_parts(),
    }
}

fn imported(name: &str, settings: &str) -> ImportOutputs {
    let imported = import(name, settings);
    imported.result.expect("the import should succeed");
    imported.outputs
}

fn emitted<T: serde::de::DeserializeOwned>(outputs: &ImportOutputs, name: &str) -> T {
    let entry = outputs
        .sub_assets
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected a {name} sub-asset"));
    bincode::deserialize(&entry.bytes).expect("the sub-asset must decode")
}

struct Case {
    source: &'static str,
    settings: String,
    emits: &'static [&'static str],
    omits: &'static [&'static str],
    depends_on: &'static [&'static str],
    refused: bool,
}

fn emits(
    source: &'static str,
    emits: &'static [&'static str],
    omits: &'static [&'static str],
    depends_on: &'static [&'static str],
) -> Case {
    Case {
        source,
        settings: String::new(),
        emits,
        omits,
        depends_on,
        refused: false,
    }
}

fn refuses(settings: String) -> Case {
    Case {
        source: "animations_named.gltf",
        settings,
        emits: &[],
        omits: &[],
        depends_on: &[],
        refused: true,
    }
}

const BASE_ID: &str = "d6e76e64-a888-49cd-bc16-61f0566f36ab";

#[test]
fn a_source_and_its_settings_import_their_sub_assets_or_are_refused() {
    let cases = [
        emits(
            "triangle.gltf",
            &["mesh/0", "material/0", "scene"],
            &[],
            &[],
        ),
        emits(
            "triangle_ext.gltf",
            &["mesh/0", "scene"],
            &[],
            &["triangle_ext.bin"],
        ),
        emits(
            "triangle_two_prims.gltf",
            &["mesh/0", "mesh/1", "material/0", "scene"],
            &[],
            &[],
        ),
        emits(
            "skinned.gltf",
            &["skeleton/0", "animation/Wiggle", "scene"],
            &[],
            &[],
        ),
        emits(
            "animations_named.gltf",
            &["animation/1", "animation/Wiggle", "animation/Wiggle_2"],
            &["animation/0", "animation/2"],
            &[],
        ),
        refuses("scale = 0.01\n".to_string()),
        refuses("[events.Wobble]\nhit = [0.5]\n".to_string()),
        refuses("[events.Wiggle]\nhit = [1.5]\n".to_string()),
        refuses("[additive.Wiggle]\nbase = \"not-an-id\"\n".to_string()),
        refuses(format!(
            "[additive.Wiggle]\nbase = \"{BASE_ID}\"\nrange = [0.0, 1.5]\n"
        )),
    ];
    for case in &cases {
        let Imported { result, outputs } = import(case.source, &case.settings);
        let label = format!("{} with {:?}", case.source, case.settings);
        assert_eq!(result.is_err(), case.refused, "{label}: {result:?}");
        let names: Vec<&str> = outputs.sub_assets.iter().map(|s| s.name.as_str()).collect();
        for name in case.emits {
            assert!(names.contains(name), "{label}: no {name} in {names:?}");
        }
        for name in case.omits {
            assert!(!names.contains(name), "{label}: {name} in {names:?}");
        }
        for file in case.depends_on {
            assert!(
                outputs.dependencies.iter().any(|dep| dep
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    == Some(*file)),
                "{label}: {file} is not a dependency"
            );
        }
    }
}

#[test]
fn a_single_primitive_node_carries_its_mesh_material_and_light_by_stable_id() {
    let outputs = imported("triangle.gltf", "");
    let scene: Scene = emitted(&outputs, "scene");
    assert_eq!(scene.nodes.len(), 1);
    let node = &scene.nodes[0];
    assert_eq!(node.name, "Triangle");
    assert_eq!(
        mesh_handle_id(node),
        Some(AssetId::from_path("triangle.gltf#mesh/0"))
    );
    assert!(has_component_ending_in(node, "MaterialComponent"));
    assert!(has_component_ending_in(node, "light::Light"));
    assert!(
        scene
            .referenced_assets
            .contains(&AssetId::from_path("triangle.gltf#mesh/0"))
    );
}

#[test]
fn a_multi_primitive_mesh_becomes_a_parent_with_one_child_per_primitive() {
    let outputs = imported("triangle_two_prims.gltf", "");
    let scene: Scene = emitted(&outputs, "scene");
    assert_eq!(scene.nodes.len(), 3);

    let parent = &scene.nodes[0];
    assert_eq!(parent.name, "Triangle");
    assert_eq!(parent.id, NodeId::from_name_path("Triangle"));
    assert!(mesh_handle_id(parent).is_none());
    assert!(!has_component_ending_in(parent, "MaterialComponent"));
    assert_eq!(parent.children, vec![1, 2]);

    for (k, (child, address)) in scene.nodes[1..]
        .iter()
        .zip([
            "triangle_two_prims.gltf#mesh/0",
            "triangle_two_prims.gltf#mesh/1",
        ])
        .enumerate()
    {
        assert_eq!(child.id, parent.id.primitive(k));
        assert!(child.children.is_empty());
        assert!(has_component_ending_in(child, "MaterialComponent"));
        assert_eq!(mesh_handle_id(child), Some(AssetId::from_path(address)));
    }
}

struct Skin {
    source: &'static str,
    names: [&'static str; 2],
}

#[test]
fn a_skin_becomes_a_parents_first_skeleton_that_the_scene_binds() {
    let skins = [
        Skin {
            source: "skinned.gltf",
            names: ["Root", "Joint"],
        },
        Skin {
            source: "skinned_parent_outside_skin.gltf",
            names: ["Joint", "Tip"],
        },
    ];
    for skin in &skins {
        let skeleton: Skeleton = emitted(&imported(skin.source, ""), "skeleton/0");
        assert_eq!(
            &*skeleton.names,
            skin.names.map(String::from),
            "{}",
            skin.source
        );
        assert_eq!(&*skeleton.parents, [None, Some(0)], "{}", skin.source);
    }

    let outputs = imported("skinned.gltf", "");
    let skeleton: Skeleton = emitted(&outputs, "skeleton/0");
    assert_eq!(skeleton.rest_pose[0], JointPose::IDENTITY);
    assert_eq!(
        skeleton.rest_pose[1].translation,
        glam::Vec3::new(0.0, 1.0, 0.0)
    );

    let scene: Scene = emitted(&outputs, "scene");
    let bound: SkeletonComponent = scene
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|c| c.type_name == SkeletonComponent::name())
        .map(decode)
        .expect("a skinned node must carry a SkeletonComponent");
    assert_eq!(
        bound.skeleton.id(),
        AssetId::from_path("skinned.gltf#skeleton/0")
    );
    assert_eq!(bound.bones.len(), skeleton.bone_count());
}

#[test]
fn multi_primitive_skinned_mesh_binds_every_primitive_child() {
    let outputs = imported("skinned_two_prims.gltf", "");
    let scene: Scene = emitted(&outputs, "scene");

    let skeletons: Vec<SkeletonComponent> = scene
        .nodes
        .iter()
        .flat_map(|n| n.components.iter())
        .filter(|c| c.type_name == SkeletonComponent::name())
        .map(decode::<SkeletonComponent>)
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
        "one rootless binding per primitive child"
    );
    for binding in &bindings {
        assert_eq!(binding.bones, source_skel.bones);
        assert_eq!(binding.skeleton.id(), source_skel.skeleton.id());
    }

    let skeleton: Skeleton = emitted(&outputs, "skeleton/0");
    let clip: AnimationClip = emitted(&outputs, "animation/Wiggle");
    assert!(
        clip.target_ids().any(|id| skeleton.bone_ids.contains(id)),
        "the animation clip must key at least one channel by a skeleton bone id"
    );
}

#[derive(Debug, PartialEq)]
struct NamedKey {
    time: f32,
    name: String,
}

fn named_key(time: f32, name: &str) -> NamedKey {
    NamedKey {
        time,
        name: name.to_string(),
    }
}

fn event_keys(outputs: &ImportOutputs, name: &str) -> Vec<NamedKey> {
    let clip: AnimationClip = emitted(outputs, name);
    clip.events()
        .iter()
        .map(|key| {
            assert_eq!(
                key.event.type_name(),
                std::any::type_name::<AnimationEvent>()
            );
            let mut event = AnimationEvent::default();
            let warnings = apply_value(&mut event, &key.event.value().unwrap());
            assert!(warnings.is_empty(), "{warnings:?}");
            named_key(key.time, event.name.as_str())
        })
        .collect()
}

#[test]
fn event_settings_add_sorted_animation_events_to_the_named_clip() {
    let outputs = imported(
        "animations_named.gltf",
        "[events.Wiggle_2]\nhit = [0.75, 0.25, 0.75]\nstart = [0.0]\n[events.1]\nend = [1.0]\n",
    );
    assert_eq!(
        event_keys(&outputs, "animation/Wiggle_2"),
        [
            named_key(0.0, "start"),
            named_key(0.25, "hit"),
            named_key(0.75, "hit"),
        ]
    );
    assert_eq!(event_keys(&outputs, "animation/1"), [named_key(1.0, "end")]);
}

#[test]
fn an_additive_table_makes_only_the_named_clip_additive_against_its_base() {
    let outputs = imported(
        "animations_named.gltf",
        &format!(
            "[additive.Wiggle_2]\nbase = \"{BASE_ID}\"\nbase_range = [0.5, 1.5]\nrange = {{ start = 0.25, end = 0.75 }}\n"
        ),
    );
    let skeleton: Skeleton = emitted(&outputs, "skeleton/0");
    let mut expected = AdditiveBase::new(AssetHandle::weak(
        AssetId::from_simple_hex(BASE_ID).unwrap(),
    ))
    .with_base_range(ClipRange::new(0.5, 1.5))
    .with_range(ClipRange::new(0.25, 0.75));
    for (parent, id) in skeleton.parents.iter().zip(skeleton.bone_ids.iter()) {
        if parent.is_none() {
            expected = expected.with_root(*id);
        }
    }
    assert!(!expected.roots.is_empty());
    let additive = |name: &str| emitted::<AnimationClip>(&outputs, name).additive().clone();
    assert_eq!(
        additive("animation/Wiggle_2"),
        Additive::LocalSpace(expected)
    );
    assert_eq!(additive("animation/Wiggle"), Additive::None);
    assert_eq!(additive("animation/1"), Additive::None);
}
