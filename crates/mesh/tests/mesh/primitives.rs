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

fn uv(mesh: &Mesh, index: u32) -> Vec2 {
    Vec2::from(mesh.vertices[index as usize].uv_coords)
}

fn assert_uvs_follow_tangents(mesh: &Mesh) {
    for triangle in mesh.indices.chunks(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        let (edge_1, edge_2) = (
            position(mesh, b) - position(mesh, a),
            position(mesh, c) - position(mesh, a),
        );
        let (delta_1, delta_2) = (uv(mesh, b) - uv(mesh, a), uv(mesh, c) - uv(mesh, a));
        let scale = 1.0 / (delta_1.x * delta_2.y - delta_1.y * delta_2.x);
        let along_u = (edge_1 * delta_2.y - edge_2 * delta_1.y) * scale;
        let along_v = (edge_2 * delta_1.x - edge_1 * delta_2.x) * scale;
        let vertex = &mesh.vertices[a as usize];
        assert!(
            Vec3::from(vertex.tangent).dot(along_u) > 0.0,
            "u runs against the tangent in {triangle:?}"
        );
        assert!(
            Vec3::from(vertex.bitangent).dot(along_v) < 0.0,
            "v runs with the bitangent in {triangle:?}"
        );
    }
}

struct Shape {
    case: &'static str,
    mesh: Mesh,
    vertices: usize,
    indices: usize,
    half_extents: Option<Vec3>,
}

#[test]
fn every_shape_is_well_formed_with_its_counts_bounds_and_texture_directions() {
    let shapes = [
        Shape {
            case: "plane",
            mesh: plane(Vec2::new(4.0, 2.0)),
            vertices: 4,
            indices: 6,
            half_extents: Some(Vec3::new(2.0, 0.0, 1.0)),
        },
        Shape {
            case: "cuboid",
            mesh: cuboid(Vec3::new(2.0, 4.0, 6.0)),
            vertices: 24,
            indices: 36,
            half_extents: Some(Vec3::new(1.0, 2.0, 3.0)),
        },
        Shape {
            case: "sphere",
            mesh: uv_sphere(1.5, 8, 16),
            vertices: 9 * 17,
            indices: 6 * 16 * 7,
            half_extents: Some(Vec3::splat(1.5)),
        },
        Shape {
            case: "sphere at the smallest counts",
            mesh: uv_sphere(1.0, 0, 0),
            vertices: 3 * 4,
            indices: 6 * 3,
            half_extents: None,
        },
        Shape {
            case: "capsule",
            mesh: capsule(0.5, 1.0, 4, 16),
            vertices: 10 * 17,
            indices: 6 * 16 * 8,
            half_extents: Some(Vec3::new(0.5, 1.5, 0.5)),
        },
        Shape {
            case: "capsule with no height",
            mesh: capsule(1.0, 0.0, 2, 8),
            vertices: 5 * 9,
            indices: 6 * 8 * 3,
            half_extents: Some(Vec3::ONE),
        },
        Shape {
            case: "capsule with a negative height",
            mesh: capsule(1.0, -1.0, 2, 8),
            vertices: 5 * 9,
            indices: 6 * 8 * 3,
            half_extents: Some(Vec3::ONE),
        },
        Shape {
            case: "capsule at the smallest counts",
            mesh: capsule(1.0, 1.0, 0, 0),
            vertices: 4 * 4,
            indices: 6 * 3 * 2,
            half_extents: None,
        },
    ];
    for shape in &shapes {
        let mesh = &shape.mesh;
        assert_eq!(mesh.vertices.len(), shape.vertices, "{}", shape.case);
        assert_eq!(mesh.indices.len(), shape.indices, "{}", shape.case);
        assert_well_formed(mesh);
        assert_uvs_follow_tangents(mesh);
        let bounds = mesh.local_aabb().unwrap();
        let Some(half_extents) = shape.half_extents else {
            continue;
        };
        assert!(
            bounds.min.abs_diff_eq(-half_extents, EPSILON)
                && bounds.max.abs_diff_eq(half_extents, EPSILON),
            "{}: {bounds:?}",
            shape.case
        );
    }
    for mesh in [uv_sphere(0.0, 4, 8), capsule(0.0, 0.0, 2, 8)] {
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| vertex.uv_coords.iter().all(|value| value.is_finite())));
    }
}

struct Round {
    mesh: Mesh,
    radius: f32,
    half_height: f32,
    segments: usize,
}

#[test]
fn a_round_shape_puts_every_vertex_at_its_radius_from_its_axis_and_closes_its_seam() {
    let shapes = [
        Round {
            mesh: uv_sphere(1.5, 8, 16),
            radius: 1.5,
            half_height: 0.0,
            segments: 16,
        },
        Round {
            mesh: capsule(0.25, 2.0, 6, 12),
            radius: 0.25,
            half_height: 2.0,
            segments: 12,
        },
        Round {
            mesh: uv_sphere(50.0, 8, 16),
            radius: 50.0,
            half_height: 0.0,
            segments: 16,
        },
        Round {
            mesh: capsule(50.0, 1.0, 4, 16),
            radius: 50.0,
            half_height: 1.0,
            segments: 16,
        },
    ];
    for shape in &shapes {
        for vertex in &shape.mesh.vertices {
            let position = Vec3::from(vertex.pos_coords);
            let on_axis = Vec3::new(
                0.0,
                position.y.clamp(-shape.half_height, shape.half_height),
                0.0,
            );
            let outward = position - on_axis;
            assert!((outward.length() - shape.radius).abs() < EPSILON * shape.radius.max(1.0));
            assert!(outward
                .normalize()
                .abs_diff_eq(Vec3::from(vertex.normal), EPSILON));
        }
        for ring in shape.mesh.vertices.chunks(shape.segments + 1) {
            assert_eq!(ring[0].pos_coords, ring[shape.segments].pos_coords);
        }
    }
}

#[test]
fn flat_faces_have_their_own_vertices_facing_out_with_upright_texture() {
    assert!(plane(Vec2::new(4.0, 2.0))
        .vertices
        .iter()
        .all(|vertex| vertex.normal == [0.0, 1.0, 0.0]));

    let mesh = cuboid(Vec3::new(2.0, 4.0, 6.0));
    for direction in [Vec3::X, Vec3::Y, Vec3::Z] {
        for side in [direction, -direction] {
            let on_face = mesh
                .vertices
                .iter()
                .filter(|vertex| Vec3::from(vertex.normal) == side)
                .filter(|vertex| Vec3::from(vertex.pos_coords).dot(side) > 0.0)
                .count();
            assert_eq!(on_face, 4, "{side}");
        }
    }
    for vertex in mesh
        .vertices
        .iter()
        .filter(|vertex| vertex.normal[1] == 0.0)
    {
        let top = vertex.pos_coords[1] > 0.0;
        assert_eq!(vertex.uv_coords[1], if top { 0.0 } else { 1.0 });
    }
}

#[test]
fn a_capsule_spreads_texture_height_by_length() {
    let (radius, half_height) = (0.3, 2.0);
    let mesh = capsule(radius, half_height, 8, 16);
    let cylinder: Vec<f32> = mesh
        .vertices
        .iter()
        .filter(|vertex| vertex.pos_coords[1].abs() >= half_height - EPSILON)
        .filter(|vertex| vertex.pos_coords[1].abs() <= half_height + EPSILON)
        .map(|vertex| vertex.uv_coords[1])
        .collect();
    let span = cylinder.iter().copied().fold(f32::MIN, f32::max)
        - cylinder.iter().copied().fold(f32::MAX, f32::min);
    let length = 2.0 * half_height + std::f32::consts::PI * radius;

    assert!((span - 2.0 * half_height / length).abs() < 1e-3, "{span}");
}
