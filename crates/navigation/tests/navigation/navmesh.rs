use concerto_navigation::{NavMeshData, NavMeshGeometry, NavMeshQuery, NavMeshSettings};
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
    let data = floor_geometry().bake(&settings).unwrap();
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
    let query = NavMeshQuery::new(&geometry.bake(&settings).unwrap());
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
fn slopes_past_max_slope_are_not_walkable_and_ledges_past_step_height_split_the_mesh() {
    let settings = NavMeshSettings::new();
    assert!(!ramp(30.0).bake(&settings).unwrap().polygons.is_empty());
    assert!(ramp(60.0).bake(&settings).unwrap().polygons.is_empty());

    let low = Vec3::new(-3.0, 0.0, 0.0);

    let step = NavMeshQuery::new(&ledge(0.2).bake(&settings).unwrap());
    assert!(step.path(low, Vec3::new(3.0, 0.2, 0.0)).is_some());

    let cliff = NavMeshQuery::new(&ledge(1.0).bake(&settings).unwrap());
    assert!(cliff.path(low, Vec3::new(-1.5, 0.0, 1.5)).is_some());
    assert!(
        cliff
            .path(Vec3::new(3.0, 1.0, 0.0), Vec3::new(1.5, 1.0, 1.5))
            .is_some()
    );
    assert_eq!(cliff.path(low, Vec3::new(3.0, 1.0, 0.0)), None);
}

#[test]
fn an_end_just_outside_the_eroded_edge_snaps_onto_the_mesh() {
    let settings = NavMeshSettings::new();
    let query = NavMeshQuery::new(&floor_geometry().bake(&settings).unwrap());
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
