//! Offline importer that relocates the runtime `OBJLoader`/`MTLLoader` parsing
//! into the import pipeline. A single `.obj` (plus the `.mtl` named in its
//! first `mtllib` line) is split into one `mesh/0` holding a primitive per OBJ
//! model, a single `material/<mtl stem>`, and a one-node `scene` sub-asset,
//! cross-referenced by stable `AssetId`.

use std::path::Path;

use concerto_asset_import::{ImportContext, ImportError, Importer, hash_file_contents};
use concerto_color::Color;
use concerto_ecs::component::scene::SceneComponent;
use concerto_foundation::assets::AssetId;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_foundation::transform::Transform;
use concerto_mesh::mesh::{Mesh, MeshComponent};
use concerto_mesh::primitive::Primitive;
use concerto_mesh::vertex::Vertex;
use concerto_render::assets::material::StandardMaterial;
use concerto_render::components::material::MaterialComponent;
use concerto_render::components::render_entity::SyncWithRenderWorld;
use concerto_scene::scene::{Scene, SceneNode};
use serde::Serialize;

pub struct ObjImporter;

impl Importer for ObjImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["obj"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let (models, _materials) = tobj::load_obj(
            source_path,
            &tobj::LoadOptions {
                single_index: true,
                triangulate: true,
                ..Default::default()
            },
        )
        .map_err(|err| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message: format!("failed to parse OBJ file: {err}"),
        })?;

        let mtl_stem = import_material(source_path, ctx)?;

        let mesh = Mesh {
            primitives: models
                .iter()
                .map(|model| build_primitive(&model.mesh))
                .collect(),
        };
        ctx.emit("mesh/0", &mesh)?;

        let mut referenced_assets: Vec<AssetId> = Vec::new();
        let mut node = SceneNode {
            name: source_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "mesh".to_string()),
            children: vec![],
            components: Vec::new(),
        };
        push_node_component(&mut node, &Transform::default())?;

        let mesh_id = ctx.sub_asset_id("mesh/0");
        push_node_component(
            &mut node,
            &MeshComponent {
                handle: AssetHandle::weak(mesh_id),
            },
        )?;
        referenced_assets.push(mesh_id);

        if let Some(stem) = mtl_stem.as_ref() {
            let material_id = ctx.sub_asset_id(&format!("material/{stem}"));
            push_node_component(
                &mut node,
                &MaterialComponent::<StandardMaterial>::all(AssetHandle::weak(material_id)),
            )?;
            referenced_assets.push(material_id);
        }

        push_node_component(&mut node, &SyncWithRenderWorld)?;
        let nodes = vec![node];

        ctx.emit(
            "scene",
            &Scene {
                nodes,
                referenced_assets,
            },
        )?;

        Ok(())
    }
}

/// Serializes `component` onto `node`, mapping the serde failure into an
/// `ImportError` tagged against the `scene` sub-asset.
fn push_node_component<T: Serialize + SceneComponent>(
    node: &mut SceneNode,
    component: &T,
) -> Result<(), ImportError> {
    node.push_component(component)
        .map_err(|err| ImportError::SerializationFailed {
            sub_asset_name: "scene".to_string(),
            message: err.to_string(),
        })
}

/// Parses the first `mtllib`-referenced `.mtl`, tracks it (and any texture
/// files it names) as build dependencies, and emits a single collapsed
/// `material/<stem>` sub-asset. Returns the stem, or `None` when the OBJ names
/// no material library.
fn import_material(
    source_path: &Path,
    ctx: &mut ImportContext,
) -> Result<Option<String>, ImportError> {
    let obj_text =
        std::fs::read_to_string(source_path).map_err(|err| ImportError::SourceUnreadable {
            source_path: source_path.to_path_buf(),
            message: err.to_string(),
        })?;

    let Some(mtl_name) = obj_text.lines().find_map(|line| {
        if line.starts_with("mtllib") {
            line.split_whitespace().nth(1).map(str::to_string)
        } else {
            None
        }
    }) else {
        return Ok(None);
    };

    let mtl_dir = source_path.parent().unwrap_or_else(|| Path::new(""));
    let mtl_path = mtl_dir.join(&mtl_name);

    let mtl_hash = hash_file_contents(&mtl_path)?;
    ctx.track_dependency(mtl_path.clone(), mtl_hash);

    let (mats, _name_to_index) =
        tobj::load_mtl(&mtl_path).map_err(|err| ImportError::MalformedSource {
            source_path: mtl_path.clone(),
            message: format!("failed to parse MTL file: {err}"),
        })?;

    // Collapse every `newmtl` entry into one StandardMaterial — a verbatim
    // port of the runtime MTLLoader quirk, deliberately preserved here.
    let mut material = StandardMaterial::new(None, None);
    for m in mats {
        if let Some(diffuse_texture) = m.diffuse_texture {
            // TODO(asset-import-pipeline): MTL texture paths are assumed
            // relative to the manifest root, and the standalone ImageImporter
            // always imports as sRGB, so a normal map wired this way loses linear
            // sampling.
            track_texture_dependency(ctx, mtl_dir, &diffuse_texture);
            material.set_base_color_texture(AssetHandle::weak(AssetId::from_path(&format!(
                "{diffuse_texture}#main"
            ))));
        }

        if let Some(normal_texture) = m.normal_texture {
            // TODO(asset-import-pipeline): MTL texture paths are assumed
            // relative to the manifest root, and the standalone ImageImporter
            // always imports as sRGB, so a normal map wired this way loses linear
            // sampling.
            track_texture_dependency(ctx, mtl_dir, &normal_texture);
            material.set_normal_texture(AssetHandle::weak(AssetId::from_path(&format!(
                "{normal_texture}#main"
            ))));
        }

        if let Some(diffuse) = m.diffuse {
            material.set_base_color_factor(Color::rgba(diffuse[0], diffuse[1], diffuse[2], 1.0));
        }

        if let Some(shininess) = m.shininess {
            // Map Blinn-Phong shininess to an equivalent GGX roughness.
            material.set_roughness_factor((2.0 / (shininess + 2.0)).sqrt().clamp(0.045, 1.0));
        }
    }

    let mtl_stem = mtl_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "material".to_string());

    ctx.emit(&format!("material/{mtl_stem}"), &material)?;

    Ok(Some(mtl_stem))
}

fn track_texture_dependency(ctx: &mut ImportContext, mtl_dir: &Path, texture_name: &str) {
    let texture_path = mtl_dir.join(texture_name);
    if let Ok(hash) = hash_file_contents(&texture_path) {
        ctx.track_dependency(texture_path, hash);
    }
}

/// Per-vertex assembly plus the normal/tangent fallback — a verbatim port of
/// the runtime `OBJLoader::load` mesh path. `single_index` + `triangulate`
/// guarantee `positions`/`texcoords`/`normals` are parallel per-vertex arrays
/// and `indices` are triangle lists.
fn build_primitive(mesh_data: &tobj::Mesh) -> Primitive {
    let mut requires_normal_computation = false;

    let vertices = (0..mesh_data.positions.len() / 3)
        .map(|vertex_index| {
            let uv_coords = match mesh_data.texcoords.len() {
                0 => [0.0, 0.0],
                _ => [
                    mesh_data.texcoords[vertex_index * 2],
                    mesh_data.texcoords[vertex_index * 2 + 1],
                ],
            };

            let normal = match mesh_data.normals.len() {
                0 => {
                    requires_normal_computation = true;
                    [0.0, 0.0, 1.0]
                }
                _ => [
                    mesh_data.normals[vertex_index * 3],
                    mesh_data.normals[vertex_index * 3 + 1],
                    mesh_data.normals[vertex_index * 3 + 2],
                ],
            };

            Vertex {
                pos_coords: [
                    mesh_data.positions[vertex_index * 3],
                    mesh_data.positions[vertex_index * 3 + 1],
                    mesh_data.positions[vertex_index * 3 + 2],
                ],
                uv_coords,
                normal,
                tangent: [0.0; 3],
                bitangent: [0.0; 3],
                bone_indices: [0; Vertex::MAX_AFFECTED_BONES],
                bone_weights: [0.0; Vertex::MAX_AFFECTED_BONES],
            }
        })
        .collect::<Vec<_>>();

    let mut primitive = Primitive {
        vertices,
        indices: mesh_data.indices.clone(),
    };

    if requires_normal_computation {
        primitive.compute_normals();
    }
    primitive.compute_tangents();

    primitive
}
