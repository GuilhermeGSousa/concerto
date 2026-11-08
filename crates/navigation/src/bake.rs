use glam::{UVec3, Vec3, Vec3A};
use rerecast::{AreaType, BuildContoursFlags, HeightfieldBuilder, PolygonNavmesh, TriMesh};

use crate::navmesh::{NavMeshBakeError, NavMeshData, NavMeshGeometry, NavMeshSettings, NavPolygon};

const MIN_REGION_AREA: u16 = 8 * 8;
const MERGE_REGION_AREA: u16 = 20 * 20;
const MAX_SIMPLIFICATION_ERROR: f32 = 1.3;
const EDGE_LENGTH_FACTOR: u16 = 8;
const MAX_VERTICES_PER_POLYGON: u16 = 6;

/// Bakes the walkable polygons of `geometry` for the agent `settings` describes; nothing walkable gives an empty mesh.
pub fn bake(
    geometry: &NavMeshGeometry,
    settings: &NavMeshSettings,
) -> Result<NavMeshData, NavMeshBakeError> {
    validate(settings)?;
    if geometry.is_empty() {
        return Ok(NavMeshData::default());
    }
    let mut trimesh = TriMesh {
        vertices: geometry
            .vertices()
            .iter()
            .copied()
            .map(Vec3A::from)
            .collect(),
        indices: geometry
            .triangles()
            .iter()
            .map(|t| UVec3::from_array(*t))
            .collect(),
        area_types: vec![AreaType::NOT_WALKABLE; geometry.triangles().len()],
    };
    trimesh.mark_walkable_triangles(settings.max_slope.to_radians());
    let Some(aabb) = trimesh.compute_aabb() else {
        return Ok(NavMeshData::default());
    };
    let width = ((aabb.max.x - aabb.min.x) / settings.cell_size + 0.5) as u64;
    let depth = ((aabb.max.z - aabb.min.z) / settings.cell_size + 0.5) as u64;
    if width > u16::MAX as u64 || depth > u16::MAX as u64 {
        return Err(NavMeshBakeError::TooManyCells { width, depth });
    }
    let walkable_height = (settings.agent_height / settings.cell_height).ceil() as u16;
    let walkable_climb = (settings.step_height / settings.cell_height).floor() as u16;
    let walkable_radius = (settings.agent_radius / settings.cell_size).ceil() as u16;

    let mut heightfield = HeightfieldBuilder {
        aabb,
        cell_size: settings.cell_size,
        cell_height: settings.cell_height,
    }
    .build()
    .map_err(|error| generation("building the heightfield", error))?;
    heightfield
        .rasterize_triangles(&trimesh, walkable_climb)
        .map_err(|error| generation("rasterizing", error))?;
    heightfield.filter_low_hanging_walkable_obstacles(walkable_climb);
    heightfield.filter_ledge_spans(walkable_height, walkable_climb);
    heightfield.filter_walkable_low_height_spans(walkable_height);

    let mut compact = heightfield
        .into_compact(walkable_height, walkable_climb)
        .map_err(|error| generation("compacting the heightfield", error))?;
    compact.erode_walkable_area(walkable_radius);
    compact.build_distance_field();
    compact
        .build_regions(0, MIN_REGION_AREA, MERGE_REGION_AREA)
        .map_err(|error| generation("building regions", error))?;
    let contours = compact.build_contours(
        MAX_SIMPLIFICATION_ERROR,
        walkable_radius.saturating_mul(EDGE_LENGTH_FACTOR),
        BuildContoursFlags::default(),
    );
    let count = contours
        .contours
        .iter()
        .map(|contour| contour.vertices.len())
        .filter(|count| *count >= 3)
        .sum::<usize>();
    if count > u16::MAX as usize {
        return Err(NavMeshBakeError::TooManyVertices { count });
    }
    let mesh = contours
        .into_polygon_mesh(MAX_VERTICES_PER_POLYGON)
        .map_err(|error| generation("building polygons", error))?;
    Ok(convert(&mesh))
}

fn validate(settings: &NavMeshSettings) -> Result<(), NavMeshBakeError> {
    positive("cell_size", settings.cell_size)?;
    positive("cell_height", settings.cell_height)?;
    positive("agent_height", settings.agent_height)?;
    non_negative("agent_radius", settings.agent_radius)?;
    non_negative("step_height", settings.step_height)?;
    if !(0.0..90.0).contains(&settings.max_slope) {
        return Err(invalid(format!(
            "max_slope is {}, it must be from 0 up to 90 degrees",
            settings.max_slope
        )));
    }
    Ok(())
}

fn positive(name: &str, value: f32) -> Result<(), NavMeshBakeError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(invalid(format!("{name} is {value}, it must be above 0")))
    }
}

fn non_negative(name: &str, value: f32) -> Result<(), NavMeshBakeError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(invalid(format!("{name} is {value}, it must be 0 or more")))
    }
}

fn invalid(reason: String) -> NavMeshBakeError {
    NavMeshBakeError::InvalidSettings { reason }
}

fn generation(stage: &'static str, error: impl std::fmt::Display) -> NavMeshBakeError {
    NavMeshBakeError::Generation {
        stage,
        message: error.to_string(),
    }
}

fn convert(mesh: &PolygonNavmesh) -> NavMeshData {
    let scale = Vec3::new(mesh.cell_size, mesh.cell_height, mesh.cell_size);
    let mut remap = vec![u32::MAX; mesh.vertices.len()];
    let mut data = NavMeshData::default();
    for polygon in mesh.polygons() {
        let vertices: Vec<u32> = polygon
            .map(|index| {
                let slot = &mut remap[index as usize];
                if *slot == u32::MAX {
                    *slot = data.vertices.len() as u32;
                    let cell = mesh.vertices[index as usize].as_vec3();
                    data.vertices.push(mesh.aabb.min + cell * scale);
                }
                *slot
            })
            .collect();
        if vertices.len() >= 3 {
            data.polygons.push(NavPolygon { vertices });
        }
    }
    data
}
