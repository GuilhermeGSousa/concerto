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

struct Case {
    case: &'static str,
    ray: Ray,
    distance: Option<f32>,
}

fn assert_distance(case: &Case, distance: Option<f32>) {
    match (distance, case.distance) {
        (Some(actual), Some(expected)) => assert!(
            (actual - expected).abs() < 1e-5,
            "{}: {actual} != {expected}",
            case.case
        ),
        (actual, expected) => assert_eq!(actual, expected, "{}", case.case),
    }
}

#[test]
fn a_box_is_entered_where_the_ray_first_touches_it() {
    let cases = [
        Case {
            case: "straight on",
            ray: ray([0.0, 0.0, -5.0], [0.0, 0.0, 1.0]),
            distance: Some(4.0),
        },
        Case {
            case: "diagonal",
            ray: ray([-3.0, 0.0, -3.0], [1.0, 0.0, 1.0]),
            distance: Some(2.0),
        },
        Case {
            case: "starting inside",
            ray: ray([0.2, 0.1, 0.0], [1.0, 0.0, 0.0]),
            distance: Some(0.0),
        },
        Case {
            case: "along a face",
            ray: ray([1.0, 0.0, -5.0], [0.0, 0.0, 1.0]),
            distance: Some(4.0),
        },
        Case {
            case: "parallel to the faces and outside them",
            ray: ray([3.0, 0.0, -5.0], [0.0, 0.0, 1.0]),
            distance: None,
        },
        Case {
            case: "the box is behind the origin",
            ray: ray([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]),
            distance: None,
        },
        Case {
            case: "passing above",
            ray: ray([0.0, 5.0, -5.0], [0.0, 0.1, 1.0]),
            distance: None,
        },
    ];
    for case in &cases {
        assert_distance(case, unit_box().ray_entry(&case.ray));
    }
}

#[test]
fn a_mesh_is_hit_at_its_nearest_triangle_from_either_side() {
    let cases = [
        Case {
            case: "the near wall, whatever the index order",
            ray: ray([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            distance: Some(2.0),
        },
        Case {
            case: "from behind, the far wall on its back face",
            ray: ray([0.0, 0.0, 9.0], [0.0, 0.0, -1.0]),
            distance: Some(4.0),
        },
        Case {
            case: "outside the triangles' edges",
            ray: ray([2.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            distance: None,
        },
        Case {
            case: "pointing away",
            ray: ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            distance: None,
        },
        Case {
            case: "parallel to the walls",
            ray: ray([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            distance: None,
        },
    ];
    for case in &cases {
        assert_distance(case, two_walls().ray_hit(&case.ray));
    }

    let mut malformed = two_walls();
    malformed.indices = vec![0, 1, 99, 0, 1, 2, 7];
    assert_distance(&cases[0], malformed.ray_hit(&cases[0].ray));
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
