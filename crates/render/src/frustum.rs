use concerto_mesh::Aabb;
use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

pub(crate) struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    pub(crate) fn from_clip_from_world(clip_from_world: Mat4) -> Self {
        let r0 = clip_from_world.row(0);
        let r1 = clip_from_world.row(1);
        let r2 = clip_from_world.row(2);
        let r3 = clip_from_world.row(3);
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2],
        }
    }

    pub(crate) fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|plane| {
            let normal = plane.xyz();
            let positive = Vec3::select(normal.cmpge(Vec3::ZERO), aabb.max, aabb.min);
            normal.dot(positive) + plane.w >= 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective() -> Frustum {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        Frustum::from_clip_from_world(proj * view)
    }

    fn cube(center: Vec3, half: f32) -> Aabb {
        Aabb {
            min: center - Vec3::splat(half),
            max: center + Vec3::splat(half),
        }
    }

    #[test]
    fn a_box_in_front_of_a_perspective_camera_intersects_its_frustum() {
        assert!(perspective().intersects_aabb(&cube(Vec3::new(0.0, 0.0, -10.0), 1.0)));
    }

    #[test]
    fn a_box_behind_a_perspective_camera_is_outside_its_frustum() {
        assert!(!perspective().intersects_aabb(&cube(Vec3::new(0.0, 0.0, 10.0), 1.0)));
    }

    #[test]
    fn a_box_beyond_the_far_plane_is_outside_its_frustum() {
        assert!(!perspective().intersects_aabb(&cube(Vec3::new(0.0, 0.0, -200.0), 1.0)));
    }

    #[test]
    fn a_box_straddling_a_side_plane_intersects_its_frustum() {
        assert!(perspective().intersects_aabb(&cube(Vec3::new(10.0, 0.0, -10.0), 1.0)));
    }

    #[test]
    fn an_orthographic_frustum_keeps_a_box_inside_its_extent_and_drops_one_outside() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::orthographic_rh(-5.0, 5.0, -5.0, 5.0, 0.1, 50.0);
        let frustum = Frustum::from_clip_from_world(proj * view);

        assert!(frustum.intersects_aabb(&cube(Vec3::new(3.0, -3.0, -20.0), 1.0)));
        assert!(!frustum.intersects_aabb(&cube(Vec3::new(8.0, 0.0, -20.0), 1.0)));
    }
}
