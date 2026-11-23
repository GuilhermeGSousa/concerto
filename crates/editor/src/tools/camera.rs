//! The editor camera, its fly navigation, and framing.
use concerto_app::{
    App,
    edit_time::AtEditTime,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::entity::hierarchy::Children;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    events::event_reader::EventReader,
    signal::{On, Signal},
};
use concerto_foundation::{
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::Aabb;
use concerto_render::{ExclusiveCamera, components::camera::Camera};
use concerto_window::{
    events::{ScrollDelta, WindowEvent},
    input::{Input, KeyCode, MouseButton, PhysicalKey},
    plugin::Window,
};
use concerto_world_grid::plugin::WorldGridPlugin;
use glam::{Quat, Vec2, Vec3};
use std::collections::HashSet;

use crate::tools::selection::Selection;

#[derive(Component)]
pub(crate) struct EditorCamera;

#[derive(Component)]
pub struct EditorHelper;

/// Where the pointer is over the picture the tools draw into.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportPointer {
    /// Logical pixels from the picture's top-left corner.
    pub position: Vec2,
    /// The picture's size in logical pixels.
    pub size: Vec2,
    /// Whether the pointer is over the picture.
    pub over: bool,
    /// The left button went down this frame.
    pub pressed: bool,
    /// The left button is down.
    pub held: bool,
    /// Physical pixels per logical pixel.
    pub scale: f32,
}

/// A first-person fly camera with Unreal's controls.
#[derive(Resource)]
pub struct FlyCamera {
    pub position: Vec3,
    /// Radians around world up, applied before pitch.
    pub yaw: f32,
    pub pitch: f32,
    /// Metres per second, adjusted by the wheel while looking around.
    pub speed: f32,
    looking: bool,
    panning: bool,
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 2.5, 8.0),
            yaw: 0.0,
            pitch: -0.15,
            speed: 6.0,
            looking: false,
            panning: false,
        }
    }
}

impl FlyCamera {
    /// Whether the pointer is currently steering the camera.
    pub(crate) fn navigating(&self) -> bool {
        self.looking || self.panning
    }

    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    pub(crate) fn forward(&self) -> Vec3 {
        self.rotation() * -Vec3::Z
    }

    pub(crate) fn placed(&self, transform: &Transform) -> bool {
        transform.translation == self.position && transform.rotation == self.rotation()
    }
}

const LOOK_SENSITIVITY: f32 = 0.0025;
const MAX_PITCH: f32 = 1.55;
const SPEED_STEP: f32 = 1.15;
const MIN_SPEED: f32 = 0.05;
const MAX_SPEED: f32 = 500.0;
const BOOST: f32 = 4.0;
const PAN_PER_PIXEL: f32 = 0.0015;
const DOLLY_PER_NOTCH: f32 = 0.25;

#[derive(Resource, Default)]
pub struct ViewportCommands {
    pub frame_selection: bool,
    pub frame_all: bool,
}

pub(crate) fn build(app: &mut App) {
    app.insert_resource(FlyCamera::default())
        .insert_resource(ViewportCommands::default())
        .insert_resource(ViewportPointer::default())
        .add_system(Update, fill_pointer_from_window.at_edit_time())
        .add_system(
            Update,
            navigate.after(fill_pointer_from_window).at_edit_time(),
        )
        .add_system(
            Update,
            fit_camera_to_picture
                .after(fill_pointer_from_window)
                .at_edit_time(),
        )
        .add_system(LateUpdate, frame_requested_bounds.at_edit_time())
        .add_system(LateUpdate, zoom.at_edit_time())
        .add_listener(frame_selection.at_edit_time())
        .add_listener(frame_all.at_edit_time())
        .add_listener(set_camera_pose.at_edit_time());
    if app.is_plugin_added::<WorldGridPlugin>() {
        app.add_system(Startup, spawn_camera_and_grid.at_edit_time());
    } else {
        app.add_system(Startup, spawn_camera.at_edit_time());
    }
}

pub(crate) fn fill_pointer_from_window(
    input: Res<Input>,
    window: Res<Window>,
    mut pointer: ResMut<ViewportPointer>,
) {
    let scale = window.scale_factor() as f32;
    let (width, height) = window.size();
    let position = window.logical_pointer_position(&input);
    let size = Vec2::new(width as f32, height as f32) / scale;
    let next = ViewportPointer {
        position,
        size,
        over: position.cmpge(Vec2::ZERO).all() && position.cmple(size).all(),
        pressed: input.is_mouse_button_just_pressed(MouseButton::Left),
        held: input.is_mouse_button_held(MouseButton::Left),
        scale,
    };
    if *pointer != next {
        *pointer = next;
    }
}

/// Moves the camera to show the selected entity and its descendants.
pub struct FrameSelection;

impl Signal for FrameSelection {}

/// Moves the camera to show the whole scene.
pub struct FrameAll;

impl Signal for FrameAll {}

/// Places the camera at `position`, turned by `yaw` and `pitch` as [`FlyCamera`] holds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetCameraPose {
    /// Where the camera goes.
    pub position: Vec3,
    /// Radians around world up; 0 faces -Z and a positive yaw turns left.
    pub yaw: f32,
    /// Radians up from level, kept within what a look can reach.
    pub pitch: f32,
}

impl Signal for SetCameraPose {}

impl SetCameraPose {
    /// The pose at `position` that faces `target`, or `None` when they are the same point.
    pub fn looking_at(position: Vec3, target: Vec3) -> Option<Self> {
        let direction = (target - position).try_normalize()?;
        let yaw = if Vec2::new(direction.x, direction.z).length() > 1e-6 {
            (-direction.x).atan2(-direction.z)
        } else {
            0.0
        };
        Some(Self {
            position,
            yaw,
            pitch: direction.y.clamp(-1.0, 1.0).asin(),
        })
    }
}

fn set_camera_pose(on: On<SetCameraPose>, mut fly: ResMut<FlyCamera>) {
    let pose = on.signal();
    fly.position = pose.position;
    fly.yaw = pose.yaw;
    fly.pitch = pose.pitch.clamp(-MAX_PITCH, MAX_PITCH);
}

fn frame_selection(_: On<FrameSelection>, mut commands: ResMut<ViewportCommands>) {
    commands.frame_selection = true;
}

fn frame_all(_: On<FrameAll>, mut commands: ResMut<ViewportCommands>) {
    commands.frame_all = true;
}

pub(crate) const SKY: Color = Camera::DEFAULT_CLEAR_COLOR;
const GRID_LINES: Color = Color::srgba(0.247, 0.259, 0.322, 0.6);
const GRID_SURFACE: Color = Color::srgba(0.247, 0.259, 0.322, 0.0);

fn spawn_editor_camera(cmd: &mut CommandQueue, fly: &FlyCamera) {
    let mut transform = Transform::IDENTITY;
    apply_fly_transform(fly, &mut transform);
    let camera = cmd.spawn((
        Camera {
            aspect: 4.0 / 3.0,
            clear_color: SKY,
            ..Default::default()
        },
        transform,
        EditorCamera,
        EditorHelper,
    ));
    let camera = camera.entity();
    cmd.insert_resource(ExclusiveCamera(camera));
}

fn spawn_camera(mut cmd: CommandQueue, fly: Res<FlyCamera>) {
    spawn_editor_camera(&mut cmd, &fly);
}

pub(crate) fn editor_grid() -> concerto_world_grid::WorldGrid {
    concerto_world_grid::WorldGrid {
        line_color: GRID_LINES,
        surface_color: GRID_SURFACE,
        lit: false,
        ..Default::default()
    }
}

fn spawn_camera_and_grid(mut cmd: CommandQueue, fly: Res<FlyCamera>) {
    spawn_editor_camera(&mut cmd, &fly);
    cmd.spawn((editor_grid(), EditorHelper));
}

fn apply_fly_transform(fly: &FlyCamera, transform: &mut Transform) {
    transform.translation = fly.position;
    transform.rotation = fly.rotation();
}

fn navigate(
    input: Res<Input>,
    time: Res<Time>,
    mut window: ResMut<Window>,
    pointer: Res<ViewportPointer>,
    mut cameras: Query<(&EditorCamera, &mut Transform)>,
    mut fly: ResMut<FlyCamera>,
) {
    if input.is_mouse_button_just_pressed(MouseButton::Right) && pointer.over {
        fly.looking = true;
        capture_pointer(&mut window, true);
    }
    if fly.looking && !input.is_mouse_button_held(MouseButton::Right) {
        fly.looking = false;
        capture_pointer(&mut window, false);
    }
    if input.is_mouse_button_just_pressed(MouseButton::Middle) && pointer.over {
        fly.panning = true;
    }
    if fly.panning && !input.is_mouse_button_held(MouseButton::Middle) {
        fly.panning = false;
    }

    let delta = input.mouse_delta();
    if fly.looking {
        fly.yaw -= delta.x * LOOK_SENSITIVITY;
        fly.pitch = (fly.pitch - delta.y * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }
    if fly.panning {
        let rotation = fly.rotation();
        let scale = fly.speed * PAN_PER_PIXEL;
        fly.position +=
            (rotation * Vec3::X) * -delta.x * scale + (rotation * Vec3::Y) * delta.y * scale;
    }
    if fly.looking {
        let step = fly.speed * speed_scale(&input) * time.delta().as_secs_f32();
        let movement = movement(&input, fly.rotation()) * step;
        fly.position += movement;
    }

    for (_, mut transform) in cameras.iter_mut() {
        apply_fly_transform(&fly, &mut transform);
    }
}

fn movement(input: &Input, rotation: Quat) -> Vec3 {
    let held = |code| input.is_held(PhysicalKey::Code(code));
    let axis = |positive, negative| match (held(positive), held(negative)) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        _ => 0.0,
    };
    let local = Vec3::new(
        axis(KeyCode::KeyD, KeyCode::KeyA),
        0.0,
        -axis(KeyCode::KeyW, KeyCode::KeyS),
    );
    let vertical = Vec3::Y * axis(KeyCode::KeyE, KeyCode::KeyQ);
    (rotation * local + vertical).normalize_or_zero()
}

fn speed_scale(input: &Input) -> f32 {
    let shift = input.is_held(PhysicalKey::Code(KeyCode::ShiftLeft))
        || input.is_held(PhysicalKey::Code(KeyCode::ShiftRight));
    if shift { BOOST } else { 1.0 }
}

fn capture_pointer(window: &mut Window, capture: bool) {
    window.settings.cursor = concerto_window::plugin::CursorMode {
        visible: !capture,
        grabbed: capture,
    };
}

pub(crate) fn subtree(root: Entity, children: &Query<&Children>) -> HashSet<Entity> {
    let mut found = HashSet::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if !found.insert(entity) {
            continue;
        }
        if let Some(kids) = children.get_entity(entity) {
            stack.extend(kids.iter().copied());
        }
    }
    found
}

fn frame_requested_bounds(
    bounded: Query<(Entity, &Aabb, &GlobalTransform)>,
    children: Query<&Children>,
    mut cameras: Query<(&EditorCamera, &mut Camera, &mut Transform)>,
    selection: Res<Selection>,
    mut commands: ResMut<ViewportCommands>,
    mut fly: ResMut<FlyCamera>,
) {
    let bounds = if commands.frame_all {
        scene_bounds(&bounded, None)
    } else if commands.frame_selection {
        selection
            .active()
            .map(|entity| subtree(entity, &children))
            .and_then(|set| scene_bounds(&bounded, Some(&set)))
    } else {
        None
    };
    commands.frame_selection = false;
    commands.frame_all = false;

    let Some(bounds) = bounds else {
        return;
    };
    let radius = bounds.extent().length().max(0.02) * 0.5;
    let fovy = cameras
        .iter_mut()
        .next()
        .map(|(_, camera, _)| camera.fovy)
        .unwrap_or(0.78);
    let distance = (radius / (fovy * 0.5).tan() * 1.25).clamp(0.02, 100_000.0);
    fly.position = bounds.center() - fly.forward() * distance;
    for (_, mut camera, mut transform) in cameras.iter_mut() {
        apply_fly_transform(&fly, &mut transform);
        let far = distance + radius * 2.0;
        if camera.zfar < far {
            camera.zfar = far;
        }
    }
}

fn scene_bounds(
    nodes: &Query<(Entity, &Aabb, &GlobalTransform)>,
    subtree: Option<&HashSet<Entity>>,
) -> Option<Aabb> {
    let mut result: Option<Aabb> = None;
    for (entity, bounds, transform) in nodes.iter() {
        if subtree.is_some_and(|set| !set.contains(&entity)) {
            continue;
        }
        let bounds = bounds.transformed(transform.matrix());
        result = Some(match result {
            Some(current) => Aabb {
                min: current.min.min(bounds.min),
                max: current.max.max(bounds.max),
            },
            None => bounds,
        });
    }
    result
}

fn zoom(
    mut events: EventReader<WindowEvent>,
    pointer: Res<ViewportPointer>,
    mut cameras: Query<(&EditorCamera, &mut Transform)>,
    mut fly: ResMut<FlyCamera>,
) {
    if !pointer.over {
        return;
    }
    let mut moved = false;
    for event in events.read() {
        if let WindowEvent::MouseWheel { delta } = event {
            let amount = match delta {
                ScrollDelta::Lines(lines) => lines.y,
                ScrollDelta::Pixels(pixels) => pixels.y / 32.0,
            };
            if fly.looking {
                fly.speed = (fly.speed * SPEED_STEP.powf(amount)).clamp(MIN_SPEED, MAX_SPEED);
            } else {
                let step = fly.forward() * amount * fly.speed * DOLLY_PER_NOTCH;
                fly.position += step;
                moved = true;
            }
        }
    }
    if moved {
        for (_, mut transform) in cameras.iter_mut() {
            apply_fly_transform(&fly, &mut transform);
        }
    }
}

fn fit_camera_to_picture(
    pointer: Res<ViewportPointer>,
    mut cameras: Query<(&EditorCamera, &mut Camera)>,
) {
    if pointer.size.x < 1.0 || pointer.size.y < 1.0 {
        return;
    }
    let aspect = pointer.size.x / pointer.size.y;
    for (_, mut camera) in cameras.iter_mut() {
        if camera.aspect != aspect {
            camera.aspect = aspect;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_ecs::{IntoSystem, System, World, events::event_channel::EventChannel};
    use glam::Mat4;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    #[test]
    fn the_spawned_editor_camera_is_the_exclusive_camera() {
        for spawn in [spawn_camera, spawn_camera_and_grid] {
            let mut world = World::new();
            world.insert_resource(FlyCamera::default());
            run(&mut world, spawn);

            let editor_camera = world
                .query::<(Entity, &EditorCamera), ()>()
                .iter(&mut world)
                .map(|(entity, _)| entity)
                .next()
                .unwrap();
            assert_eq!(
                world.get_resource::<ExclusiveCamera>().copied(),
                Some(ExclusiveCamera(editor_camera))
            );
        }
    }

    #[test]
    fn framing_looks_at_the_selection_or_everything_and_keeps_it_inside_the_far_plane() {
        let mut world = World::new();
        world.insert_resource(ViewportCommands::default());
        world.insert_resource(FlyCamera::default());
        world.insert_resource(Selection::default());
        world.add_listener(crate::tools::selection::select);
        world.add_listener(frame_selection);
        world.add_listener(frame_all);
        world.spawn((EditorCamera, Camera::default(), Transform::IDENTITY));
        let mut bounded = |x: f32| {
            world.spawn((
                Aabb {
                    min: Vec3::splat(-1.0),
                    max: Vec3::splat(1.0),
                },
                GlobalTransform::new(Mat4::from_translation(Vec3::new(x, 0.0, 0.0))),
            ))
        };
        let near = bounded(40.0);
        bounded(-40.0);
        let looks_at = |world: &World, point: Vec3| {
            let fly = world.get_resource::<FlyCamera>().unwrap();
            ((point - fly.position).normalize() - fly.forward()).length() < 1e-3
        };
        let far_plane = |world: &mut World| {
            world
                .query::<&Camera, ()>()
                .iter(world)
                .next()
                .unwrap()
                .zfar
        };

        world.trigger(FrameAll);
        run(&mut world, frame_requested_bounds);
        assert!(looks_at(&world, Vec3::ZERO));
        let position = world.get_resource::<FlyCamera>().unwrap().position;
        let farthest = [Vec3::new(41.0, 1.0, 1.0), Vec3::new(-41.0, 1.0, 1.0)]
            .map(|corner| (corner - position).length())
            .into_iter()
            .fold(0.0, f32::max);
        let all = far_plane(&mut world);
        assert!(farthest < all, "{farthest} lies beyond the far plane {all}");

        world.trigger(crate::tools::selection::Select::only(near));
        world.trigger(FrameSelection);
        run(&mut world, frame_requested_bounds);
        assert!(looks_at(&world, Vec3::new(40.0, 0.0, 0.0)));
        assert_eq!(
            far_plane(&mut world),
            all,
            "framing less never clips the rest"
        );
    }

    fn pointing(over: bool) -> World {
        let mut world = World::new();
        world.insert_resource(Input::new());
        world.insert_resource(Time::new());
        world.insert_resource(Window::new((800, 600), 1.0));
        world.insert_resource(FlyCamera::default());
        world.insert_resource(ViewportCommands::default());
        world.insert_resource(EventChannel::<WindowEvent>::default());
        world.insert_resource(ViewportPointer {
            size: Vec2::new(800.0, 600.0),
            over,
            scale: 1.0,
            ..Default::default()
        });
        world.spawn((EditorCamera, Camera::default(), Transform::IDENTITY));
        world
    }

    #[test]
    fn the_mouse_steers_the_camera_only_over_the_picture_and_until_its_button_is_released() {
        for over in [false, true] {
            let mut world = pointing(over);
            world
                .get_resource_mut::<Input>()
                .unwrap()
                .update_mouse_button(MouseButton::Right, true);
            world
                .get_resource_mut::<EventChannel<WindowEvent>>()
                .unwrap()
                .push_event(WindowEvent::MouseWheel {
                    delta: ScrollDelta::Lines(Vec2::Y),
                });
            run(&mut world, zoom);
            let mut navigation = navigate.into_system();
            navigation.initialize(&mut world);
            navigation.run_and_apply((), &mut world);

            let grabbed = |world: &World| {
                world
                    .get_resource::<Window>()
                    .unwrap()
                    .settings
                    .cursor
                    .grabbed
            };
            let fly = world.get_resource::<FlyCamera>().unwrap();
            assert_eq!(fly.navigating(), over);
            assert_eq!(fly.position != FlyCamera::default().position, over);
            assert_eq!(grabbed(&world), over);

            let input = world.get_resource_mut::<Input>().unwrap();
            input.update();
            input.release_all();
            input.update();
            navigation.run_and_apply((), &mut world);
            assert!(!world.get_resource::<FlyCamera>().unwrap().navigating());
            assert!(!grabbed(&world));
        }
    }

    #[test]
    fn the_pointer_is_read_in_the_pictures_logical_pixels_and_shapes_the_camera() {
        let mut world = pointing(true);
        let mut input = Input::new();
        input.update_mouse_position(600.0, 400.0);
        input.update_mouse_button(MouseButton::Left, true);
        world.insert_resource(input);
        world.insert_resource(Window::new((1600, 1000), 2.0));

        run(&mut world, fill_pointer_from_window);
        run(&mut world, fit_camera_to_picture);

        assert_eq!(
            *world.get_resource::<ViewportPointer>().unwrap(),
            ViewportPointer {
                position: Vec2::new(300.0, 200.0),
                size: Vec2::new(800.0, 500.0),
                over: true,
                pressed: true,
                held: true,
                scale: 2.0,
            }
        );
        let aspect = world
            .query::<(&EditorCamera, &Camera), ()>()
            .iter(&mut world)
            .next()
            .unwrap()
            .1
            .aspect;
        assert_eq!(aspect, 1.6);
    }

    #[test]
    fn a_pose_looking_at_a_point_faces_it_within_the_pitch_a_look_can_reach() {
        let position = Vec3::new(1.0, 2.0, 3.0);
        for direction in [Vec3::NEG_Z, Vec3::Z, Vec3::new(1.0, -1.0, 0.5), Vec3::Y] {
            let mut world = World::new();
            world.insert_resource(FlyCamera::default());
            world.add_listener(set_camera_pose);
            world.trigger(SetCameraPose::looking_at(position, position + direction * 4.0).unwrap());
            let fly = world.get_resource::<FlyCamera>().unwrap();
            assert_eq!(fly.position, position);
            assert!(fly.pitch.abs() <= MAX_PITCH);
            assert!(
                (fly.forward() - direction.normalize()).length() < 0.03,
                "{direction} gives {}",
                fly.forward()
            );
        }
        assert!(SetCameraPose::looking_at(Vec3::ONE, Vec3::ONE).is_none());
    }

    #[test]
    fn movement_is_relative_to_where_the_camera_faces() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut input = Input::new();
        input.update_key_input(PhysicalKey::Code(KeyCode::KeyW), true);
        assert!((movement(&input, rotation) - Vec3::NEG_X).length() < 0.001);
    }
}
