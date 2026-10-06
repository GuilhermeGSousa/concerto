//! Selecting scene entities by clicking them in the viewport.
use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_debug_gizmos::DebugGizmos;
use concerto_ecs::{Entity, Query, Res, ResMut, Resource, entity::hierarchy::Children, signal::On};
use concerto_foundation::{
    assets::asset_store::AssetStore,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{Aabb, Mesh, MeshComponent, Ray};
use concerto_render::components::camera::Camera;
use concerto_ui::{
    interaction::{UIClick, UIPointerDown},
    node::{UIBox, UILayout},
    theme::UITheme,
};
use concerto_window::input::MouseButton;
use glam::{Mat4, Vec2, Vec3};

use crate::{
    gizmo::GizmoState,
    hierarchy::HierarchyState,
    scene::SceneRoot,
    selection::Selection,
    viewport::{EditorCamera, FlyCamera, subtree},
};

/// How far the pointer may travel between press and release and still count as a click.
const CLICK_SLOP: f32 = 4.0;

#[derive(Resource, Default)]
pub(crate) struct PickPress(Option<Vec2>);

pub struct PickingPlugin;

impl Plugin for PickingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PickPress::default());
        app.add_system(LateUpdate, draw_selection_bounds);
    }
}

/// The world-space ray under `pointer`, or `None` when it is outside `rect`.
pub fn viewport_ray(pointer: Vec2, rect: UIBox, projection: Mat4, camera: Mat4) -> Option<Ray> {
    if !rect.contains(pointer) {
        return None;
    }
    ray_through(pointer, rect, projection, camera)
}

/// [`viewport_ray`] without the bounds check, for a drag that has left the viewport.
pub fn ray_through(pointer: Vec2, rect: UIBox, projection: Mat4, camera: Mat4) -> Option<Ray> {
    if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
        return None;
    }
    let uv = (pointer - rect.min) / rect.size;
    let ndc = Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let unproject = camera * projection.inverse();
    let near = unproject.project_point3(ndc.extend(0.0));
    let far = unproject.project_point3(ndc.extend(1.0));
    let direction = (far - near).try_normalize()?;
    Some(Ray {
        origin: near,
        direction,
    })
}

/// Where a world point appears in the viewport, or `None` when it is behind the camera.
pub fn world_to_viewport(point: Vec3, rect: UIBox, view_proj: Mat4) -> Option<Vec2> {
    let clip = view_proj * point.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate().truncate() / clip.w;
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    Some(rect.min + uv * rect.size)
}

/// The entity whose mesh `ray` hits first.
pub fn pick<'a>(
    ray: &Ray,
    meshes: &AssetStore<Mesh>,
    nodes: impl Iterator<Item = (Entity, &'a MeshComponent, &'a Aabb, &'a GlobalTransform)>,
) -> Option<Entity> {
    let mut candidates: Vec<_> = nodes
        .filter_map(|(entity, mesh, aabb, transform)| {
            let matrix = transform.matrix();
            if matrix.determinant().abs() < 1e-12 {
                return None;
            }
            let local = ray.transformed(matrix.inverse());
            let entry = aabb.ray_entry(&local)?;
            Some((entry, entity, local, mesh))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut nearest: Option<(f32, Entity)> = None;
    for (entry, entity, local, mesh) in candidates {
        if nearest.is_some_and(|(distance, _)| entry > distance) {
            break;
        }
        if let Some(distance) = meshes
            .get(&mesh.handle)
            .and_then(|mesh| mesh.ray_hit(&local))
            && nearest.is_none_or(|(best, _)| distance < best)
        {
            nearest = Some((distance, entity));
        }
    }
    nearest.map(|(_, entity)| entity)
}

pub(crate) fn press_viewport(on: On<UIPointerDown>, mut press: ResMut<PickPress>) {
    if on.signal().button == MouseButton::Left {
        press.0 = Some(on.signal().position);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn click_viewport(
    on: On<UIClick>,
    press: Res<PickPress>,
    fly: Res<FlyCamera>,
    gizmo: Res<GizmoState>,
    layouts: Query<&UILayout>,
    cameras: Query<(&EditorCamera, &Camera, &Transform)>,
    roots: Query<(Entity, &SceneRoot)>,
    children: Query<&Children>,
    nodes: Query<(Entity, &MeshComponent, &Aabb, &GlobalTransform)>,
    meshes: Res<AssetStore<Mesh>>,
    mut selection: ResMut<Selection>,
    mut hierarchy: ResMut<HierarchyState>,
) {
    let position = on.signal().position;
    if on.signal().button != MouseButton::Left
        || fly.navigating()
        || gizmo.captured()
        || press
            .0
            .is_none_or(|pressed| pressed.distance(position) > CLICK_SLOP)
    {
        return;
    }
    let Some(layout) = layouts.get_entity(on.entity()) else {
        return;
    };
    let Some((_, camera, transform)) = cameras.iter().next() else {
        return;
    };
    let Some(ray) = viewport_ray(
        position,
        layout.rect,
        camera.build_projection_matrix(),
        transform.compute_matrix(),
    ) else {
        return;
    };

    let mut scene = std::collections::HashSet::new();
    for (root, _) in roots.iter() {
        scene.extend(subtree(root, &children));
    }
    let hit = pick(
        &ray,
        &meshes,
        nodes.iter().filter(|(entity, ..)| scene.contains(entity)),
    );
    match hit {
        Some(entity) => {
            selection.select_entity(entity);
            hierarchy.reveal_entity(entity);
        }
        None => selection.clear(),
    }
}

/// The corners of the box that marks `entity` as selected.
fn selection_corners(
    entity: Entity,
    roots: &Query<&SceneRoot>,
    bounds: &Query<(&Aabb, &GlobalTransform)>,
    children: &Query<&Children>,
) -> Option<[Vec3; 8]> {
    if roots.get_entity(entity).is_some() {
        return None;
    }
    if let Some((aabb, transform)) = bounds.get_entity(entity) {
        return Some(corners(*aabb).map(|corner| transform.matrix().transform_point3(corner)));
    }
    subtree(entity, children)
        .into_iter()
        .filter_map(|descendant| bounds.get_entity(descendant))
        .map(|(aabb, transform)| aabb.transformed(transform.matrix()))
        .reduce(|a, b| Aabb {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        })
        .map(corners)
}

fn corners(aabb: Aabb) -> [Vec3; 8] {
    std::array::from_fn(|index| {
        Vec3::new(
            if index & 1 == 0 {
                aabb.min.x
            } else {
                aabb.max.x
            },
            if index & 2 == 0 {
                aabb.min.y
            } else {
                aabb.max.y
            },
            if index & 4 == 0 {
                aabb.min.z
            } else {
                aabb.max.z
            },
        )
    })
}

fn draw_selection_bounds(
    selection: Res<Selection>,
    roots: Query<&SceneRoot>,
    bounds: Query<(&Aabb, &GlobalTransform)>,
    children: Query<&Children>,
    theme: Res<UITheme>,
    mut gizmos: DebugGizmos,
) {
    let Some(corners) = selection
        .entity()
        .and_then(|entity| selection_corners(entity, &roots, &bounds, &children))
    else {
        return;
    };
    for from in 0..8_usize {
        for axis in [1, 2, 4] {
            if from & axis == 0 {
                gizmos.line(corners[from], corners[from | axis], theme.accent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_debug_gizmos::GizmoStorage;
    use concerto_ecs::{
        IntoSystem, System, World, entity::hierarchy::ChildOf, signal::listener::IntoListener,
    };
    use concerto_foundation::assets::{AssetId, handle::AssetHandle};
    use concerto_mesh::Vertex;
    use glam::Quat;

    fn cube_mesh() -> Mesh {
        let vertices = (0..8)
            .map(|index| Vertex {
                pos_coords: [
                    if index & 1 == 0 { -1.0 } else { 1.0 },
                    if index & 2 == 0 { -1.0 } else { 1.0 },
                    if index & 4 == 0 { -1.0 } else { 1.0 },
                ],
                ..Default::default()
            })
            .collect();
        let indices = vec![
            0, 1, 3, 0, 3, 2, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6,
            4, 1, 3, 7, 1, 7, 5,
        ];
        Mesh { vertices, indices }
    }

    struct Fixture {
        world: World,
        root: Entity,
        mesh: AssetId,
    }

    fn fixture() -> Fixture {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        let mesh = AssetId::new();
        let mut meshes = AssetStore::<Mesh>::default();
        meshes.insert(mesh, cube_mesh());
        world.insert_resource(meshes);
        world.insert_resource(Selection::default());
        world.insert_resource(HierarchyState::default());
        world.insert_resource(FlyCamera::default());
        world.insert_resource(PickPress::default());
        world.insert_resource(GizmoState::default());
        world.insert_resource(UITheme::default());
        world.insert_resource(GizmoStorage::default());
        let root = world.spawn(SceneRoot {
            asset_id: AssetId::new(),
            address: String::new(),
        });
        Fixture { world, root, mesh }
    }

    impl Fixture {
        fn cube(&mut self, parent: Entity, matrix: Mat4) -> Entity {
            let entity = self.world.spawn((
                MeshComponent {
                    handle: AssetHandle::weak(self.mesh),
                },
                Aabb {
                    min: Vec3::splat(-1.0),
                    max: Vec3::splat(1.0),
                },
                GlobalTransform::new(matrix),
            ));
            self.world.entity_mut(parent).add_child(entity);
            entity
        }
        fn pick(&mut self, origin: [f32; 3], direction: [f32; 3]) -> Option<Entity> {
            let ray = Ray {
                origin: origin.into(),
                direction: Vec3::from(direction).normalize(),
            };
            let mut scene = std::collections::HashSet::new();
            let children: Vec<(Entity, Vec<Entity>)> = self
                .world
                .query::<(Entity, &Children), ()>()
                .iter(&mut self.world)
                .map(|(entity, children)| (entity, children.iter().copied().collect()))
                .collect();
            let mut stack = vec![self.root];
            while let Some(entity) = stack.pop() {
                scene.insert(entity);
                if let Some((_, kids)) = children.iter().find(|(parent, _)| *parent == entity) {
                    stack.extend(kids);
                }
            }
            let nodes: Vec<_> = self
                .world
                .query::<(Entity, &MeshComponent, &Aabb, &GlobalTransform), ()>()
                .iter(&mut self.world)
                .filter(|(entity, ..)| scene.contains(entity))
                .map(|(entity, mesh, aabb, transform)| {
                    (
                        entity,
                        MeshComponent {
                            handle: AssetHandle::weak(mesh.handle.id()),
                        },
                        *aabb,
                        GlobalTransform::new(transform.matrix()),
                    )
                })
                .collect();
            super::pick(
                &ray,
                self.world.get_resource::<AssetStore<Mesh>>().unwrap(),
                nodes
                    .iter()
                    .map(|(entity, mesh, aabb, transform)| (*entity, mesh, aabb, transform)),
            )
        }
        fn selected(&self) -> Option<Entity> {
            self.world.get_resource::<Selection>().unwrap().entity()
        }
        fn segments(&mut self) -> usize {
            self.world.insert_resource(GizmoStorage::default());
            let mut system = draw_selection_bounds.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
            self.world
                .get_resource::<GizmoStorage>()
                .unwrap()
                .segment_count()
        }
    }

    const VIEW: UIBox = UIBox {
        min: Vec2::new(100.0, 50.0),
        size: Vec2::new(800.0, 600.0),
    };

    #[test]
    fn the_centre_of_the_viewport_looks_along_the_camera() {
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let transform = Transform::from_translation_rotation(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(0.6),
        );
        let ray = viewport_ray(
            Vec2::new(500.0, 350.0),
            VIEW,
            camera.build_projection_matrix(),
            transform.compute_matrix(),
        )
        .unwrap();
        let forward = transform.rotation * Vec3::NEG_Z;
        assert!((ray.direction - forward).length() < 1e-4);
        assert!((ray.origin - (transform.translation + forward * camera.znear)).length() < 1e-3);
    }

    #[test]
    fn a_corner_of_the_viewport_matches_the_field_of_view_and_aspect() {
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let ray = viewport_ray(
            Vec2::new(900.0, 50.0),
            VIEW,
            camera.build_projection_matrix(),
            Mat4::IDENTITY,
        )
        .unwrap();
        let half_height = (camera.fovy * 0.5).tan();
        let expected = Vec3::new(half_height * camera.aspect, half_height, -1.0).normalize();
        assert!(
            (ray.direction - expected).length() < 1e-4,
            "{:?}",
            ray.direction
        );
        assert!(
            viewport_ray(
                Vec2::new(99.0, 300.0),
                VIEW,
                camera.build_projection_matrix(),
                Mat4::IDENTITY
            )
            .is_none()
        );
    }

    #[test]
    fn a_mesh_inside_a_larger_one_is_picked_only_when_the_ray_reaches_it_first() {
        let mut f = fixture();
        let room = f.cube(f.root, Mat4::from_scale(Vec3::splat(10.0)));
        let crate_ = f.cube(f.root, Mat4::IDENTITY);
        assert_eq!(f.pick([0.0, 0.0, 5.0], [0.0, 0.0, -1.0]), Some(crate_));
        assert_eq!(f.pick([0.0, 5.0, 5.0], [0.0, 0.0, -1.0]), Some(room));
        assert_eq!(f.pick([0.0, 0.0, 50.0], [0.0, 0.0, -1.0]), Some(room));
    }

    #[test]
    fn a_rotated_and_scaled_entity_is_hit_where_it_is_drawn() {
        let mut f = fixture();
        let slab = f.cube(
            f.root,
            Mat4::from_scale_rotation_translation(
                Vec3::new(4.0, 0.1, 0.1),
                Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                Vec3::new(6.0, 0.0, 0.0),
            ),
        );
        assert_eq!(f.pick([6.0, 3.5, 5.0], [0.0, 0.0, -1.0]), Some(slab));
        assert_eq!(f.pick([8.0, 0.0, 5.0], [0.0, 0.0, -1.0]), None);
    }

    #[test]
    fn an_entity_outside_the_scene_is_ignored_and_does_not_block() {
        let mut f = fixture();
        let stray_parent = f.world.spawn(());
        f.cube(
            stray_parent,
            Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0)),
        );
        let inside = f.cube(f.root, Mat4::IDENTITY);
        assert_eq!(f.pick([0.0, 0.0, 9.0], [0.0, 0.0, -1.0]), Some(inside));
        assert_eq!(f.pick([0.0, 9.0, 9.0], [0.0, 0.0, -1.0]), None);
    }

    fn viewport(f: &mut Fixture) -> Entity {
        f.world.spawn((
            EditorCamera,
            Camera {
                aspect: 4.0 / 3.0,
                ..Default::default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, 6.0)),
        ));
        f.world.spawn((
            UILayout {
                rect: VIEW,
                content_rect: VIEW,
                clip_rect: VIEW,
                paint_order: 0,
            },
            press_viewport.into_listener(),
            click_viewport.into_listener(),
        ))
    }

    fn click(f: &mut Fixture, node: Entity, pressed: Vec2, released: Vec2) {
        f.world.trigger_on(
            node,
            UIPointerDown {
                position: pressed,
                button: MouseButton::Left,
            },
        );
        f.world.trigger_on(
            node,
            UIClick {
                position: released,
                button: MouseButton::Left,
            },
        );
    }

    #[test]
    fn a_click_selects_and_reveals_the_hit_and_a_miss_clears_the_selection() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        let node = viewport(&mut f);
        let centre = Vec2::new(500.0, 350.0);

        click(&mut f, node, centre, centre + Vec2::new(2.0, 1.0));
        assert_eq!(f.selected(), Some(cube));
        assert_eq!(
            f.world
                .get_resource::<HierarchyState>()
                .unwrap()
                .pending_reveal(),
            Some(cube)
        );

        let corner = Vec2::new(110.0, 60.0);
        click(&mut f, node, corner, corner);
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_that_moved_or_a_flying_camera_selects_nothing() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        let node = viewport(&mut f);
        let centre = Vec2::new(500.0, 350.0);

        click(&mut f, node, centre - Vec2::new(30.0, 0.0), centre);
        assert_eq!(f.selected(), None);

        f.world
            .get_resource_mut::<FlyCamera>()
            .unwrap()
            .set_looking(true);
        click(&mut f, node, centre, centre);
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_that_landed_on_a_gizmo_handle_is_not_a_pick() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        let node = viewport(&mut f);
        let centre = Vec2::new(500.0, 350.0);
        f.world
            .get_resource_mut::<GizmoState>()
            .unwrap()
            .set_captured(true);
        click(&mut f, node, centre, centre);
        assert_eq!(f.selected(), None);
        f.world
            .get_resource_mut::<GizmoState>()
            .unwrap()
            .set_captured(false);
        click(&mut f, node, centre, centre);
        assert_eq!(f.selected(), Some(cube));
    }

    #[test]
    fn a_world_point_projects_to_its_place_in_the_viewport_or_nowhere_behind_the_camera() {
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let view = Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)).compute_matrix();
        let view_proj = camera.build_projection_matrix() * view.inverse();
        assert!(
            (world_to_viewport(Vec3::ZERO, VIEW, view_proj).unwrap() - Vec2::new(500.0, 350.0))
                .length()
                < 1e-3
        );
        let up = world_to_viewport(Vec3::Y, VIEW, view_proj).unwrap();
        assert!(up.y < 350.0 && (up.x - 500.0).abs() < 1e-3);
        let ray = viewport_ray(up, VIEW, camera.build_projection_matrix(), view).unwrap();
        let back = ray.at((Vec3::Y - ray.origin).length());
        assert!((back - Vec3::Y).length() < 1e-3, "{back:?}");
        assert_eq!(
            world_to_viewport(Vec3::new(0.0, 0.0, 20.0), VIEW, view_proj),
            None
        );
    }

    #[test]
    fn the_selection_box_is_drawn_for_meshes_and_groups_but_not_the_scene_root() {
        let mut f = fixture();
        let group = f.world.spawn(());
        f.world.entity_mut(f.root).add_child(group);
        let cube = f.cube(group, Mat4::IDENTITY);
        f.cube(group, Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)));
        let empty = f.world.spawn(());
        f.world.entity_mut(f.root).add_child(empty);

        assert_eq!(f.segments(), 0, "nothing is selected");
        for (entity, segments) in [(cube, 12), (group, 12), (empty, 0), (f.root, 0)] {
            f.world
                .get_resource_mut::<Selection>()
                .unwrap()
                .select_entity(entity);
            assert_eq!(f.segments(), segments);
        }
    }

    #[test]
    fn a_group_is_boxed_around_all_of_its_descendants() {
        let mut f = fixture();
        let group = f.world.spawn(());
        f.world.entity_mut(f.root).add_child(group);
        f.cube(group, Mat4::IDENTITY);
        f.cube(group, Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)));
        let mut system = (move |roots: Query<&SceneRoot>,
                                bounds: Query<(&Aabb, &GlobalTransform)>,
                                children: Query<&Children>| {
            let corners = selection_corners(group, &roots, &bounds, &children).unwrap();
            assert_eq!(corners[0], Vec3::splat(-1.0));
            assert_eq!(corners[7], Vec3::new(6.0, 1.0, 1.0));
        })
        .into_system();
        system.initialize(&mut f.world);
        system.run_and_apply((), &mut f.world);
    }
}
