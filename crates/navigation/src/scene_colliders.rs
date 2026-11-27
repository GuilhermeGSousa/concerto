use std::collections::HashMap;

use anyhow::Context;
use concerto_foundation::{
    assets::{AssetId, handle::AssetHandle},
    transform::Transform,
};
use concerto_mesh::{Mesh, MeshComponent};
use concerto_physics::{collider::BoxCollider, sensor::Sensor, shape::MeshCollider};
use concerto_reflect::{LeafError, LeafHook, Reflect, apply_value, apply_value_with, decode_leaf};
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};
use glam::{Affine3A, Vec3};

use crate::navmesh::NavMeshGeometry;

enum Shape {
    Box { half_extents: Vec3 },
    Mesh { id: AssetId },
}

struct NodeCollider {
    transform: Affine3A,
    shape: Option<Shape>,
}

struct Placement {
    node: usize,
    parent: Affine3A,
}

struct MeshHandleAsId;

impl LeafHook for MeshHandleAsId {
    fn encode(&mut self, _: &dyn Reflect) -> Option<Result<Vec<u8>, LeafError>> {
        None
    }

    fn decode(&mut self, leaf: &mut dyn Reflect, bytes: &[u8]) -> Option<Result<(), LeafError>> {
        let handle = leaf.downcast_mut::<AssetHandle<Mesh>>()?;
        let mut id = AssetId::nil();
        Some(decode_leaf(&mut id, bytes).map(|()| *handle = AssetHandle::weak(id)))
    }
}

fn names<T>(component: &SerializedComponent) -> bool {
    let path = std::any::type_name::<T>();
    let without_generics = path.split('<').next().unwrap_or(path);
    let short = &path[without_generics.rfind("::").map_or(0, |index| index + 2)..];
    component.type_name == path || component.type_name == short
}

fn find<T>(node: &SceneNode) -> Option<&SerializedComponent> {
    node.components
        .iter()
        .find(|component| names::<T>(component))
}

fn decoded<T: Reflect + Default>(node: &SceneNode) -> Option<T> {
    let component = find::<T>(node)?;
    let mut value = T::default();
    apply_value(&mut value, &component.value);
    Some(value)
}

fn mesh_id(node: &SceneNode) -> Option<AssetId> {
    let component = find::<MeshComponent>(node)?;
    let mut mesh = MeshComponent::default();
    apply_value_with(&mut mesh, &component.value, &mut MeshHandleAsId);
    Some(mesh.handle.id()).filter(|id| !id.is_nil())
}

impl NodeCollider {
    fn of(node: &SceneNode) -> Self {
        let transform = decoded::<Transform>(node).unwrap_or_default();
        let transform = Affine3A::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );
        let sensor = find::<Sensor>(node).is_some();
        let shape = match decoded::<BoxCollider>(node) {
            Some(_) if sensor => None,
            Some(box_collider) => Some(Shape::Box {
                half_extents: box_collider.half_extents.abs().max(Vec3::splat(0.001)),
            }),
            None if find::<MeshCollider>(node).is_some() => {
                mesh_id(node).map(|id| Shape::Mesh { id })
            }
            None => None,
        };
        Self { transform, shape }
    }
}

pub(crate) fn collider_meshes(scene: &Scene) -> Vec<AssetId> {
    let mut ids = Vec::new();
    for node in &scene.nodes {
        if let Some(Shape::Mesh { id }) = NodeCollider::of(node).shape
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

impl NavMeshGeometry {
    pub(crate) fn add_scene(
        &mut self,
        scene: &Scene,
        meshes: &HashMap<AssetId, Mesh>,
    ) -> anyhow::Result<()> {
        let mut is_child = vec![false; scene.nodes.len()];
        for child in scene.nodes.iter().flat_map(|node| &node.children) {
            if let Some(flag) = is_child.get_mut(*child) {
                *flag = true;
            }
        }
        let mut visited = vec![false; scene.nodes.len()];
        let mut pending: Vec<Placement> = (0..scene.nodes.len())
            .rev()
            .filter(|index| !is_child[*index])
            .map(|node| Placement {
                node,
                parent: Affine3A::IDENTITY,
            })
            .collect();
        while let Some(Placement { node, parent }) = pending.pop() {
            if std::mem::replace(&mut visited[node], true) {
                continue;
            }
            let scene_node = &scene.nodes[node];
            let collider = NodeCollider::of(scene_node);
            let world = parent * collider.transform;
            match collider.shape {
                Some(Shape::Box { half_extents }) => self.add_box(world, half_extents),
                Some(Shape::Mesh { id }) => {
                    let mesh = meshes
                        .get(&id)
                        .with_context(|| format!("the collider mesh {id:?} was not read"))?;
                    self.add_collider_mesh(world, mesh, id, &scene_node.name);
                }
                None => {}
            }
            pending.extend(
                scene_node
                    .children
                    .iter()
                    .rev()
                    .filter(|child| **child < scene.nodes.len())
                    .map(|child| Placement {
                        node: *child,
                        parent: world,
                    }),
            );
        }
        Ok(())
    }

    fn add_collider_mesh(&mut self, transform: Affine3A, mesh: &Mesh, id: AssetId, node: &str) {
        if mesh
            .indices
            .iter()
            .any(|index| *index as usize >= mesh.vertices.len())
        {
            log::warn!(
                "skipping the collider of node '{node}' in the navmesh: its mesh {id:?} indexes past its {} vertices",
                mesh.vertices.len()
            );
            return;
        }
        let vertices: Vec<Vec3> = mesh
            .vertices
            .iter()
            .map(|vertex| Vec3::from_array(vertex.pos_coords))
            .collect();
        self.add_triangles(transform, &vertices, &mesh.indices);
    }
}
