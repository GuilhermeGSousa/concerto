use std::path::Path;

use concerto_foundation::assets::content::check_encoding;
use concerto_mesh::mesh::Mesh;
use concerto_mesh::vertex::Vertex;
use concerto_mesh::{JointPose, Skeleton};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

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
fn mesh_encoding_matches_its_fixture() {
    let mesh = Mesh {
        vertices: vec![Vertex {
            pos_coords: [1.0, 2.0, 3.0],
            uv_coords: [0.25, 0.75],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0],
            bitangent: [0.0, 0.0, 1.0],
            bone_indices: [1, 2, 3, 4],
            bone_weights: [0.4, 0.3, 0.2, 0.1],
        }],
        indices: vec![0, 0, 0],
    };

    check_encoding(&mesh, encoding_dir());
}

#[test]
fn skeleton_encoding_matches_its_fixture() {
    let skeleton = Skeleton::new(
        vec![Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)); 2],
        vec![None, Some(0)],
        vec!["Root".into(), "Child".into()],
        vec![
            JointPose {
                translation: Vec3::new(0.5, 0.0, 0.0),
                rotation: Quat::from_xyzw(0.0, 0.6, 0.0, 0.8),
                scale: Vec3::splat(2.0),
            },
            JointPose {
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::from_xyzw(0.8, 0.0, 0.0, 0.6),
                scale: Vec3::new(1.0, 1.5, 1.0),
            },
        ],
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
    )
    .unwrap();

    check_encoding(&skeleton, encoding_dir());
}
