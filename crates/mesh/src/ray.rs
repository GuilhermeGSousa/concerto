use glam::{Mat4, Vec3};

use crate::mesh::{Aabb, Mesh};

/// A half-line from `origin` along `direction`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }

    /// The same ray in another space. The direction is not renormalized, so a
    /// distance along one is the same distance along the other.
    pub fn transformed(&self, matrix: Mat4) -> Ray {
        Ray {
            origin: matrix.transform_point3(self.origin),
            direction: matrix.transform_vector3(self.direction),
        }
    }
}

impl Aabb {
    /// Where the ray enters the box: zero when it starts inside, `None` on a miss.
    pub fn ray_entry(&self, ray: &Ray) -> Option<f32> {
        let mut near = 0.0_f32;
        let mut far = f32::INFINITY;
        for axis in 0..3 {
            let (origin, direction) = (ray.origin[axis], ray.direction[axis]);
            let (min, max) = (self.min[axis], self.max[axis]);
            if direction.abs() < f32::EPSILON {
                if origin < min || origin > max {
                    return None;
                }
                continue;
            }
            let a = (min - origin) / direction;
            let b = (max - origin) / direction;
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
        Some(near)
    }
}

impl Mesh {
    /// The nearest triangle the ray hits, from either side.
    pub fn ray_hit(&self, ray: &Ray) -> Option<f32> {
        let position = |index: u32| {
            self.vertices
                .get(index as usize)
                .map(|vertex| Vec3::from(vertex.pos_coords))
        };
        let mut nearest: Option<f32> = None;
        for triangle in self.indices.chunks_exact(3) {
            let (Some(a), Some(b), Some(c)) = (
                position(triangle[0]),
                position(triangle[1]),
                position(triangle[2]),
            ) else {
                continue;
            };
            if let Some(t) = triangle_hit(ray, a, b, c) {
                if nearest.is_none_or(|best| t < best) {
                    nearest = Some(t);
                }
            }
        }
        nearest
    }
}

fn triangle_hit(ray: &Ray, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let edge_ab = b - a;
    let edge_ac = c - a;
    let p = ray.direction.cross(edge_ac);
    let determinant = edge_ab.dot(p);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from_a = ray.origin - a;
    let u = from_a.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = from_a.cross(edge_ab);
    let v = ray.direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge_ac.dot(q) * inverse;
    (t > 0.0).then_some(t)
}
