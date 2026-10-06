use concerto_mesh::{Aabb, Mesh, Ray, Vertex};
use glam::{Mat4, Quat, Vec3};

fn unit_box() -> Aabb {
    Aabb {
        min: Vec3::splat(-1.0),
        max: Vec3::splat(1.0),
    }
}

fn ray(origin: [f32; 3], direction: [f32; 3]) -> Ray {
    Ray {
        origin: origin.into(),
        direction: direction.into(),
    }
}

fn vertex(position: [f32; 3]) -> Vertex {
    Vertex {
        pos_coords: position,
        ..Default::default()
    }
}

fn two_walls() -> Mesh {
    Mesh {
        vertices: vec![
            vertex([-1.0, -1.0, 2.0]),
            vertex([1.0, -1.0, 2.0]),
            vertex([0.0, 1.0, 2.0]),
            vertex([-1.0, -1.0, 5.0]),
            vertex([1.0, -1.0, 5.0]),
            vertex([0.0, 1.0, 5.0]),
        ],
        indices: vec![3, 4, 5, 0, 1, 2],
    }
}

#[test]
fn a_box_is_entered_where_the_ray_first_touches_it() {
    let entry = unit_box().ray_entry(&ray([0.0, 0.0, -5.0], [0.0, 0.0, 1.0]));
    assert_eq!(entry, Some(4.0));
    let diagonal = unit_box()
        .ray_entry(&ray([-3.0, 0.0, -3.0], [1.0, 0.0, 1.0]))
        .unwrap();
    assert!((diagonal - 2.0).abs() < 1e-5);
}

#[test]
fn a_ray_starting_inside_enters_at_zero() {
    assert_eq!(
        unit_box().ray_entry(&ray([0.2, 0.1, 0.0], [1.0, 0.0, 0.0])),
        Some(0.0)
    );
}

#[test]
fn a_box_beside_or_behind_the_ray_is_missed() {
    assert_eq!(
        unit_box().ray_entry(&ray([3.0, 0.0, -5.0], [0.0, 0.0, 1.0])),
        None,
        "parallel to the faces and outside them"
    );
    assert_eq!(
        unit_box().ray_entry(&ray([0.0, 0.0, 5.0], [0.0, 0.0, 1.0])),
        None,
        "the box is behind the origin"
    );
    assert_eq!(
        unit_box().ray_entry(&ray([0.0, 5.0, -5.0], [0.0, 0.1, 1.0])),
        None
    );
}

#[test]
fn a_ray_along_a_face_still_hits() {
    assert_eq!(
        unit_box().ray_entry(&ray([1.0, 0.0, -5.0], [0.0, 0.0, 1.0])),
        Some(4.0)
    );
}

#[test]
fn the_nearest_triangle_wins_regardless_of_index_order_or_facing() {
    let mesh = two_walls();
    assert_eq!(
        mesh.ray_hit(&ray([0.0, 0.0, 0.0], [0.0, 0.0, 1.0])),
        Some(2.0)
    );
    assert_eq!(
        mesh.ray_hit(&ray([0.0, 0.0, 9.0], [0.0, 0.0, -1.0])),
        Some(4.0),
        "from behind, the far wall is nearer and is hit on its back face"
    );
}

#[test]
fn a_ray_outside_the_triangles_edges_or_pointing_away_misses() {
    let mesh = two_walls();
    assert_eq!(mesh.ray_hit(&ray([2.0, 0.0, 0.0], [0.0, 0.0, 1.0])), None);
    assert_eq!(mesh.ray_hit(&ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0])), None);
    assert_eq!(mesh.ray_hit(&ray([0.0, 0.0, 0.0], [1.0, 0.0, 0.0])), None);
}

#[test]
fn malformed_indices_are_skipped() {
    let mut mesh = two_walls();
    mesh.indices = vec![0, 1, 99, 0, 1, 2, 7];
    assert_eq!(
        mesh.ray_hit(&ray([0.0, 0.0, 0.0], [0.0, 0.0, 1.0])),
        Some(2.0)
    );
}

#[test]
fn a_transformed_ray_keeps_its_distances() {
    let world = Mat4::from_scale_rotation_translation(
        Vec3::new(2.0, 3.0, 0.5),
        Quat::from_rotation_y(0.7),
        Vec3::new(4.0, -1.0, 2.0),
    );
    let local_hit = Vec3::new(0.0, 0.0, 2.0);
    let world_ray = Ray {
        origin: Vec3::new(10.0, 4.0, -6.0),
        direction: (world.transform_point3(local_hit) - Vec3::new(10.0, 4.0, -6.0)).normalize(),
    };
    let local_ray = world_ray.transformed(world.inverse());

    let t = two_walls().ray_hit(&local_ray).unwrap();

    assert!((world_ray.at(t) - world.transform_point3(local_hit)).length() < 1e-3);
    assert!((world.transform_point3(local_ray.at(t)) - world_ray.at(t)).length() < 1e-3);
}
