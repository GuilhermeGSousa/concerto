use glam::{Vec3, Vec3Swizzles};
use polyanya::{Mesh, Polygon, Vertex};

use crate::navmesh::NavMeshData;

const OUTSIDE: u32 = u32::MAX;

/// Finds paths over a [`NavMeshData`], built once from it.
#[derive(Clone, Debug)]
pub struct NavMeshQuery {
    mesh: Option<Mesh>,
}

struct Corner {
    polygon: u32,
    next: u32,
    previous: u32,
}

impl NavMeshQuery {
    /// Builds the query mesh for `data`; an empty mesh has no paths.
    pub fn new(data: &NavMeshData) -> Self {
        Self { mesh: build(data) }
    }

    /// The corners after `from` up to `to`, both snapped to the mesh; `from` itself is not included, or `None` when either end is off the mesh or no path joins them.
    pub fn path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        self.mesh.as_ref()?.path_3d(from, to)
    }
}

fn build(data: &NavMeshData) -> Option<Mesh> {
    let polygons: Vec<Vec<u32>> = data
        .polygons
        .iter()
        .filter(|polygon| {
            polygon.vertices.len() >= 3
                && polygon
                    .vertices
                    .iter()
                    .all(|index| (*index as usize) < data.vertices.len())
        })
        .map(|polygon| counter_clockwise(&polygon.vertices, &data.vertices))
        .collect();
    if polygons.is_empty() {
        return None;
    }
    let mut corners: Vec<Vec<Corner>> = data.vertices.iter().map(|_| Vec::new()).collect();
    for (index, polygon) in polygons.iter().enumerate() {
        let count = polygon.len();
        for at in 0..count {
            corners[polygon[at] as usize].push(Corner {
                polygon: index as u32,
                next: polygon[(at + 1) % count],
                previous: polygon[(at + count - 1) % count],
            });
        }
    }
    let vertices = data
        .vertices
        .iter()
        .zip(&corners)
        .map(|(position, corners)| Vertex::new(position.xz(), ring(corners)))
        .collect();
    let polygons = polygons
        .into_iter()
        .map(|vertices| Polygon::new(vertices, false))
        .collect();
    let mut mesh = match Mesh::new(vertices, polygons) {
        Ok(mesh) => mesh,
        Err(error) => {
            log::warn!("navmesh has no query mesh: {error}");
            return None;
        }
    };
    mesh.update_is_one_way();
    let heights = data.vertices.iter().map(|vertex| vertex.y).collect();
    if let Err(error) = mesh.layers[0].set_heights(heights) {
        log::warn!("navmesh has no query mesh: {error}");
        return None;
    }
    Some(mesh)
}

fn counter_clockwise(polygon: &[u32], vertices: &[Vec3]) -> Vec<u32> {
    let count = polygon.len();
    let doubled_area: f32 = (0..count)
        .map(|at| {
            let a = vertices[polygon[at] as usize].xz();
            let b = vertices[polygon[(at + 1) % count] as usize].xz();
            a.perp_dot(b)
        })
        .sum();
    if doubled_area < 0.0 {
        polygon.iter().rev().copied().collect()
    } else {
        polygon.to_vec()
    }
}

fn ring(corners: &[Corner]) -> Vec<u32> {
    let after = |corner: &Corner| {
        corners
            .iter()
            .position(|other| other.next == corner.previous)
    };
    let has_before = |at: usize| {
        corners
            .iter()
            .any(|other| corners[at].next == other.previous)
    };
    let mut visited = vec![false; corners.len()];
    let mut ring = Vec::new();
    let starts = (0..corners.len())
        .filter(|at| !has_before(*at))
        .chain(0..corners.len());
    for start in starts {
        if visited[start] {
            continue;
        }
        let mut at = Some(start);
        while let Some(current) = at.filter(|current| !visited[*current]) {
            visited[current] = true;
            ring.push(corners[current].polygon);
            at = after(&corners[current]);
        }
        if at.is_none() {
            ring.push(OUTSIDE);
        }
    }
    if ring.is_empty() {
        ring.push(OUTSIDE);
    }
    ring
}
