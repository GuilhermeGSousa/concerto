//! Selecting scene entities by clicking them in the viewport.
use concerto_app::{
    App,
    edit_time::AtEditTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, Res, ResMut, Resource, command::CommandQueue,
    entity::hierarchy::Children, events::event_writer::EventWriter,
};
use concerto_foundation::{
    assets::asset_store::AssetStore,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{Aabb, Mesh, MeshComponent, Ray};
use concerto_render::components::camera::Camera;
use concerto_window::input::{Input, MouseButton};
use glam::{Mat4, Vec2, Vec3};

use crate::tools::{
    camera::{EditorCamera, FlyCamera, ViewportPointer, fill_pointer_from_window, subtree},
    gizmo::GizmoState,
    scene::{RevealEntity, SceneRoot},
    selection::{Select, Selection},
};

/// How far the pointer may travel between press and release and still count as a click.
const CLICK_SLOP: f32 = 4.0;
const SELECTION_BOX: Color = Color::srgba(0.569, 0.518, 0.851, 1.0);

#[derive(Resource, Default)]
pub(crate) struct PickPress(Option<Vec2>);

pub(crate) fn build(app: &mut App) {
    app.insert_resource(PickPress::default());
    app.add_system(
        Update,
        pick_on_click.after(fill_pointer_from_window).at_edit_time(),
    );
    app.add_system(LateUpdate, draw_selection_bounds.at_edit_time());
}

/// The world-space ray under `pointer`, or `None` when it is outside a picture of `size`.
pub fn viewport_ray(pointer: Vec2, size: Vec2, projection: Mat4, camera: Mat4) -> Option<Ray> {
    if pointer.cmplt(Vec2::ZERO).any() || pointer.cmpgt(size).any() {
        return None;
    }
    ray_through(pointer, size, projection, camera)
}

/// [`viewport_ray`] without the bounds check, for a drag that has left the viewport.
pub fn ray_through(pointer: Vec2, size: Vec2, projection: Mat4, camera: Mat4) -> Option<Ray> {
    if size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let uv = pointer / size;
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

/// Where a world point appears in a picture of `size`, or `None` when it is behind the camera.
pub fn world_to_viewport(point: Vec3, size: Vec2, view_proj: Mat4) -> Option<Vec2> {
    let clip = view_proj * point.extend(1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate().truncate() / clip.w;
    let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    Some(uv * size)
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn pick_on_click(
    pointer: Res<ViewportPointer>,
    input: Res<Input>,
    fly: Res<FlyCamera>,
    gizmo: Res<GizmoState>,
    cameras: Query<(&EditorCamera, &Camera, &Transform)>,
    roots: Query<(Entity, &SceneRoot)>,
    children: Query<&Children>,
    nodes: Query<(Entity, &MeshComponent, &Aabb, &GlobalTransform)>,
    meshes: Res<AssetStore<Mesh>>,
    mut press: ResMut<PickPress>,
    mut reveal: EventWriter<RevealEntity>,
    mut cmd: CommandQueue,
) {
    if pointer.pressed {
        press.0 = pointer.over.then_some(pointer.position);
    }
    if pointer.held
        && press
            .0
            .is_some_and(|pressed_at| pressed_at.distance(pointer.position) >= CLICK_SLOP)
    {
        press.0 = None;
    }
    if !input.is_mouse_button_just_released(MouseButton::Left) {
        return;
    }
    let Some(pressed_at) = press.0.take() else {
        return;
    };
    if !pointer.over
        || fly.navigating()
        || gizmo.captured()
        || pressed_at.distance(pointer.position) >= CLICK_SLOP
    {
        return;
    }
    let Some((_, camera, transform)) = cameras.iter().next() else {
        return;
    };
    let Some(ray) = viewport_ray(
        pointer.position,
        pointer.size,
        camera.build_projection_matrix(),
        transform.compute_matrix(),
    ) else {
        return;
    };

    let mut scene = std::collections::HashSet::new();
    for (root, _) in roots.iter() {
        scene.extend(subtree(root, &children));
    }
    match pick(
        &ray,
        &meshes,
        nodes.iter().filter(|(entity, ..)| scene.contains(entity)),
    ) {
        Some(entity) => {
            cmd.trigger(Select(Some(entity)));
            reveal.write(RevealEntity(entity));
        }
        None => cmd.trigger(Select(None)),
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
                gizmos.line(corners[from], corners[from | axis], SELECTION_BOX);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_debug_gizmos::GizmoStorage;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::ChildOf,
        events::{event_channel::EventChannel, event_cursor::EventCursor},
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
        Mesh::single(vertices, indices)
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
        world.add_listener(crate::tools::selection::select);
        world.insert_resource(EventChannel::<RevealEntity>::default());
        world.insert_resource(FlyCamera::default());
        world.insert_resource(PickPress::default());
        world.insert_resource(GizmoState::default());
        world.insert_resource(Input::new());
        world.insert_resource(ViewportPointer::default());
        world.insert_resource(GizmoStorage::default());
        let root = world.spawn(SceneRoot);
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
        fn revealed(&self) -> Vec<Entity> {
            self.world
                .get_resource::<EventChannel<RevealEntity>>()
                .unwrap()
                .read_from(&mut EventCursor::default())
                .map(|reveal| reveal.0)
                .collect()
        }
        fn point_at(&mut self, position: Vec2, pressed: bool, held: bool) {
            *self.world.get_resource_mut::<ViewportPointer>().unwrap() = ViewportPointer {
                position,
                size: VIEW,
                over: true,
                pressed,
                held,
                scale: 1.0,
            };
            let input = self.world.get_resource_mut::<Input>().unwrap();
            input.update();
            if pressed {
                input.update_mouse_button(MouseButton::Left, true);
            } else if !held {
                input.update_mouse_button(MouseButton::Left, false);
            }
        }
        fn leave_the_picture(&mut self) {
            self.world
                .get_resource_mut::<ViewportPointer>()
                .unwrap()
                .over = false;
        }
        fn step(&mut self) {
            let mut system = pick_on_click.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
        }
        fn click(&mut self, pressed: Vec2, released: Vec2) {
            self.point_at(pressed, true, true);
            self.step();
            self.point_at(released, false, false);
            self.step();
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

    const VIEW: Vec2 = Vec2::new(800.0, 600.0);
    const CENTRE: Vec2 = Vec2::new(400.0, 300.0);

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
            CENTRE,
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
            Vec2::new(800.0, 0.0),
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
                Vec2::new(-1.0, 300.0),
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

    fn camera(f: &mut Fixture) {
        f.world.spawn((
            EditorCamera,
            Camera {
                aspect: 4.0 / 3.0,
                ..Default::default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, 6.0)),
        ));
    }

    #[test]
    fn a_press_and_release_in_one_place_selects_what_is_under_the_pointer() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE, false, false);
        f.step();

        assert_eq!(f.selected(), Some(cube));
    }

    #[test]
    fn a_release_far_from_the_press_selects_nothing() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(30.0, 0.0), false, false);
        f.step();

        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_outside_the_picture_is_not_a_click() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.leave_the_picture();
        f.step();
        f.point_at(CENTRE, false, false);
        f.step();

        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_release_outside_the_picture_is_not_a_click() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE, false, false);
        f.leave_the_picture();
        f.step();

        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_that_strayed_and_came_back_while_held_is_not_a_click() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(CLICK_SLOP, 0.0), false, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(1.0, 0.0), false, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(1.0, 0.0), false, false);
        f.step();

        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_that_wandered_less_than_the_slop_and_came_back_is_still_a_click() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(CLICK_SLOP - 0.5, 0.0), false, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(1.0, 0.0), false, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(1.0, 0.0), false, false);
        f.step();

        assert_eq!(f.selected(), Some(cube));
    }

    #[test]
    fn a_release_exactly_at_the_slop_is_not_a_click() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.point_at(CENTRE, true, true);
        f.step();
        f.point_at(CENTRE + Vec2::new(CLICK_SLOP, 0.0), false, false);
        f.step();

        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_click_selects_and_reveals_the_hit_and_a_miss_clears_the_selection() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.click(CENTRE, CENTRE + Vec2::new(2.0, 1.0));
        assert_eq!(f.selected(), Some(cube));
        assert_eq!(f.revealed(), [cube]);

        let corner = Vec2::new(10.0, 10.0);
        f.click(corner, corner);
        assert_eq!(f.selected(), None);
        assert_eq!(f.revealed(), [cube], "a miss reveals nothing");
    }

    #[test]
    fn a_press_that_moved_or_a_flying_camera_selects_nothing() {
        let mut f = fixture();
        f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);

        f.click(CENTRE - Vec2::new(30.0, 0.0), CENTRE);
        assert_eq!(f.selected(), None);

        f.world
            .get_resource_mut::<FlyCamera>()
            .unwrap()
            .set_looking(true);
        f.click(CENTRE, CENTRE);
        assert_eq!(f.selected(), None);
    }

    #[test]
    fn a_press_that_landed_on_a_gizmo_handle_is_not_a_pick() {
        let mut f = fixture();
        let cube = f.cube(f.root, Mat4::IDENTITY);
        camera(&mut f);
        f.world
            .get_resource_mut::<GizmoState>()
            .unwrap()
            .set_captured(true);
        f.click(CENTRE, CENTRE);
        assert_eq!(f.selected(), None);
        f.world
            .get_resource_mut::<GizmoState>()
            .unwrap()
            .set_captured(false);
        f.click(CENTRE, CENTRE);
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
        assert!((world_to_viewport(Vec3::ZERO, VIEW, view_proj).unwrap() - CENTRE).length() < 1e-3);
        let up = world_to_viewport(Vec3::Y, VIEW, view_proj).unwrap();
        assert!(up.y < CENTRE.y && (up.x - CENTRE.x).abs() < 1e-3);
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
            f.world.trigger(Select(Some(entity)));
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
