//! Covers Mesh/Vertex round-tripping through bincode directly (no DTO).
use glam::{Mat4, Vec3};
use mesh::mesh::Mesh;
use mesh::vertex::Vertex;

fn sample_vertex(x: f32) -> Vertex {
    Vertex {
        pos_coords: [x, 0.0, 0.0],
        uv_coords: [0.5, 0.5],
        normal: [0.0, 1.0, 0.0],
        tangent: [1.0, 0.0, 0.0],
        bitangent: [0.0, 0.0, 1.0],
        bone_indices: [0, 0, 0, 0],
        bone_weights: [1.0, 0.0, 0.0, 0.0],
    }
}

#[test]
fn local_aabb_handles_empty_and_nonempty_meshes() {
    let empty = Mesh {
        vertices: vec![],
        indices: vec![],
    };
    assert_eq!(empty.local_aabb(), None);

    let mesh = Mesh {
        vertices: vec![sample_vertex(-2.0), sample_vertex(4.0)],
        indices: vec![],
    };
    let bounds = mesh.local_aabb().unwrap();
    assert_eq!(bounds.min, Vec3::new(-2.0, 0.0, 0.0));
    assert_eq!(bounds.max, Vec3::new(4.0, 0.0, 0.0));
    assert_eq!(bounds.center(), Vec3::X);
    assert_eq!(bounds.extent(), Vec3::new(6.0, 0.0, 0.0));
}

#[test]
fn transformed_aabb_contains_all_rotated_and_scaled_corners() {
    let mesh = Mesh {
        vertices: vec![
            Vertex {
                pos_coords: [-1.0, -2.0, -3.0],
                ..Default::default()
            },
            Vertex {
                pos_coords: [1.0, 2.0, 3.0],
                ..Default::default()
            },
        ],
        indices: vec![],
    };
    let transform = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 1.0, 0.5),
        glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        Vec3::new(10.0, 20.0, 30.0),
    );
    let bounds = mesh.local_aabb().unwrap().transformed(transform);

    assert!(bounds.min.abs_diff_eq(Vec3::new(8.0, 18.0, 28.5), 1e-5));
    assert!(bounds.max.abs_diff_eq(Vec3::new(12.0, 22.0, 31.5), 1e-5));
}

#[test]
fn mesh_round_trips_through_bincode_directly() {
    let mesh = Mesh {
        vertices: vec![sample_vertex(0.0), sample_vertex(1.0)],
        indices: vec![0, 1, 0],
    };

    let bytes = bincode::serialize(&mesh).expect("Mesh should serialize through bincode");
    let decoded: Mesh =
        bincode::deserialize(&bytes).expect("Mesh should deserialize back from bincode bytes");

    assert_eq!(
        decoded.indices, mesh.indices,
        "indices must survive a bincode round-trip unchanged"
    );
    assert_eq!(
        decoded.vertices[1].pos_coords,
        [1.0, 0.0, 0.0],
        "per-vertex position data must survive a bincode round-trip unchanged"
    );
}
