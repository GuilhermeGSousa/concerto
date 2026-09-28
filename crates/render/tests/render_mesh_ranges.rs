//! Covers the primitive range table `RenderMesh` builds from a `Mesh`.
use concerto_mesh::mesh::Mesh;
use concerto_mesh::primitive::Primitive;
use concerto_mesh::vertex::Vertex;
use concerto_render::render_asset::render_mesh::primitive_ranges;

fn primitive(vertex_count: usize, index_count: usize) -> Primitive {
    Primitive {
        vertices: vec![Vertex::default(); vertex_count],
        indices: vec![0; index_count],
    }
}

#[test]
fn each_primitive_gets_a_contiguous_index_range() {
    let mesh = Mesh {
        primitives: vec![primitive(4, 6), primitive(8, 12), primitive(3, 3)],
    };

    let ranges = primitive_ranges(&mesh);

    assert_eq!(ranges.len(), 3);
    assert_eq!(ranges[0].indices, 0..6);
    assert_eq!(ranges[1].indices, 6..18);
    assert_eq!(ranges[2].indices, 18..21);
}

#[test]
fn ranges_address_each_primitives_geometry_in_the_merged_buffers() {
    let mesh = Mesh {
        primitives: vec![
            Primitive {
                vertices: vec![Vertex::default(); 3],
                indices: vec![0, 1, 2],
            },
            Primitive {
                vertices: vec![
                    Vertex {
                        pos_coords: [7.0, 0.0, 0.0],
                        ..Default::default()
                    };
                    3
                ],
                indices: vec![2, 1, 0],
            },
        ],
    };

    let (vertices, indices) = mesh.merged_geometry();
    let ranges = primitive_ranges(&mesh);

    let second = &ranges[1];
    for &index in &indices[second.indices.start as usize..second.indices.end as usize] {
        let vertex = &vertices[(index as i32 + second.base_vertex) as usize];
        assert_eq!(vertex.pos_coords, [7.0, 0.0, 0.0]);
    }
}

#[test]
fn base_vertex_is_zero_because_indices_are_pre_offset() {
    let mesh = Mesh {
        primitives: vec![primitive(4, 6), primitive(8, 12)],
    };

    let ranges = primitive_ranges(&mesh);

    assert!(ranges.iter().all(|range| range.base_vertex == 0));
}

#[test]
fn a_mesh_with_no_primitives_has_no_ranges() {
    assert!(primitive_ranges(&Mesh { primitives: vec![] }).is_empty());
}
