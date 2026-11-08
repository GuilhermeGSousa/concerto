use concerto_navigation::{
    NavMeshBakeError, NavMeshData, NavMeshGeometry, NavMeshQuery, NavMeshSettings, bake,
};
use glam::{Affine3A, Vec3};

struct Bounds {
    min: Vec3,
    max: Vec3,
}

fn bounds(data: &NavMeshData) -> Bounds {
    Bounds {
        min: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MAX), |min, vertex| min.min(*vertex)),
        max: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MIN), |max, vertex| max.max(*vertex)),
    }
}

fn floor(geometry: &mut NavMeshGeometry, center: Vec3, half_x: f32, half_z: f32, top: f32) {
    geometry.add_box(
        Affine3A::from_translation(Vec3::new(center.x, top - 0.5, center.z)),
        Vec3::new(half_x, 0.5, half_z),
    );
}

fn floor_geometry() -> NavMeshGeometry {
    let mut geometry = NavMeshGeometry::new();
    floor(&mut geometry, Vec3::ZERO, 5.0, 5.0, 0.0);
    geometry
}

fn ramp(degrees: f32) -> NavMeshGeometry {
    let rise = 2.0 * degrees.to_radians().tan();
    let mut geometry = NavMeshGeometry::new();
    geometry.add_triangles(
        Affine3A::IDENTITY,
        &[
            Vec3::new(-2.0, -rise, -2.0),
            Vec3::new(-2.0, -rise, 2.0),
            Vec3::new(2.0, rise, 2.0),
            Vec3::new(2.0, rise, -2.0),
        ],
        &[0, 1, 2, 0, 2, 3],
    );
    geometry
}

fn ledge(height: f32) -> NavMeshGeometry {
    let mut geometry = NavMeshGeometry::new();
    floor(&mut geometry, Vec3::new(-2.5, 0.0, 0.0), 2.5, 2.5, 0.0);
    floor(&mut geometry, Vec3::new(2.5, 0.0, 0.0), 2.5, 2.5, height);
    geometry
}

fn outside_box(point: Vec3, half: f32) -> f32 {
    point.x.abs().max(point.z.abs()) - half
}

#[test]
fn a_flat_floor_bakes_to_polygons_inset_by_the_agent_radius() {
    let settings = NavMeshSettings::new();
    let data = bake(&floor_geometry(), &settings).unwrap();
    assert!(!data.polygons.is_empty());
    assert!(
        data.polygons
            .iter()
            .flat_map(|polygon| &polygon.vertices)
            .all(|index| (*index as usize) < data.vertices.len())
    );
    let Bounds { min, max } = bounds(&data);
    let inset = 5.0 - settings.agent_radius;
    for corner in [-min.x, -min.z, max.x, max.z] {
        assert!(
            (corner - inset).abs() <= 2.0 * settings.cell_size,
            "edge at {corner}, expected about {inset}"
        );
    }
    assert!(min.y.abs() <= 0.1 && max.y.abs() <= 0.1, "{min} {max}");
}

#[test]
fn a_path_goes_around_a_box_on_the_floor() {
    let settings = NavMeshSettings::new();
    let mut geometry = floor_geometry();
    geometry.add_box(
        Affine3A::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        Vec3::splat(0.5),
    );
    let query = NavMeshQuery::new(&bake(&geometry, &settings).unwrap());
    let from = Vec3::new(-3.0, 0.0, 0.0);
    let to = Vec3::new(3.0, 0.0, 0.0);
    let path = query.path(from, to).unwrap();
    assert!(path.len() >= 2, "{path:?}");
    assert!(path.last().unwrap().distance(to) < 0.1, "{path:?}");
    let points: Vec<Vec3> = std::iter::once(from).chain(path.iter().copied()).collect();
    for segment in points.windows(2) {
        for step in 0..=20 {
            let point = segment[0].lerp(segment[1], step as f32 / 20.0);
            assert!(
                outside_box(point, 0.5) >= settings.agent_radius - 2.0 * settings.cell_size,
                "{point} on {path:?} is too close to the box"
            );
        }
    }
}

#[test]
fn a_slope_steeper_than_max_slope_is_not_walkable() {
    let settings = NavMeshSettings::new();
    assert!(!bake(&ramp(30.0), &settings).unwrap().polygons.is_empty());
    assert!(bake(&ramp(60.0), &settings).unwrap().polygons.is_empty());
}

#[test]
fn a_ledge_taller_than_step_height_splits_the_mesh() {
    let settings = NavMeshSettings::new();
    let low = Vec3::new(-3.0, 0.0, 0.0);

    let step = NavMeshQuery::new(&bake(&ledge(0.2), &settings).unwrap());
    assert!(step.path(low, Vec3::new(3.0, 0.2, 0.0)).is_some());

    let cliff = NavMeshQuery::new(&bake(&ledge(1.0), &settings).unwrap());
    assert!(cliff.path(low, Vec3::new(-1.5, 0.0, 1.5)).is_some());
    assert!(
        cliff
            .path(Vec3::new(3.0, 1.0, 0.0), Vec3::new(1.5, 1.0, 1.5))
            .is_some()
    );
    assert_eq!(cliff.path(low, Vec3::new(3.0, 1.0, 0.0)), None);
}

#[test]
fn a_bake_past_the_vertex_cap_fails_with_too_many_vertices() {
    let settings = NavMeshSettings::new().with_agent_radius(0.0);
    let mut geometry = NavMeshGeometry::new();
    for x in 0..130 {
        for z in 0..130 {
            let center = Vec3::new(x as f32 * 1.3, 0.0, z as f32 * 1.3);
            floor(&mut geometry, center, 0.5, 0.5, 0.0);
        }
    }
    match bake(&geometry, &settings) {
        Err(NavMeshBakeError::TooManyVertices { count }) => assert!(count > 65_535),
        other => panic!("expected TooManyVertices, got {other:?}"),
    }
}

#[test]
fn an_empty_geometry_bakes_to_an_empty_mesh_with_no_paths() {
    let data = bake(&NavMeshGeometry::new(), &NavMeshSettings::new()).unwrap();
    assert_eq!(data, NavMeshData::default());
    assert_eq!(NavMeshQuery::new(&data).path(Vec3::ZERO, Vec3::X), None);
}

#[test]
fn an_end_just_outside_the_eroded_edge_snaps_onto_the_mesh() {
    let settings = NavMeshSettings::new();
    let query = NavMeshQuery::new(&bake(&floor_geometry(), &settings).unwrap());
    let edge = 5.0 - settings.agent_radius;
    let outside = Vec3::new(edge + settings.agent_radius - 0.05, 0.0, 0.0);

    let to = query.path(Vec3::ZERO, outside).unwrap();
    assert!(
        (to.last().unwrap().x - edge).abs() <= 2.0 * settings.cell_size,
        "{to:?}"
    );
    let from = query.path(outside, Vec3::new(-2.0, 0.0, 0.0)).unwrap();
    assert!(from.last().unwrap().distance(Vec3::new(-2.0, 0.0, 0.0)) < 0.1);

    assert_eq!(query.path(Vec3::ZERO, Vec3::new(7.0, 0.0, 0.0)), None);
}

#[test]
fn a_bake_wider_than_the_cell_cap_fails_with_too_many_cells() {
    let settings = NavMeshSettings::new();
    let mut geometry = floor_geometry();
    floor(&mut geometry, Vec3::new(30_000.0, 0.0, 0.0), 1.0, 1.0, 0.0);
    match bake(&geometry, &settings) {
        Err(NavMeshBakeError::TooManyCells { width, depth }) => {
            assert!(width > 65_535, "{width}");
            assert!(depth <= 65_535, "{depth}");
        }
        other => panic!("expected TooManyCells, got {other:?}"),
    }
}

struct BadSetting {
    name: &'static str,
    settings: NavMeshSettings,
}

fn bad(name: &'static str, settings: NavMeshSettings) -> BadSetting {
    BadSetting { name, settings }
}

#[test]
fn settings_that_cannot_be_baked_with_fail_naming_the_setting() {
    let cases = [
        bad("cell_size", NavMeshSettings::new().with_cell_size(0.0)),
        bad(
            "cell_height",
            NavMeshSettings::new().with_cell_height(f32::NAN),
        ),
        bad(
            "agent_height",
            NavMeshSettings::new().with_agent_height(-1.0),
        ),
        bad(
            "agent_radius",
            NavMeshSettings::new().with_agent_radius(-0.1),
        ),
        bad(
            "step_height",
            NavMeshSettings::new().with_step_height(f32::INFINITY),
        ),
        bad("max_slope", NavMeshSettings::new().with_max_slope(90.0)),
    ];
    for BadSetting { name, settings } in cases {
        match bake(&floor_geometry(), &settings) {
            Err(NavMeshBakeError::InvalidSettings { reason }) => {
                assert!(reason.starts_with(name), "{name}: {reason}")
            }
            other => panic!("{name}: expected InvalidSettings, got {other:?}"),
        }
    }
}
