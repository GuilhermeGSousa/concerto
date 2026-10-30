use concerto_mesh::{
    primitives::{capsule, cuboid, plane, uv_sphere},
    Mesh,
};
use glam::{Vec2, Vec3};

const EPSILON: f32 = 1e-5;

fn position(mesh: &Mesh, index: u32) -> Vec3 {
    Vec3::from(mesh.vertices[index as usize].pos_coords)
}

fn normal(mesh: &Mesh, index: u32) -> Vec3 {
    Vec3::from(mesh.vertices[index as usize].normal)
}

fn assert_well_formed(mesh: &Mesh) {
    assert_eq!(mesh.indices.len() % 3, 0);
    assert!(mesh
        .indices
        .iter()
        .all(|index| (*index as usize) < mesh.vertices.len()));
    for vertex in &mesh.vertices {
        let normal = Vec3::from(vertex.normal);
        let tangent = Vec3::from(vertex.tangent);
        let bitangent = Vec3::from(vertex.bitangent);
        assert!((normal.length() - 1.0).abs() < EPSILON, "{normal}");
        assert!((tangent.length() - 1.0).abs() < EPSILON, "{tangent}");
        assert!(normal.dot(tangent).abs() < EPSILON);
        assert!(bitangent.abs_diff_eq(normal.cross(tangent), EPSILON));
        assert_eq!(vertex.bone_weights, [0.0; 4]);
    }
    for triangle in mesh.indices.chunks(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        let face =
            (position(mesh, b) - position(mesh, a)).cross(position(mesh, c) - position(mesh, a));
        assert!(face.length() > 1e-7, "degenerate triangle {triangle:?}");
        let vertices = normal(mesh, a) + normal(mesh, b) + normal(mesh, c);
        assert!(face.dot(vertices) > 0.0, "inward triangle {triangle:?}");
    }
}

fn assert_bounds(mesh: &Mesh, half_extents: Vec3) {
    let bounds = mesh.local_aabb().unwrap();
    assert!(bounds.min.abs_diff_eq(-half_extents, EPSILON));
    assert!(bounds.max.abs_diff_eq(half_extents, EPSILON));
}

#[test]
fn a_plane_faces_up_from_the_xz_plane() {
    let mesh = plane(Vec2::new(4.0, 2.0));

    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.indices.len(), 6);
    assert_well_formed(&mesh);
    assert_bounds(&mesh, Vec3::new(2.0, 0.0, 1.0));
    assert!(mesh
        .vertices
        .iter()
        .all(|vertex| vertex.normal == [0.0, 1.0, 0.0]));
}

#[test]
fn a_cuboid_has_its_own_vertices_on_each_face() {
    let mesh = cuboid(Vec3::new(2.0, 4.0, 6.0));

    assert_eq!(mesh.vertices.len(), 24);
    assert_eq!(mesh.indices.len(), 36);
    assert_well_formed(&mesh);
    assert_bounds(&mesh, Vec3::new(1.0, 2.0, 3.0));
    for direction in [Vec3::X, Vec3::Y, Vec3::Z] {
        for side in [direction, -direction] {
            let on_face = mesh
                .vertices
                .iter()
                .filter(|vertex| Vec3::from(vertex.normal) == side)
                .count();
            assert_eq!(on_face, 4, "{side}");
        }
    }
}

#[test]
fn a_sphere_has_every_vertex_at_its_radius_with_a_radial_normal() {
    let mesh = uv_sphere(1.5, 8, 16);

    assert_eq!(mesh.vertices.len(), 9 * 17);
    assert_eq!(mesh.indices.len(), 6 * 16 * 7);
    assert_well_formed(&mesh);
    assert_bounds(&mesh, Vec3::splat(1.5));
    for vertex in &mesh.vertices {
        let position = Vec3::from(vertex.pos_coords);
        assert!((position.length() - 1.5).abs() < EPSILON);
        assert!(position
            .normalize()
            .abs_diff_eq(Vec3::from(vertex.normal), EPSILON));
    }
}

#[test]
fn a_sphere_keeps_a_usable_shape_at_the_smallest_counts() {
    let mesh = uv_sphere(1.0, 0, 0);

    assert_eq!(mesh.vertices.len(), 3 * 4);
    assert_eq!(mesh.indices.len(), 6 * 3);
    assert_well_formed(&mesh);
}

#[test]
fn a_capsule_runs_along_y_past_its_half_height_by_its_radius() {
    let mesh = capsule(0.5, 1.0, 4, 16);

    assert_eq!(mesh.vertices.len(), 10 * 17);
    assert_eq!(mesh.indices.len(), 6 * 16 * 8);
    assert_well_formed(&mesh);
    assert_bounds(&mesh, Vec3::new(0.5, 1.5, 0.5));
}

#[test]
fn a_capsule_has_every_vertex_at_its_radius_from_its_axis_segment() {
    let mesh = capsule(0.25, 2.0, 6, 12);

    for vertex in &mesh.vertices {
        let position = Vec3::from(vertex.pos_coords);
        let on_axis = Vec3::new(0.0, position.y.clamp(-2.0, 2.0), 0.0);
        let outward = position - on_axis;
        assert!((outward.length() - 0.25).abs() < EPSILON);
        assert!(outward
            .normalize()
            .abs_diff_eq(Vec3::from(vertex.normal), EPSILON));
    }
}

#[test]
fn a_capsule_with_no_height_is_a_sphere() {
    let mesh = capsule(1.0, 0.0, 2, 8);

    assert_eq!(mesh.vertices.len(), 5 * 9);
    assert_well_formed(&mesh);
    assert_bounds(&mesh, Vec3::ONE);
}

#[test]
fn a_capsule_keeps_a_usable_shape_at_the_smallest_counts() {
    let mesh = capsule(1.0, 1.0, 0, 0);

    assert_eq!(mesh.vertices.len(), 4 * 4);
    assert_eq!(mesh.indices.len(), 6 * 3 * 2);
    assert_well_formed(&mesh);
}
