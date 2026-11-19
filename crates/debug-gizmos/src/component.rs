use std::collections::HashSet;

use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Query,
    component::scene::SceneEntityRef,
    entity::hierarchy::ChildOf,
    system::input::{ReadOnlySystemInput, SystemInputData},
};
use concerto_foundation::transform::GlobalTransform;
use concerto_mesh::{Aabb, SkeletonComponent};
use concerto_render::components::{Camera, Light, light::LightType};
use glam::Vec3;

use crate::DebugGizmos;

/// The length, in metres, drawn for anything unbounded.
pub const REACH: f32 = 5.0;

/// Draws a component in the editor's viewport while its entity is selected.
pub trait ComponentGizmo: Send + Sync + 'static {
    type Component: Component;
    type Input: ReadOnlySystemInput + 'static;
    /// The kind's name in the viewport's Gizmos menu and the `gizmos` tool.
    const NAME: &'static str;

    fn draw(
        entity: Entity,
        component: &Self::Component,
        transform: &GlobalTransform,
        input: &SystemInputData<'_, '_, Self::Input>,
        gizmos: &mut DebugGizmos,
    );
}

/// Draws an entity's [`Aabb`] as a box carried by its transform.
pub struct BoundsGizmo;

/// Draws a skeleton's bones, each joined to its parent bone.
pub struct SkeletonGizmo;

/// Draws a light's reach: a sphere, a cone, or a direction.
pub struct LightGizmo;

/// Draws a camera's frustum, cut at [`REACH`].
pub struct CameraGizmo;

const BOUNDS_COLOR: Color = Color::srgba(0.569, 0.518, 0.851, 1.0);
const LIGHT_COLOR: Color = Color::srgba(1.0, 0.9, 0.4, 1.0);
const CAMERA_COLOR: Color = Color::srgba(0.85, 0.85, 0.85, 1.0);
const BONE_COLOR: Color = Color::srgba(1.0, 0.8, 0.2, 1.0);
const JOINT_SIZE: f32 = 0.02;
const WIDEST_CONE: f32 = 89.0 * std::f32::consts::PI / 180.0;

impl ComponentGizmo for BoundsGizmo {
    type Component = Aabb;
    type Input = ();
    const NAME: &'static str = "Bounds";

    fn draw(
        _entity: Entity,
        aabb: &Aabb,
        transform: &GlobalTransform,
        _input: &(),
        gizmos: &mut DebugGizmos,
    ) {
        let matrix = transform.matrix();
        let corners: [Vec3; 8] = std::array::from_fn(|index| {
            let pick = |bit: usize, min: f32, max: f32| if index & bit == 0 { min } else { max };
            matrix.transform_point3(Vec3::new(
                pick(1, aabb.min.x, aabb.max.x),
                pick(2, aabb.min.y, aabb.max.y),
                pick(4, aabb.min.z, aabb.max.z),
            ))
        });
        for from in 0..8_usize {
            for axis in [1, 2, 4] {
                if from & axis == 0 {
                    gizmos.line(corners[from], corners[from | axis], BOUNDS_COLOR);
                }
            }
        }
    }
}

impl ComponentGizmo for SkeletonGizmo {
    type Component = SkeletonComponent;
    type Input = Query<'static, 'static, (&'static GlobalTransform, Option<&'static ChildOf>)>;
    const NAME: &'static str = "Skeleton";

    fn draw(
        _entity: Entity,
        skeleton: &SkeletonComponent,
        _transform: &GlobalTransform,
        bones: &SystemInputData<'_, '_, Self::Input>,
        gizmos: &mut DebugGizmos,
    ) {
        if skeleton.root().is_none() {
            return;
        }
        let entities: Vec<Entity> = skeleton
            .bones()
            .iter()
            .filter_map(|bone| SceneEntityRef::entity(*bone))
            .collect();
        let members: HashSet<Entity> = entities.iter().copied().collect();
        for bone in entities {
            let Some((global, parent)) = bones.get_entity(bone) else {
                continue;
            };
            let joint = global.translation();
            gizmos.sphere(joint, JOINT_SIZE, BONE_COLOR);
            let Some(parent) = parent
                .map(ChildOf::parent)
                .filter(|parent| members.contains(parent))
            else {
                continue;
            };
            if let Some((parent_global, _)) = bones.get_entity(parent) {
                gizmos.line(parent_global.translation(), joint, BONE_COLOR);
            }
        }
    }
}

impl ComponentGizmo for LightGizmo {
    type Component = Light;
    type Input = ();
    const NAME: &'static str = "Light";

    fn draw(
        _entity: Entity,
        light: &Light,
        transform: &GlobalTransform,
        _input: &(),
        gizmos: &mut DebugGizmos,
    ) {
        let reach = if light.range > 0.0 {
            light.range
        } else {
            REACH
        };
        let origin = transform.translation();
        let rotation = transform.rotation();
        let forward = rotation * Vec3::NEG_Z;
        match light.light_type {
            LightType::Point => gizmos.sphere(origin, reach, LIGHT_COLOR),
            LightType::Spot { cone_angle } => {
                let center = origin + forward * reach;
                let radius = reach * cone_angle.min(WIDEST_CONE).tan();
                gizmos.circle(center, forward, radius, LIGHT_COLOR);
                for side in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y] {
                    gizmos.line(origin, center + rotation * side * radius, LIGHT_COLOR);
                }
            }
            LightType::Directional => gizmos.arrow(origin, origin + forward, LIGHT_COLOR),
        }
    }
}

impl ComponentGizmo for CameraGizmo {
    type Component = Camera;
    type Input = ();
    const NAME: &'static str = "Camera";

    fn draw(
        _entity: Entity,
        camera: &Camera,
        transform: &GlobalTransform,
        _input: &(),
        gizmos: &mut DebugGizmos,
    ) {
        let origin = transform.translation();
        let rotation = transform.rotation();
        let slope = (camera.fovy * 0.5).tan();
        let rect = |depth: f32| -> [Vec3; 4] {
            let up = depth * slope;
            let right = up * camera.aspect;
            [
                Vec3::new(-right, -up, -depth),
                Vec3::new(right, -up, -depth),
                Vec3::new(right, up, -depth),
                Vec3::new(-right, up, -depth),
            ]
            .map(|corner| origin + rotation * corner)
        };
        let near = rect(camera.znear);
        let far = rect(camera.zfar.min(REACH));
        for index in 0..4 {
            let next = (index + 1) % 4;
            gizmos.line(near[index], near[next], CAMERA_COLOR);
            gizmos.line(far[index], far[next], CAMERA_COLOR);
            gizmos.line(near[index], far[index], CAMERA_COLOR);
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{
        IntoSystem, System, World, component::scene::SceneEntityRef,
        system::input::StaticSystemInput,
    };
    use glam::{Mat4, Quat};

    use super::*;
    use crate::GizmoStorage;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Segment {
        start: Vec3,
        end: Vec3,
    }

    fn draw_all<G: ComponentGizmo>(
        drawn: Query<(Entity, &G::Component, &GlobalTransform)>,
        input: StaticSystemInput<G::Input>,
        mut gizmos: DebugGizmos,
    ) {
        for (entity, component, transform) in drawn.iter() {
            G::draw(entity, component, transform, &input, &mut gizmos);
        }
    }

    fn drawn<G: ComponentGizmo>(world: &mut World) -> Vec<Segment> {
        world.insert_resource(GizmoStorage::default());
        let mut system = draw_all::<G>.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
        world
            .get_resource::<GizmoStorage>()
            .unwrap()
            .vertices
            .chunks(2)
            .map(|pair| Segment {
                start: Vec3::from_array(pair[0].position),
                end: Vec3::from_array(pair[1].position),
            })
            .collect()
    }

    fn one<G: ComponentGizmo>(component: G::Component, at: Mat4) -> Vec<Segment> {
        let mut world = World::default();
        world.spawn((component, GlobalTransform::new(at)));
        drawn::<G>(&mut world)
    }

    fn skeleton(rooted: bool) -> Vec<Segment> {
        let mut world = World::default();
        let owner = world.spawn(GlobalTransform::new(Mat4::IDENTITY));
        let mut bone = |at: Vec3, parent: Entity| {
            world.spawn((
                GlobalTransform::new(Mat4::from_translation(at)),
                ChildOf::new(parent),
            ))
        };
        let hip = bone(HIP, owner);
        let knee = bone(KNEE, hip);
        let foot = bone(FOOT, knee);
        world.spawn((
            SkeletonComponent::new(
                Default::default(),
                [hip, knee, foot].map(SceneEntityRef::Entity).to_vec(),
                rooted.then_some(SceneEntityRef::Entity(owner)),
            ),
            GlobalTransform::new(Mat4::IDENTITY),
        ));
        drawn::<SkeletonGizmo>(&mut world)
    }

    const HIP: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    const KNEE: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    const FOOT: Vec3 = Vec3::new(0.0, 0.0, 0.1);

    fn light(light_type: LightType, range: f32) -> Light {
        Light {
            light_type,
            range,
            ..Light::point_light()
        }
    }

    fn camera(zfar: f32) -> Camera {
        Camera {
            fovy: std::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            znear: 0.1,
            zfar,
            ..Camera::default()
        }
    }

    enum Expected {
        Nothing,
        Corners(Vec<Vec3>),
        Joins {
            bones: Vec<Segment>,
            never: Vec3,
        },
        Reach {
            from: Vec3,
            along: Vec3,
            depth: f32,
            farthest: f32,
        },
    }

    struct Row {
        name: &'static str,
        drawn: Vec<Segment>,
        expected: Expected,
    }

    fn vertices(segments: &[Segment]) -> impl Iterator<Item = Vec3> + '_ {
        segments
            .iter()
            .flat_map(|segment| [segment.start, segment.end])
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3 * b.abs().max(1.0)
    }

    #[test]
    fn each_engine_gizmo_draws_its_component() {
        let turned = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let at = Vec3::new(3.0, 1.0, -2.0);
        let placed = Mat4::from_rotation_translation(turned, at);
        let forward = turned * Vec3::NEG_Z;
        let aabb = Aabb {
            min: Vec3::new(-1.0, 0.0, -2.0),
            max: Vec3::new(1.0, 2.0, 2.0),
        };
        let cone = 0.5_f32;
        let rows = [
            Row {
                name: "bounds",
                drawn: one::<BoundsGizmo>(aabb, placed),
                expected: Expected::Corners(
                    [aabb.min.x, aabb.max.x]
                        .into_iter()
                        .flat_map(|x| [aabb.min.y, aabb.max.y].map(|y| Vec3::new(x, y, 0.0)))
                        .flat_map(|xy| [aabb.min.z, aabb.max.z].map(|z| xy.with_z(z)))
                        .map(|corner| placed.transform_point3(corner))
                        .collect(),
                ),
            },
            Row {
                name: "skeleton",
                drawn: skeleton(true),
                expected: Expected::Joins {
                    bones: vec![
                        Segment {
                            start: HIP,
                            end: KNEE,
                        },
                        Segment {
                            start: KNEE,
                            end: FOOT,
                        },
                    ],
                    never: Vec3::ZERO,
                },
            },
            Row {
                name: "rootless skeleton",
                drawn: skeleton(false),
                expected: Expected::Nothing,
            },
            Row {
                name: "point light with a range",
                drawn: one::<LightGizmo>(light(LightType::Point, 2.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: 2.0,
                    farthest: 2.0,
                },
            },
            Row {
                name: "point light without a range",
                drawn: one::<LightGizmo>(light(LightType::Point, 0.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: REACH,
                    farthest: REACH,
                },
            },
            Row {
                name: "spot light",
                drawn: one::<LightGizmo>(light(LightType::Spot { cone_angle: cone }, 0.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: REACH,
                    farthest: REACH / cone.cos(),
                },
            },
            Row {
                name: "spot light wider than a right angle",
                drawn: one::<LightGizmo>(light(LightType::Spot { cone_angle: 45.0 }, 0.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: REACH,
                    farthest: REACH / 89.0_f32.to_radians().cos(),
                },
            },
            Row {
                name: "directional light",
                drawn: one::<LightGizmo>(light(LightType::Directional, 3.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: 1.0,
                    farthest: 1.0,
                },
            },
            Row {
                name: "camera beyond reach",
                drawn: one::<CameraGizmo>(camera(100.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: REACH,
                    farthest: REACH * 3.0_f32.sqrt(),
                },
            },
            Row {
                name: "camera within reach",
                drawn: one::<CameraGizmo>(camera(2.0), placed),
                expected: Expected::Reach {
                    from: at,
                    along: forward,
                    depth: 2.0,
                    farthest: 2.0 * 3.0_f32.sqrt(),
                },
            },
        ];

        for row in rows {
            let name = row.name;
            assert_eq!(
                row.drawn.is_empty(),
                matches!(row.expected, Expected::Nothing),
                "{name}: drew {} segments",
                row.drawn.len()
            );
            match row.expected {
                Expected::Nothing => {}
                Expected::Corners(corners) => {
                    for vertex in vertices(&row.drawn) {
                        assert!(
                            corners.iter().any(|corner| corner.distance(vertex) < 1e-4),
                            "{name}: {vertex} is not a corner"
                        );
                    }
                    for corner in corners {
                        assert!(
                            vertices(&row.drawn).any(|vertex| corner.distance(vertex) < 1e-4),
                            "{name}: corner {corner} is missing"
                        );
                    }
                }
                Expected::Joins { bones, never } => {
                    for bone in bones {
                        assert!(row.drawn.contains(&bone), "{name}: {bone:?} is missing");
                    }
                    assert!(
                        !vertices(&row.drawn).any(|vertex| vertex == never),
                        "{name}: a line reaches {never}, which is not a bone"
                    );
                }
                Expected::Reach {
                    from,
                    along,
                    depth,
                    farthest,
                } => {
                    let deepest = vertices(&row.drawn)
                        .map(|vertex| (vertex - from).dot(along))
                        .fold(f32::MIN, f32::max);
                    let reached = vertices(&row.drawn)
                        .map(|vertex| vertex.distance(from))
                        .fold(0.0, f32::max);
                    assert!(
                        close(deepest, depth),
                        "{name}: reaches {deepest} deep, not {depth}"
                    );
                    assert!(
                        close(reached, farthest),
                        "{name}: reaches {reached} away, not {farthest}"
                    );
                }
            }
        }
    }
}
