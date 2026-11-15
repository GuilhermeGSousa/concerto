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

    pub(crate) fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|plane| {
            let normal = plane.xyz();
            normal.dot(center) + plane.w >= -radius * normal.length()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perspective() -> Frustum {
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective(
            std::f32::consts::FRAC_PI_2,
            1.0,
            0.1,
            100.0,
        );
        Frustum::from_clip_from_world(proj * view)
    }

    fn cube(center: Vec3, half: f32) -> Aabb {
        Aabb {
            min: center - Vec3::splat(half),
            max: center + Vec3::splat(half),
        }
    }

    fn orthographic() -> Frustum {
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::orthographic(-5.0, 5.0, -5.0, 5.0, 0.1, 50.0);
        Frustum::from_clip_from_world(proj * view)
    }

    struct Case {
        case: &'static str,
        frustum: Frustum,
        center: Vec3,
        intersects: bool,
    }

    #[test]
    fn a_box_or_sphere_intersects_a_frustum_when_any_part_of_it_is_inside() {
        let cases = [
            Case {
                case: "in front of a perspective camera",
                frustum: perspective(),
                center: Vec3::new(0.0, 0.0, -10.0),
                intersects: true,
            },
            Case {
                case: "behind a perspective camera",
                frustum: perspective(),
                center: Vec3::new(0.0, 0.0, 10.0),
                intersects: false,
            },
            Case {
                case: "beyond the far plane",
                frustum: perspective(),
                center: Vec3::new(0.0, 0.0, -200.0),
                intersects: false,
            },
            Case {
                case: "centred just past a side plane",
                frustum: perspective(),
                center: Vec3::new(11.2, 0.0, -10.0),
                intersects: true,
            },
            Case {
                case: "outside an orthographic extent",
                frustum: orthographic(),
                center: Vec3::new(8.0, 0.0, -20.0),
                intersects: false,
            },
        ];
        for case in &cases {
            assert_eq!(
                case.frustum.intersects_aabb(&cube(case.center, 1.0)),
                case.intersects,
                "{}",
                case.case
            );
            assert_eq!(
                case.frustum.intersects_sphere(case.center, 1.0),
                case.intersects,
                "a sphere {}",
                case.case
            );
        }
    }
}
