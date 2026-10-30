use std::f32::consts::{FRAC_PI_2, PI, TAU};

use glam::{Vec2, Vec3};

use crate::{mesh::Mesh, vertex::Vertex};

/// A rectangle on the XZ plane, `size.x` along X and `size.y` along Z, centred on the origin and facing up.
pub fn plane(size: Vec2) -> Mesh {
    let mut mesh = empty();
    push_quad(
        &mut mesh,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::X,
        size.x * 0.5,
        size.y * 0.5,
    );
    mesh
}

/// A box of full extents `size`, centred on the origin, with its own vertices on each face.
pub fn cuboid(size: Vec3) -> Mesh {
    let half = size * 0.5;
    let mut mesh = empty();
    for (normal, tangent) in [
        (Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_X, Vec3::Z),
        (Vec3::Y, Vec3::X),
        (Vec3::NEG_Y, Vec3::X),
        (Vec3::Z, Vec3::X),
        (Vec3::NEG_Z, Vec3::NEG_X),
    ] {
        let bitangent = normal.cross(tangent);
        push_quad(
            &mut mesh,
            normal * half.dot(normal.abs()),
            normal,
            tangent,
            half.dot(tangent.abs()),
            half.dot(bitangent.abs()),
        );
    }
    mesh
}

/// A sphere of `radius` centred on the origin, with at least 2 `rings` from pole to pole and 3 `segments` around Y.
pub fn uv_sphere(radius: f32, rings: u32, segments: u32) -> Mesh {
    let rings = rings.max(2);
    let profile: Vec<(Vec2, Vec2)> = (0..=rings)
        .map(|ring| {
            let normal = meridian(PI * ring as f32 / rings as f32);
            (normal * radius, normal)
        })
        .collect();
    revolve(&profile, segments)
}

/// A capsule along Y: a cylinder of `half_height` either side of the origin, capped by hemispheres of at least 1 ring each, with at least 3 `segments`; a `half_height` of 0 or less gives a sphere.
pub fn capsule(radius: f32, half_height: f32, rings: u32, segments: u32) -> Mesh {
    let rings = rings.max(1);
    if half_height <= 0.0 {
        return uv_sphere(radius, rings * 2, segments);
    }
    let cap = |offset: f32, height: f32| {
        (0..=rings).map(move |ring| {
            let normal = meridian(offset + FRAC_PI_2 * ring as f32 / rings as f32);
            (normal * radius + Vec2::new(0.0, height), normal)
        })
    };
    let profile: Vec<(Vec2, Vec2)> = cap(0.0, half_height)
        .chain(cap(FRAC_PI_2, -half_height))
        .collect();
    revolve(&profile, segments)
}

fn empty() -> Mesh {
    Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    }
}

fn meridian(polar_angle: f32) -> Vec2 {
    let (sin, cos) = polar_angle.sin_cos();
    Vec2::new(sin, cos)
}

fn vertex(position: Vec3, uv: Vec2, normal: Vec3, tangent: Vec3) -> Vertex {
    Vertex {
        pos_coords: position.into(),
        uv_coords: uv.into(),
        normal: normal.into(),
        tangent: tangent.into(),
        bitangent: normal.cross(tangent).into(),
        ..Default::default()
    }
}

fn push_quad(
    mesh: &mut Mesh,
    center: Vec3,
    normal: Vec3,
    tangent: Vec3,
    half_width: f32,
    half_height: f32,
) {
    let bitangent = normal.cross(tangent);
    let first = mesh.vertices.len() as u32;
    for corner in [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ] {
        let offset = tangent * (corner.x * 2.0 - 1.0) * half_width
            + bitangent * (corner.y * 2.0 - 1.0) * half_height;
        let uv = Vec2::new(corner.x, 1.0 - corner.y);
        mesh.vertices
            .push(vertex(center + offset, uv, normal, tangent));
    }
    mesh.indices
        .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
}

fn revolve(profile: &[(Vec2, Vec2)], segments: u32) -> Mesh {
    let segments = segments.max(3);
    let rows = profile.len() as u32;
    let columns = segments + 1;
    let mut travelled = vec![0.0; profile.len()];
    for row in 1..profile.len() {
        travelled[row] = travelled[row - 1] + profile[row].0.distance(profile[row - 1].0);
    }
    let length = travelled[profile.len() - 1];
    let mut mesh = empty();
    for (row, (point, normal)) in profile.iter().enumerate() {
        for column in 0..columns {
            let (sin, cos) = (TAU * (column % segments) as f32 / segments as f32).sin_cos();
            let around = Vec3::new(sin, 0.0, cos);
            mesh.vertices.push(vertex(
                around * point.x + Vec3::Y * point.y,
                Vec2::new(column as f32 / segments as f32, travelled[row] / length),
                around * normal.x + Vec3::Y * normal.y,
                Vec3::new(cos, 0.0, -sin),
            ));
        }
    }
    for row in 0..rows - 1 {
        for column in 0..segments {
            let top = row * columns + column;
            let bottom = top + columns;
            if row != rows - 2 {
                mesh.indices.extend([top, bottom, bottom + 1]);
            }
            if row != 0 {
                mesh.indices.extend([top, bottom + 1, top + 1]);
            }
        }
    }
    mesh
}
