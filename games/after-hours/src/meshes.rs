//! Procedural meshes. Everything in the store except the mannequins is built
//! here, so the game downloads no environment art.
use concerto::render::assets::{mesh::Mesh, vertex::Vertex};
use glam::{Vec2, Vec3};

fn vertex(pos: Vec3, normal: Vec3, uv: Vec2) -> Vertex {
    Vertex {
        pos_coords: pos.to_array(),
        normal: normal.to_array(),
        uv_coords: uv.to_array(),
        ..Vertex::default()
    }
}

fn finish(vertices: Vec<Vertex>, indices: Vec<u32>) -> Mesh {
    let mut mesh = Mesh { vertices, indices };
    mesh.compute_tangents();
    mesh
}

/// An axis-aligned box with the given half-extents, centered on the origin.
/// UVs are in world units so tiled textures keep a constant density.
pub fn cuboid(half: Vec3) -> Mesh {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    // (normal, u axis, v axis) per face; corners are normal ± u ± v.
    let faces = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];
    for (normal, u, v) in faces {
        let base = vertices.len() as u32;
        let center = normal * half;
        let u_len = (u * half).length();
        let v_len = (v * half).length();
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let pos = center + u * half * su + v * half * sv;
            vertices.push(vertex(
                pos,
                normal,
                Vec2::new(su * u_len, -sv * v_len) * 0.5,
            ));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    finish(vertices, indices)
}

/// A capped cylinder along Y, from `y = 0` to `y = height`.
pub fn cylinder(radius: f32, height: f32, segments: u32) -> Mesh {
    frustum(radius, radius, height, segments)
}

/// A capped cone frustum along Y, from `y = 0` (radius `bottom`) to
/// `y = height` (radius `top`).
pub fn frustum(bottom: f32, top: f32, height: f32, segments: u32) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let slope = (bottom - top) / height;
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        let normal = Vec3::new(c, slope, s).normalize();
        let u = i as f32 / segments as f32 * std::f32::consts::TAU * bottom;
        vertices.push(vertex(
            Vec3::new(c * bottom, 0.0, s * bottom),
            normal,
            Vec2::new(u, 0.0),
        ));
        vertices.push(vertex(
            Vec3::new(c * top, height, s * top),
            normal,
            Vec2::new(u, -height),
        ));
    }
    for i in 0..segments {
        let b = i * 2;
        indices.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
    }
    for (y, radius, normal) in [(height, top, Vec3::Y), (0.0, bottom, Vec3::NEG_Y)] {
        if radius <= 0.0 {
            continue;
        }
        let center = vertices.len() as u32;
        vertices.push(vertex(Vec3::new(0.0, y, 0.0), normal, Vec2::ZERO));
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * std::f32::consts::TAU;
            let pos = Vec3::new(a.cos() * radius, y, a.sin() * radius);
            vertices.push(vertex(pos, normal, Vec2::new(pos.x, pos.z)));
        }
        for i in 1..=segments {
            if normal.y > 0.0 {
                indices.extend_from_slice(&[center, center + i + 1, center + i]);
            } else {
                indices.extend_from_slice(&[center, center + i, center + i + 1]);
            }
        }
    }
    finish(vertices, indices)
}

/// Accumulates many axis-aligned boxes and quads into one mesh, so a whole
/// floor of shelving draws in a handful of calls. UVs are world-space meters
/// (scaled per material with `uv_scale`).
#[derive(Default)]
pub struct MeshBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A box centered at `center` with half-extents `half`. Faces listed in
    /// `skip` (as outward normals) are left out, e.g. bottoms nobody sees.
    pub fn cuboid(&mut self, center: Vec3, half: Vec3, skip: &[Vec3]) -> &mut Self {
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        for (normal, u, v) in faces {
            if skip.contains(&normal) {
                continue;
            }
            let face_center = center + normal * half;
            let du = u * half;
            let dv = v * half;
            self.quad(face_center, du, dv, normal);
        }
        self
    }

    /// A rectangle centered at `center` spanning `±du` and `±dv`, facing
    /// `normal` (`du × dv` must point along `normal`).
    pub fn quad(&mut self, center: Vec3, du: Vec3, dv: Vec3, normal: Vec3) -> &mut Self {
        let base = self.vertices.len() as u32;
        // World-space UVs: project onto the face's own axes so textures keep
        // a constant density and line up across neighbouring boxes.
        let u_axis = du.normalize_or_zero();
        let v_axis = dv.normalize_or_zero();
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let pos = center + du * su + dv * sv;
            let uv = Vec2::new(pos.dot(u_axis), -pos.dot(v_axis));
            self.vertices.push(vertex(pos, normal, uv));
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self
    }

    pub fn build(self) -> Mesh {
        finish(self.vertices, self.indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid(mesh: &Mesh) {
        assert_eq!(mesh.indices.len() % 3, 0);
        let count = mesh.vertices.len() as u32;
        assert!(mesh.indices.iter().all(|&i| i < count));
    }

    /// Every triangle of a closed convex mesh centered on the origin must wind
    /// counter-clockwise seen from outside, or back-face culling hides it.
    fn assert_outward(mesh: &Mesh, center: Vec3) {
        for tri in mesh.indices.chunks(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]]
                .map(|i| Vec3::from_array(mesh.vertices[i as usize].pos_coords));
            let n = (b - a).cross(c - a);
            if n.length_squared() < 1e-10 {
                continue;
            }
            let centroid = (a + b + c) / 3.0;
            assert!(n.dot(centroid - center) > 0.0, "inward-facing triangle");
        }
    }

    #[test]
    fn shapes_are_valid_and_face_outward() {
        let boxed = cuboid(Vec3::new(1.0, 2.0, 3.0));
        assert_valid(&boxed);
        assert_outward(&boxed, Vec3::ZERO);

        let column = cylinder(0.5, 2.0, 12);
        assert_valid(&column);
        assert_outward(&column, Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn builder_boxes_face_outward() {
        let mut builder = MeshBuilder::default();
        builder.cuboid(Vec3::new(3.0, 1.0, -2.0), Vec3::new(0.5, 1.0, 2.0), &[]);
        let mesh = builder.build();
        assert_valid(&mesh);
        assert_outward(&mesh, Vec3::new(3.0, 1.0, -2.0));
    }
}
