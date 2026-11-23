//! Selecting scene entities by clicking them in the viewport.
use concerto_app::{App, edit_time::AtEditTime, schedule_groups::Update};
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
use concerto_window::input::{Input, MouseButton, actions::Modifiers};
use glam::{Mat4, Vec2, Vec3};

use crate::tools::{
    camera::{EditorCamera, FlyCamera, ViewportPointer, fill_pointer_from_window, subtree},
    gizmo::GizmoState,
    scene::{RevealEntity, SceneRoot},
    selection::Select,
};

/// How far the pointer may travel between press and release and still count as a click.
const CLICK_SLOP: f32 = 4.0;

#[derive(Resource, Default)]
pub(crate) struct PickPress(Option<Vec2>);

pub(crate) fn build(app: &mut App) {
    app.insert_resource(PickPress::default());
    app.add_system(
        Update,
        pick_on_click.after(fill_pointer_from_window).at_edit_time(),
    );
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

/// The mesh a ray hits first: its entity and how far along the ray the hit is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickHit {
    pub entity: Entity,
    pub distance: f32,
}

/// The entity whose mesh `ray` hits first.
pub fn pick<'a>(
    ray: &Ray,
    meshes: &AssetStore<Mesh>,
    nodes: impl Iterator<Item = (Entity, &'a MeshComponent, &'a Aabb, &'a GlobalTransform)>,
) -> Option<Entity> {
    pick_hit(ray, meshes, nodes).map(|hit| hit.entity)
}

/// The nearest mesh hit of `ray` among `nodes`.
pub fn pick_hit<'a>(
    ray: &Ray,
    meshes: &AssetStore<Mesh>,
    nodes: impl Iterator<Item = (Entity, &'a MeshComponent, &'a Aabb, &'a GlobalTransform)>,
) -> Option<PickHit> {
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

    let mut nearest: Option<PickHit> = None;
    for (entry, entity, local, mesh) in candidates {
        if nearest.is_some_and(|hit| entry > hit.distance) {
            break;
        }
        if let Some(distance) = meshes
            .get(&mesh.handle)
            .and_then(|mesh| mesh.ray_hit(&local))
            && nearest.is_none_or(|hit| distance < hit.distance)
        {
            nearest = Some(PickHit { entity, distance });
        }
    }
    nearest
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
    let modifiers = Modifiers::from_input(&input);
    let adding = modifiers.ctrl || modifiers.shift || modifiers.logo;
    match pick(
        &ray,
        &meshes,
        nodes.iter().filter(|(entity, ..)| scene.contains(entity)),
    ) {
        Some(entity) if adding => cmd.trigger(Select::Toggle(entity)),
        Some(entity) => {
            cmd.trigger(Select::only(entity));
            reveal.write(RevealEntity(entity));
        }
        None if adding => {}
        None => cmd.trigger(Select::none()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::selection::Selection;
    use concerto_ecs::{
        IntoSystem, System, World,
        entity::hierarchy::ChildOf,
        events::{event_channel::EventChannel, event_cursor::EventCursor},
    };
    use concerto_foundation::assets::{AssetId, handle::AssetHandle};
    use concerto_mesh::Vertex;
    use concerto_window::input::{KeyCode, PhysicalKey};
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
        world.add_listener(crate::tools::selection::select);
        world.insert_resource(EventChannel::<RevealEntity>::default());
        world.insert_resource(FlyCamera::default());
        world.insert_resource(PickPress::default());
        world.insert_resource(GizmoState::default());
        world.insert_resource(Input::new());
        world.insert_resource(ViewportPointer::default());
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
        fn selected(&self) -> Option<Entity> {
            self.world.get_resource::<Selection>().unwrap().active()
        }
        fn selection(&self) -> Vec<Entity> {
            self.world
                .get_resource::<Selection>()
                .unwrap()
                .entities()
                .to_vec()
        }
        fn hold(&mut self, key: KeyCode, down: bool) {
            self.world
                .get_resource_mut::<Input>()
                .unwrap()
                .update_key_input(PhysicalKey::Code(key), down);
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
            let mut system = pick_on_click.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
        }
        fn click(&mut self, pressed: Vec2, released: Vec2) {
            self.point_at(pressed, true, true);
            self.point_at(released, false, false);
        }
    }

    const VIEW: Vec2 = Vec2::new(800.0, 600.0);
    const CENTRE: Vec2 = Vec2::new(400.0, 300.0);

    #[test]
    fn a_viewport_point_and_its_ray_agree_with_the_projection() {
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let corner = viewport_ray(
            Vec2::new(800.0, 0.0),
            VIEW,
            camera.build_projection_matrix(),
            Mat4::IDENTITY,
        )
        .unwrap();
        let half_height = (camera.fovy * 0.5).tan();
        let expected = Vec3::new(half_height * camera.aspect, half_height, -1.0).normalize();
        assert!((corner.direction - expected).length() < 1e-4);

        let view = Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)).compute_matrix();
        let view_proj = camera.build_projection_matrix() * view.inverse();
        assert!((world_to_viewport(Vec3::ZERO, VIEW, view_proj).unwrap() - CENTRE).length() < 1e-3);
        let up = world_to_viewport(Vec3::Y, VIEW, view_proj).unwrap();
        let ray = viewport_ray(up, VIEW, camera.build_projection_matrix(), view).unwrap();
        let back = ray.at((Vec3::Y - ray.origin).length());
        assert!((back - Vec3::Y).length() < 1e-3, "{back:?}");
        assert_eq!(
            world_to_viewport(Vec3::new(0.0, 0.0, 20.0), VIEW, view_proj),
            None
        );
    }

    struct Shot {
        origin: [f32; 3],
        hits: Option<usize>,
    }

    #[test]
    fn a_ray_picks_the_first_surface_it_reaches_where_it_is_drawn() {
        let mut f = fixture();
        let root = f.root;
        let cubes = [
            f.cube(root, Mat4::from_scale(Vec3::splat(10.0))),
            f.cube(root, Mat4::IDENTITY),
            f.cube(
                root,
                Mat4::from_scale_rotation_translation(
                    Vec3::new(4.0, 0.1, 0.1),
                    Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                    Vec3::new(16.0, 0.0, 0.0),
                ),
            ),
        ];
        let shots = [
            Shot {
                origin: [0.0, 0.0, 5.0],
                hits: Some(1),
            },
            Shot {
                origin: [0.0, 5.0, 5.0],
                hits: Some(0),
            },
            Shot {
                origin: [0.0, 0.0, 50.0],
                hits: Some(0),
            },
            Shot {
                origin: [16.0, 3.5, 5.0],
                hits: Some(2),
            },
            Shot {
                origin: [18.0, 0.0, 5.0],
                hits: None,
            },
        ];
        let meshes = f.world.get_resource::<AssetStore<Mesh>>().unwrap();
        let handle = MeshComponent {
            handle: AssetHandle::weak(f.mesh),
        };
        let aabb = Aabb {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        let transforms: Vec<GlobalTransform> = cubes
            .iter()
            .map(|cube| {
                GlobalTransform::new(
                    f.world
                        .get_component_for_entity::<GlobalTransform>(*cube)
                        .unwrap()
                        .matrix(),
                )
            })
            .collect();
        for shot in shots {
            let ray = Ray {
                origin: shot.origin.into(),
                direction: Vec3::NEG_Z,
            };
            let hit = super::pick(
                &ray,
                meshes,
                cubes
                    .iter()
                    .zip(&transforms)
                    .map(|(cube, transform)| (*cube, &handle, &aabb, transform)),
            );
            assert_eq!(
                hit,
                shot.hits.map(|index| cubes[index]),
                "{:?}",
                shot.origin
            );
        }
    }

    #[test]
    fn a_click_selects_and_reveals_the_scene_entity_under_it_and_a_miss_clears_the_selection() {
        let mut f = fixture();
        let stray = f.world.spawn(());
        f.cube(stray, Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0)));
        let cube = f.cube(f.root, Mat4::IDENTITY);
        f.world.spawn((
            EditorCamera,
            Camera {
                aspect: 4.0 / 3.0,
                ..Default::default()
            },
            Transform::from_translation(Vec3::new(0.0, 0.0, 6.0)),
        ));

        f.click(CENTRE, CENTRE + Vec2::new(30.0, 0.0));
        assert_eq!(f.selected(), None, "a press that moved is not a click");

        f.click(CENTRE, CENTRE + Vec2::new(2.0, 1.0));
        assert_eq!(f.selected(), Some(cube));
        assert_eq!(f.revealed(), [cube]);

        let corner = Vec2::new(10.0, 10.0);
        f.click(corner, corner);
        assert_eq!(f.selected(), None);
        assert_eq!(f.revealed(), [cube]);

        let other = f.cube(f.root, Mat4::from_translation(Vec3::new(-3.0, 0.0, 0.0)));
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let view = Transform::from_translation(Vec3::new(0.0, 0.0, 6.0)).compute_matrix();
        let beside = world_to_viewport(
            Vec3::new(-3.0, 0.0, 0.0),
            VIEW,
            camera.build_projection_matrix() * view.inverse(),
        )
        .unwrap();
        f.click(CENTRE, CENTRE);
        f.hold(KeyCode::ControlLeft, true);
        f.click(beside, beside);
        assert_eq!(f.selection(), [cube, other]);
        f.click(CENTRE, CENTRE);
        assert_eq!(
            f.selection(),
            [other],
            "a modified click on a selected one removes it"
        );
        f.click(corner, corner);
        assert_eq!(
            f.selection(),
            [other],
            "a modified click on nothing changes nothing"
        );
        f.hold(KeyCode::ControlLeft, false);
        f.hold(KeyCode::ShiftLeft, true);
        f.click(CENTRE, CENTRE);
        assert_eq!(f.selection(), [other, cube]);
        f.hold(KeyCode::ShiftLeft, false);
        f.click(corner, corner);
        assert_eq!(f.selected(), None);

        f.world
            .get_resource_mut::<GizmoState>()
            .unwrap()
            .set_captured(true);
        f.click(CENTRE, CENTRE);
        assert_eq!(
            f.selected(),
            None,
            "a press on a gizmo handle is not a pick"
        );
    }
}
