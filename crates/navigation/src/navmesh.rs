use std::fmt;

use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Affine3A, Vec3};
use serde::{Deserialize, Serialize};

const BOX_CORNERS: [Vec3; 8] = [
    Vec3::new(-1.0, -1.0, -1.0),
    Vec3::new(1.0, -1.0, -1.0),
    Vec3::new(1.0, 1.0, -1.0),
    Vec3::new(-1.0, 1.0, -1.0),
    Vec3::new(-1.0, -1.0, 1.0),
    Vec3::new(1.0, -1.0, 1.0),
    Vec3::new(1.0, 1.0, 1.0),
    Vec3::new(-1.0, 1.0, 1.0),
];

const BOX_INDICES: [u32; 36] = [
    0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2,
    6, 1, 6, 5,
];

/// World-space triangles a navmesh is baked from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavMeshGeometry {
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
}

impl NavMeshGeometry {
    /// An empty geometry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a box of `half_extents` around the origin of `transform`, as 12 triangles.
    pub fn add_box(&mut self, transform: Affine3A, half_extents: Vec3) {
        let corners = BOX_CORNERS.map(|corner| corner * half_extents.abs());
        self.add_triangles(transform, &corners, &BOX_INDICES);
    }

    /// Adds the triangles `indices` makes of `vertices`, placed by `transform`.
    pub fn add_triangles(&mut self, transform: Affine3A, vertices: &[Vec3], indices: &[u32]) {
        let base = self.vertices.len() as u32;
        let mirrored = transform.matrix3.determinant() < 0.0;
        self.vertices.extend(
            vertices
                .iter()
                .map(|vertex| transform.transform_point3(*vertex)),
        );
        self.triangles
            .extend(indices.as_chunks::<3>().0.iter().map(|[a, b, c]| {
                if mirrored {
                    [base + a, base + c, base + b]
                } else {
                    [base + a, base + b, base + c]
                }
            }));
    }

    /// The world-space vertices.
    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices
    }

    /// The triangles, as indices into [`Self::vertices`].
    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    /// Whether there are no triangles.
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }
}

/// Convex walkable polygons over shared world-space vertices.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavMeshData {
    /// The world-space vertices the polygons share.
    pub vertices: Vec<Vec3>,
    /// The walkable polygons.
    pub polygons: Vec<NavPolygon>,
}

/// One convex polygon of a [`NavMeshData`], as indices into its vertices.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NavPolygon {
    /// The polygon's corners, in order, as indices into [`NavMeshData::vertices`].
    pub vertices: Vec<u32>,
}

/// The agent a navmesh is baked for, and the resolution it is baked at.
///
/// Lengths are in metres and `max_slope` is in degrees.
#[derive(Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Default)]
pub struct NavMeshSettings {
    /// How far the mesh keeps from walls and edges.
    pub agent_radius: f32,
    /// The headroom the agent needs.
    pub agent_height: f32,
    /// The steepest walkable slope, in degrees.
    pub max_slope: f32,
    /// The tallest ledge the agent steps up or down.
    pub step_height: f32,
    /// The horizontal size of a cell; smaller cells follow the geometry closer and bake slower.
    pub cell_size: f32,
    /// The vertical size of a cell.
    pub cell_height: f32,
}

impl NavMeshSettings {
    /// Settings for a human-sized agent at 10 cm cells.
    pub fn new() -> Self {
        Self {
            agent_radius: 0.3,
            agent_height: 1.8,
            max_slope: 45.0,
            step_height: 0.35,
            cell_size: 0.1,
            cell_height: 0.05,
        }
    }

    /// Sets how far the mesh keeps from walls and edges.
    pub fn with_agent_radius(mut self, agent_radius: f32) -> Self {
        self.agent_radius = agent_radius;
        self
    }

    /// Sets the headroom the agent needs.
    pub fn with_agent_height(mut self, agent_height: f32) -> Self {
        self.agent_height = agent_height;
        self
    }

    /// Sets the steepest walkable slope, in degrees.
    pub fn with_max_slope(mut self, max_slope: f32) -> Self {
        self.max_slope = max_slope;
        self
    }

    /// Sets the tallest ledge the agent steps up or down.
    pub fn with_step_height(mut self, step_height: f32) -> Self {
        self.step_height = step_height;
        self
    }

    /// Sets the horizontal size of a cell.
    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.cell_size = cell_size;
        self
    }

    /// Sets the vertical size of a cell.
    pub fn with_cell_height(mut self, cell_height: f32) -> Self {
        self.cell_height = cell_height;
        self
    }
}

impl Default for NavMeshSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a navmesh could not be baked.
#[derive(Clone, Debug, PartialEq)]
pub enum NavMeshBakeError {
    /// The settings cannot be baked with, such as a cell size that is not positive.
    InvalidSettings {
        /// What is wrong with the settings.
        reason: String,
    },
    /// The geometry spans more cells along one axis than a bake can index.
    TooManyCells {
        /// The cell count along x.
        width: u64,
        /// The cell count along z.
        depth: u64,
    },
    /// The mesh would have more vertices than a bake can index.
    TooManyVertices {
        /// The number of vertices the mesh would have.
        count: usize,
    },
    /// The generator failed at `stage`.
    Generation {
        /// The name of the generator stage that failed.
        stage: &'static str,
        /// What the generator reported.
        message: String,
    },
}

impl fmt::Display for NavMeshBakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NavMeshBakeError::InvalidSettings { reason } => {
                write!(f, "navmesh settings are invalid: {reason}")
            }
            NavMeshBakeError::TooManyCells { width, depth } => write!(
                f,
                "the geometry is {width} by {depth} cells, more than 65535 along one axis; use a larger cell_size or less geometry"
            ),
            NavMeshBakeError::TooManyVertices { count } => write!(
                f,
                "the navmesh would have {count} vertices, more than 65535; use a larger cell_size"
            ),
            NavMeshBakeError::Generation { stage, message } => {
                write!(f, "navmesh generation failed while {stage}: {message}")
            }
        }
    }
}

impl std::error::Error for NavMeshBakeError {}
