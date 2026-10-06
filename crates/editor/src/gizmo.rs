//! Handles for moving and rotating the selected entity in the viewport.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;
use concerto_ecs::{
    Entity, Query, Res, ResMut, Resource, entity::hierarchy::ChildOf,
    events::event_reader::EventReader,
};
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_mesh::Ray;
use concerto_render::components::camera::Camera;
use concerto_ui::{
    interaction::HoveredNode,
    node::{UIBox, UILayout},
};
use concerto_window::input::{Input, MouseButton, actions::ActionFired};
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::{
    actions::{GizmoRotate, GizmoTranslate},
    asset_editor::{EditorDocument, EditorOwned, mark_edited_with},
    picking::{ray_through, world_to_viewport},
    scene::SceneRoot,
    selection::Selection,
    viewport::{EditorCamera, FlyCamera, ViewportRegion},
};

/// How much of the viewport's height a handle spans.
const SCREEN_FRACTION: f32 = 0.14;
/// How close, in logical pixels, the pointer must be to a handle to grab it.
const GRAB_DISTANCE: f32 = 8.0;
const LINE_WIDTH: f32 = 3.0;
const RING_SEGMENTS: usize = 48;
/// A ring whose plane the pointer's ray meets more shallowly than this cannot be dragged.
const EDGE_ON: f32 = 0.05;
const HIGHLIGHT: Color = Color::srgba(1.0, 0.85, 0.2, 1.0);

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GizmoMode {
    #[default]
    Translate,
    Rotate,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn direction(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub fn color(self) -> Color {
        match self {
            Axis::X => Color::srgba(0.92, 0.26, 0.30, 1.0),
            Axis::Y => Color::srgba(0.46, 0.82, 0.26, 1.0),
            Axis::Z => Color::srgba(0.27, 0.52, 0.96, 1.0),
        }
    }
}

/// The pointer relative to the viewport, sampled once per frame.
#[derive(Resource, Default, Clone, Copy)]
pub struct ViewportPointer {
    pub position: Vec2,
    pub rect: UIBox,
    /// Whether the viewport is the node under the pointer.
    pub over: bool,
    /// Whether the left button went down this frame.
    pub pressed: bool,
    pub held: bool,
    pub scale: f32,
}

struct Drag {
    entity: Entity,
    axis: Axis,
    mode: GizmoMode,
    origin: Vec3,
    start: Transform,
    parent: Mat4,
    grab: f32,
    moved: bool,
}

#[derive(Resource, Default)]
pub struct GizmoState {
    mode: GizmoMode,
    hovered: Option<Axis>,
    drag: Option<Drag>,
    captured: bool,
}

impl GizmoState {
    /// Whether the current or most recent press landed on a handle.
    pub fn captured(&self) -> bool {
        self.captured
    }

    pub fn mode(&self) -> GizmoMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: GizmoMode) {
        if self.mode != mode {
            self.mode = mode;
        }
    }

    pub fn hovered(&self) -> Option<Axis> {
        self.hovered
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    #[cfg(test)]
    pub(crate) fn set_captured(&mut self, captured: bool) {
        self.captured = captured;
    }
}

pub struct GizmoPlugin;

impl Plugin for GizmoPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GizmoState::default());
        app.insert_resource(ViewportPointer::default());
        app.add_system(Update, track_viewport_pointer)
            .add_system(Update, update_gizmo)
            .add_system(LateUpdate, gizmo_shortcuts);
    }
}

/// The handle length that spans [`SCREEN_FRACTION`] of the viewport at `origin`.
pub fn gizmo_size(origin: Vec3, camera: Vec3, fovy: f32) -> f32 {
    origin.distance(camera) * (fovy * 0.5).tan() * 2.0 * SCREEN_FRACTION
}

/// How far along `axis` from `origin` the line comes nearest to `ray`.
pub fn axis_param(ray: &Ray, origin: Vec3, axis: Vec3) -> Option<f32> {
    let offset = origin - ray.origin;
    let along = axis.dot(ray.direction);
    let denominator = axis.length_squared() * ray.direction.length_squared() - along * along;
    if denominator.abs() < 1e-6 {
        return None;
    }
    let axis_offset = axis.dot(offset);
    let ray_offset = ray.direction.dot(offset);
    Some((along * ray_offset - ray.direction.length_squared() * axis_offset) / denominator)
}

/// The angle, about `axis`, of the point where `ray` meets the plane through `origin`.
pub fn plane_angle(ray: &Ray, origin: Vec3, axis: Vec3) -> Option<f32> {
    let facing = ray.direction.dot(axis);
    if facing.abs() < EDGE_ON {
        return None;
    }
    let distance = (origin - ray.origin).dot(axis) / facing;
    if distance < 0.0 {
        return None;
    }
    let (u, v) = ring_basis(axis);
    let hit = ray.at(distance) - origin;
    Some(hit.dot(v).atan2(hit.dot(u)))
}

fn ring_basis(axis: Vec3) -> (Vec3, Vec3) {
    let u = axis.any_orthonormal_vector();
    (u, axis.cross(u))
}

fn segment_distance(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let length = along.length_squared();
    let t = if length > 0.0 {
        ((point - a).dot(along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(a + along * t)
}

/// The handle under `pointer`, if one is within reach.
pub fn hovered_handle(
    pointer: Vec2,
    mode: GizmoMode,
    origin: Vec3,
    size: f32,
    rect: UIBox,
    view_proj: Mat4,
) -> Option<Axis> {
    let project = |point: Vec3| world_to_viewport(point, rect, view_proj);
    Axis::ALL
        .into_iter()
        .filter_map(|axis| {
            let distance = match mode {
                GizmoMode::Translate => segment_distance(
                    pointer,
                    project(origin)?,
                    project(origin + axis.direction() * size)?,
                ),
                GizmoMode::Rotate => {
                    let (u, v) = ring_basis(axis.direction());
                    let on_ring = |index: usize| {
                        let angle = index as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                        project(origin + (u * angle.cos() + v * angle.sin()) * size)
                    };
                    (0..RING_SEGMENTS)
                        .filter_map(|index| {
                            Some(segment_distance(
                                pointer,
                                on_ring(index)?,
                                on_ring(index + 1)?,
                            ))
                        })
                        .fold(f32::INFINITY, f32::min)
                }
            };
            (distance <= GRAB_DISTANCE).then_some((distance, axis))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, axis)| axis)
}

fn track_viewport_pointer(
    input: Res<Input>,
    window: Res<concerto_window::plugin::Window>,
    hovered: Res<HoveredNode>,
    regions: Query<(Entity, &ViewportRegion, &UILayout)>,
    mut pointer: ResMut<ViewportPointer>,
) {
    let Some((region, _, layout)) = regions.iter().next() else {
        return;
    };
    *pointer = ViewportPointer {
        position: window.logical_pointer_position(&input),
        rect: layout.rect,
        over: **hovered == Some(region),
        pressed: input.is_mouse_button_just_pressed(MouseButton::Left),
        held: input.is_mouse_button_held(MouseButton::Left),
        scale: window.scale_factor() as f32,
    };
}

#[allow(clippy::too_many_arguments)]
fn update_gizmo(
    pointer: Res<ViewportPointer>,
    selection: Res<Selection>,
    fly: Res<FlyCamera>,
    cameras: Query<(Entity, &EditorCamera, &Camera)>,
    roots: Query<&SceneRoot>,
    globals: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    owners: Query<&EditorOwned>,
    documents: Query<&mut EditorDocument>,
    transforms: Query<&mut Transform>,
    mut state: ResMut<GizmoState>,
    mut gizmos: DebugGizmos,
) {
    if pointer.pressed && state.captured {
        state.captured = false;
    }
    let mode = state.mode;
    let target = selection.entity().filter(|entity| {
        roots.get_entity(*entity).is_none() && transforms.get_entity(*entity).is_some()
    });
    if state
        .drag
        .as_ref()
        .is_some_and(|drag| !pointer.held || Some(drag.entity) != target)
    {
        if let Some(drag) = state.drag.take().filter(|drag| drag.moved) {
            mark_edited_with(drag.entity, &owners, &parents, &documents);
        }
    }

    let camera = cameras.iter().next().and_then(|(entity, _, camera)| {
        let transform = transforms.get_entity(entity)?;
        Some((
            camera.build_projection_matrix(),
            transform.compute_matrix(),
            camera.fovy,
        ))
    });
    let (Some(target), Some((projection, camera_matrix, fovy))) = (target, camera) else {
        if state.hovered.is_some() {
            state.hovered = None;
        }
        return;
    };
    if fly.navigating() && state.drag.is_none() {
        if state.hovered.is_some() {
            state.hovered = None;
        }
        return;
    }

    let view_proj = projection * camera_matrix.inverse();
    let ray = ray_through(pointer.position, pointer.rect, projection, camera_matrix);
    let parent = parents
        .get_entity(target)
        .and_then(|parent| globals.get_entity(parent.parent()))
        .map_or(Mat4::IDENTITY, |parent| parent.matrix());
    let mut origin = globals
        .get_entity(target)
        .map(|global| global.matrix().w_axis.truncate())
        .or_else(|| {
            let transform = transforms.get_entity(target)?;
            Some(parent.transform_point3(transform.translation))
        })
        .unwrap_or_default();

    if let Some(drag) = state.drag.as_mut() {
        let axis = drag.axis.direction();
        origin = drag.origin;
        if let (Some(ray), Some(mut transform)) = (ray, transforms.get_entity(target)) {
            match drag.mode {
                GizmoMode::Translate => {
                    if let Some(param) = axis_param(&ray, drag.origin, axis) {
                        let delta = param - drag.grab;
                        origin = drag.origin + axis * delta;
                        let local = drag.parent.inverse().transform_point3(origin);
                        if transform.translation != local {
                            transform.translation = local;
                            drag.moved = true;
                        }
                    } else {
                        origin = drag.parent.transform_point3(transform.translation);
                    }
                }
                GizmoMode::Rotate => {
                    if let Some(angle) = plane_angle(&ray, drag.origin, axis) {
                        let parent_rotation = drag.parent.to_scale_rotation_translation().1;
                        let local = parent_rotation.inverse()
                            * Quat::from_axis_angle(axis, angle - drag.grab)
                            * parent_rotation
                            * drag.start.rotation;
                        if transform.rotation != local {
                            transform.rotation = local;
                            drag.moved = true;
                        }
                    }
                }
            }
        }
    }

    let camera_position = camera_matrix.w_axis.truncate();
    let size = gizmo_size(origin, camera_position, fovy);

    if state.drag.is_none() {
        let hovered = pointer
            .over
            .then(|| {
                hovered_handle(
                    pointer.position,
                    mode,
                    origin,
                    size,
                    pointer.rect,
                    view_proj,
                )
            })
            .flatten();
        if state.hovered != hovered {
            state.hovered = hovered;
        }
        if let (true, Some(axis), Some(ray)) = (pointer.pressed, hovered, ray) {
            let grab = match mode {
                GizmoMode::Translate => axis_param(&ray, origin, axis.direction()),
                GizmoMode::Rotate => plane_angle(&ray, origin, axis.direction()),
            };
            if let (Some(grab), Some(start)) = (grab, transforms.get_entity(target)) {
                state.captured = true;
                state.drag = Some(Drag {
                    entity: target,
                    axis,
                    mode,
                    origin,
                    start: start.clone(),
                    parent,
                    grab,
                    moved: false,
                });
            }
        }
    }

    let active = state.drag.as_ref().map(|drag| drag.axis).or(state.hovered);
    let shape = state.drag.as_ref().map_or(mode, |drag| drag.mode);
    gizmos.set_line_width(LINE_WIDTH * pointer.scale.max(1.0));
    for axis in Axis::ALL {
        let color = if active == Some(axis) {
            HIGHLIGHT
        } else {
            axis.color()
        };
        match shape {
            GizmoMode::Translate => gizmos.arrow(origin, origin + axis.direction() * size, color),
            GizmoMode::Rotate => gizmos.circle(origin, axis.direction(), size, color),
        }
    }
}

fn gizmo_shortcuts(
    mut fired: EventReader<ActionFired>,
    fly: Res<FlyCamera>,
    mut state: ResMut<GizmoState>,
) {
    for action in fired.read() {
        if fly.navigating() {
            continue;
        }
        if action.is(GizmoTranslate) {
            state.set_mode(GizmoMode::Translate);
        } else if action.is(GizmoRotate) {
            state.set_mode(GizmoMode::Rotate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_debug_gizmos::GizmoStorage;
    use concerto_ecs::{
        IntoSystem, System, World, entity::hierarchy::Children, events::event_channel::EventChannel,
    };
    use concerto_foundation::assets::AssetId;
    use concerto_window::input::actions::ActionLabel;
    use std::f32::consts::FRAC_PI_2;

    const VIEW: UIBox = UIBox {
        min: Vec2::new(100.0, 50.0),
        size: Vec2::new(800.0, 600.0),
    };

    fn ray(origin: [f32; 3], direction: [f32; 3]) -> Ray {
        Ray {
            origin: origin.into(),
            direction: Vec3::from(direction).normalize(),
        }
    }

    #[test]
    fn the_nearest_point_on_an_axis_to_a_ray() {
        let param = axis_param(
            &ray([3.0, 5.0, 4.0], [0.0, -1.0, -1.0]),
            Vec3::ZERO,
            Vec3::X,
        );
        assert!((param.unwrap() - 3.0).abs() < 1e-5);
        let offset = axis_param(
            &ray([0.0, 0.0, 9.0], [0.0, 0.0, -1.0]),
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::X,
        );
        assert!((offset.unwrap() - 2.0).abs() < 1e-5);
        assert_eq!(
            axis_param(&ray([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]), Vec3::ZERO, Vec3::X),
            None,
            "a ray along the axis has no single nearest point"
        );
    }

    #[test]
    fn the_angle_of_a_ray_on_a_plane_and_none_when_edge_on() {
        let (u, v) = ring_basis(Vec3::Z);
        for expected in [0.0_f32, 0.7, FRAC_PI_2, -2.0] {
            let point = (u * expected.cos() + v * expected.sin()) * 3.0;
            let angle = plane_angle(
                &ray([point.x, point.y, 8.0], [0.0, 0.0, -1.0]),
                Vec3::ZERO,
                Vec3::Z,
            )
            .unwrap();
            assert!((angle - expected).abs() < 1e-4, "{angle} vs {expected}");
        }
        assert_eq!(
            plane_angle(&ray([0.0, 0.0, 8.0], [1.0, 0.0, 0.01]), Vec3::ZERO, Vec3::Z),
            None
        );
        assert_eq!(
            plane_angle(&ray([0.0, 0.0, 8.0], [0.0, 0.0, 1.0]), Vec3::ZERO, Vec3::Z),
            None,
            "the plane is behind the ray"
        );
    }

    struct Fixture {
        world: World,
        document: Entity,
        root: Entity,
        camera: Camera,
        camera_matrix: Mat4,
    }

    fn fixture() -> Fixture {
        let mut world = World::new();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(GizmoState::default());
        world.insert_resource(Selection::default());
        world.insert_resource(FlyCamera::default());
        world.insert_resource(GizmoStorage::default());
        world.insert_resource(EventChannel::<ActionFired>::default());
        world.insert_resource(ViewportPointer {
            rect: VIEW,
            scale: 1.0,
            ..Default::default()
        });
        let camera = Camera {
            aspect: 4.0 / 3.0,
            ..Default::default()
        };
        let camera_transform = Transform::from_translation(Vec3::new(0.0, 0.0, 10.0));
        let camera_matrix = camera_transform.compute_matrix();
        world.spawn((
            EditorCamera,
            Camera {
                aspect: 4.0 / 3.0,
                ..Default::default()
            },
            camera_transform,
        ));
        let document = world.spawn(EditorDocument {
            asset_type: "Scene",
            title: "level".into(),
            current: None,
            pending: None,
            project_generation: 0,
            request_generation: 0,
            order: 0,
            status: String::new(),
            revision: 0,
            saved_revision: 0,
        });
        let root = world.spawn((
            Transform::IDENTITY,
            GlobalTransform::new(Mat4::IDENTITY),
            SceneRoot {
                asset_id: AssetId::new(),
                address: String::new(),
            },
            EditorOwned(document),
        ));
        Fixture {
            world,
            document,
            root,
            camera,
            camera_matrix,
        }
    }

    impl Fixture {
        fn child(&mut self, parent: Entity, local: Transform) -> Entity {
            let parent_matrix = self
                .world
                .get_component_for_entity::<GlobalTransform>(parent)
                .unwrap()
                .matrix();
            let entity = self.world.spawn((
                GlobalTransform::new(parent_matrix * local.compute_matrix()),
                local,
            ));
            self.world.entity_mut(parent).add_child(entity);
            entity
        }
        fn select(&mut self, entity: Entity) {
            self.world
                .get_resource_mut::<Selection>()
                .unwrap()
                .select_entity(entity);
        }
        fn screen(&self, point: Vec3) -> Vec2 {
            let view_proj = self.camera.build_projection_matrix() * self.camera_matrix.inverse();
            world_to_viewport(point, VIEW, view_proj).unwrap()
        }
        fn size(&self, origin: Vec3) -> f32 {
            gizmo_size(
                origin,
                self.camera_matrix.w_axis.truncate(),
                self.camera.fovy,
            )
        }
        fn frame(&mut self, position: Vec2, pressed: bool, held: bool) {
            *self.world.get_resource_mut::<ViewportPointer>().unwrap() = ViewportPointer {
                position,
                rect: VIEW,
                over: true,
                pressed,
                held,
                scale: 1.0,
            };
            self.world.insert_resource(GizmoStorage::default());
            let mut system = update_gizmo.into_system();
            system.initialize(&mut self.world);
            system.run_and_apply((), &mut self.world);
        }
        fn drag(&mut self, from: Vec3, to: Vec3) {
            let (from, to) = (self.screen(from), self.screen(to));
            self.frame(from, false, false);
            self.frame(from, true, true);
            self.frame(to, false, true);
            self.frame(to, false, false);
        }
        fn state(&self) -> &GizmoState {
            self.world.get_resource::<GizmoState>().unwrap()
        }
        fn local(&self, entity: Entity) -> Transform {
            self.world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .clone()
        }
        fn dirty(&self) -> bool {
            self.world
                .get_component_for_entity::<EditorDocument>(self.document)
                .unwrap()
                .is_dirty()
        }
        fn segments(&self) -> usize {
            self.world
                .get_resource::<GizmoStorage>()
                .unwrap()
                .segment_count()
        }
        fn set_mode(&mut self, mode: GizmoMode) {
            self.world
                .get_resource_mut::<GizmoState>()
                .unwrap()
                .set_mode(mode);
        }
    }

    #[test]
    fn the_pointer_over_an_arrow_or_ring_finds_its_axis() {
        let f = fixture();
        let view_proj = f.camera.build_projection_matrix() * f.camera_matrix.inverse();
        let size = f.size(Vec3::ZERO);
        let at = |point: Vec3, mode| {
            hovered_handle(f.screen(point), mode, Vec3::ZERO, size, VIEW, view_proj)
        };
        assert_eq!(
            at(Vec3::X * size * 0.6, GizmoMode::Translate),
            Some(Axis::X)
        );
        assert_eq!(
            at(Vec3::Y * size * 0.6, GizmoMode::Translate),
            Some(Axis::Y)
        );
        assert_eq!(
            at(Vec3::new(size, size, 0.0), GizmoMode::Translate),
            None,
            "between the arrows and beyond reach of both"
        );
        let diagonal = Vec3::new(size, size, 0.0) * std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(at(diagonal, GizmoMode::Rotate), Some(Axis::Z));
        assert_eq!(at(diagonal * 0.5, GizmoMode::Rotate), None);
        assert_eq!(
            at(Vec3::new(0.0, size * 0.5, 0.0), GizmoMode::Rotate),
            Some(Axis::X),
            "the X ring is seen edge-on as a vertical line"
        );
    }

    #[test]
    fn nothing_is_drawn_or_hovered_without_a_movable_selection() {
        let mut f = fixture();
        f.frame(f.screen(Vec3::ZERO), false, false);
        assert_eq!(f.segments(), 0);
        f.select(f.root);
        f.frame(f.screen(Vec3::ZERO), false, false);
        assert_eq!(f.segments(), 0, "the scene root has no gizmo");
        let entity = f.child(f.root, Transform::IDENTITY);
        f.select(entity);
        f.frame(f.screen(Vec3::ZERO), false, false);
        assert_eq!(f.segments(), 15, "three arrows of five lines each");
        f.set_mode(GizmoMode::Rotate);
        f.frame(f.screen(Vec3::ZERO), false, false);
        assert_eq!(f.segments(), 96, "three rings");
        f.world
            .get_resource_mut::<FlyCamera>()
            .unwrap()
            .set_looking(true);
        f.frame(f.screen(Vec3::ZERO), false, false);
        assert_eq!(f.segments(), 0);
    }

    #[test]
    fn a_translate_drag_moves_the_entity_along_the_axis_and_marks_the_document_once() {
        let mut f = fixture();
        let entity = f.child(
            f.root,
            Transform::from_translation(Vec3::new(1.0, 0.5, 0.0)),
        );
        f.select(entity);
        let origin = Vec3::new(1.0, 0.5, 0.0);
        let grab = origin + Vec3::X * f.size(origin) * 0.5;

        let (from, to) = (f.screen(grab), f.screen(grab + Vec3::X * 2.0));
        f.frame(from, false, false);
        assert_eq!(f.state().hovered(), Some(Axis::X));
        f.frame(from, true, true);
        assert!(f.state().dragging());
        assert!(f.state().captured());
        f.frame(to, false, true);
        assert!(!f.dirty(), "the document is marked when the drag ends");
        f.frame(to, false, false);

        let moved = f.local(entity).translation;
        assert!(
            (moved - Vec3::new(3.0, 0.5, 0.0)).length() < 1e-3,
            "{moved:?}"
        );
        assert!(f.dirty());
        assert!(!f.state().dragging());
        assert_eq!(
            f.world
                .get_component_for_entity::<EditorDocument>(f.document)
                .unwrap()
                .revision,
            1
        );
    }

    #[test]
    fn a_press_on_a_handle_without_movement_changes_nothing() {
        let mut f = fixture();
        let entity = f.child(f.root, Transform::IDENTITY);
        f.select(entity);
        let grab = Vec3::Y * f.size(Vec3::ZERO) * 0.5;
        f.drag(grab, grab);
        assert_eq!(f.local(entity).translation, Vec3::ZERO);
        assert!(!f.dirty());
        assert!(f.state().captured(), "the click that follows is not a pick");
        f.frame(f.screen(Vec3::new(3.0, 3.0, 0.0)), true, true);
        assert!(!f.state().captured());
    }

    #[test]
    fn a_rotate_drag_turns_the_entity_by_the_angle_swept() {
        let mut f = fixture();
        let entity = f.child(f.root, Transform::IDENTITY);
        f.select(entity);
        f.set_mode(GizmoMode::Rotate);
        let (u, v) = ring_basis(Vec3::Z);
        let size = f.size(Vec3::ZERO);
        let on_ring = |angle: f32| (u * angle.cos() + v * angle.sin()) * size;
        f.drag(on_ring(0.6), on_ring(0.6 + FRAC_PI_2));
        let rotation = f.local(entity).rotation;
        assert!(
            rotation.dot(Quat::from_rotation_z(FRAC_PI_2)).abs() > 0.99999,
            "{rotation:?}"
        );
        assert!(f.dirty());
    }

    #[test]
    fn drags_come_out_right_under_a_rotated_and_scaled_parent() {
        let mut f = fixture();
        let parent_local = Transform::from_translation_rotation_scale(
            Vec3::new(-1.0, 0.0, 0.0),
            Quat::from_rotation_y(FRAC_PI_2),
            Vec3::splat(2.0),
        );
        let parent = f.child(f.root, parent_local.clone());
        let entity = f.child(
            parent,
            Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        );
        f.select(entity);
        let parent_matrix = parent_local.compute_matrix();
        let world = parent_matrix.transform_point3(Vec3::new(0.0, 0.5, 0.0));

        let grab = world + Vec3::X * f.size(world) * 0.5;
        f.drag(grab, grab + Vec3::X * 1.5);
        let moved = parent_matrix.transform_point3(f.local(entity).translation);
        assert!(
            (moved - (world + Vec3::X * 1.5)).length() < 1e-3,
            "{moved:?}"
        );

        let world_matrix = parent_matrix * f.local(entity).compute_matrix();
        f.world
            .get_component_for_entity_mut::<GlobalTransform>(entity)
            .unwrap()
            .set_matrix(world_matrix);
        f.set_mode(GizmoMode::Rotate);
        let (u, v) = ring_basis(Vec3::Z);
        let size = f.size(moved);
        let on_ring = |angle: f32| moved + (u * angle.cos() + v * angle.sin()) * size;
        f.drag(on_ring(0.6), on_ring(0.6 + FRAC_PI_2));
        let world_rotation =
            parent_matrix.to_scale_rotation_translation().1 * f.local(entity).rotation;
        let expected = Quat::from_rotation_z(FRAC_PI_2) * Quat::from_rotation_y(FRAC_PI_2);
        assert!(
            world_rotation.dot(expected).abs() > 0.99999,
            "{world_rotation:?}"
        );
    }

    #[test]
    fn mode_keys_switch_the_gizmo_and_are_ignored_while_flying() {
        let mut f = fixture();
        let fire =
            |f: &mut Fixture, action: concerto_window::input::actions::InternedActionLabel| {
                let channel = f
                    .world
                    .get_resource_mut::<EventChannel<ActionFired>>()
                    .unwrap();
                channel.update();
                channel.update();
                channel.push_event(ActionFired { action });
                let mut system = gizmo_shortcuts.into_system();
                system.initialize(&mut f.world);
                system.run_and_apply((), &mut f.world);
                f.world.get_resource::<GizmoState>().unwrap().mode()
            };
        assert_eq!(fire(&mut f, GizmoRotate.intern()), GizmoMode::Rotate);
        assert_eq!(fire(&mut f, GizmoTranslate.intern()), GizmoMode::Translate);
        f.world
            .get_resource_mut::<FlyCamera>()
            .unwrap()
            .set_looking(true);
        assert_eq!(fire(&mut f, GizmoRotate.intern()), GizmoMode::Translate);
    }
}
