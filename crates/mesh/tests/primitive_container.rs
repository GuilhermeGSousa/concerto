//! Covers the multi-primitive `Mesh` container: construction, bounds union,
//! and the merged geometry used by colliders and GPU upload.
use concerto_mesh::mesh::Mesh;
use concerto_mesh::primitive::Primitive;
use concerto_mesh::vertex::Vertex;
use glam::Vec3;

fn vertex_at(x: f32, y: f32, z: f32) -> Vertex {
    Vertex {
        pos_coords: [x, y, z],
        ..Default::default()
    }
}

fn triangle(offset: f32) -> Primitive {
    Primitive {
        vertices: vec![
            vertex_at(offset, 0.0, 0.0),
            vertex_at(offset + 1.0, 0.0, 0.0),
            vertex_at(offset, 1.0, 0.0),
        ],
        indices: vec![0, 1, 2],
    }
}

#[test]
fn single_builds_a_one_primitive_mesh() {
    let mesh = Mesh::single(vec![vertex_at(1.0, 2.0, 3.0)], vec![0]);

    assert_eq!(mesh.primitives.len(), 1);
    assert_eq!(mesh.primitives[0].vertices.len(), 1);
    assert_eq!(mesh.primitives[0].indices, vec![0]);
}

#[test]
fn local_aabb_unions_every_primitive() {
    let mesh = Mesh {
        primitives: vec![triangle(0.0), triangle(10.0)],
    };

    let bounds = mesh.local_aabb().expect("a populated mesh has bounds");
    assert_eq!(bounds.min, Vec3::ZERO);
    assert_eq!(bounds.max, Vec3::new(11.0, 1.0, 0.0));
}

#[test]
fn local_aabb_is_none_when_no_primitive_has_vertices() {
    assert_eq!(Mesh { primitives: vec![] }.local_aabb(), None);
    assert_eq!(Mesh::single(vec![], vec![]).local_aabb(), None);
}

#[test]
fn merged_geometry_offsets_each_primitives_indices() {
    let mesh = Mesh {
        primitives: vec![triangle(0.0), triangle(10.0)],
    };

    let (vertices, indices) = mesh.merged_geometry();

    assert_eq!(vertices.len(), 6);
    assert_eq!(
        indices,
        vec![0, 1, 2, 3, 4, 5],
        "the second primitive's indices shift by the first's vertex count"
    );
    assert_eq!(vertices[3].pos_coords, [10.0, 0.0, 0.0]);
}

#[test]
fn meshes_round_trip_through_bincode_with_any_primitive_count() {
    for count in [0usize, 1, 3] {
        let mesh = Mesh {
            primitives: (0..count).map(|i| triangle(i as f32 * 5.0)).collect(),
        };

        let bytes = bincode::serialize(&mesh).expect("serializes");
        let restored: Mesh = bincode::deserialize(&bytes).expect("deserializes");

        assert_eq!(restored.primitives.len(), count);
        if let Some(last) = restored.primitives.last() {
            assert_eq!(
                last.vertices[0].pos_coords,
                [(count - 1) as f32 * 5.0, 0.0, 0.0]
            );
        }
    }
}
