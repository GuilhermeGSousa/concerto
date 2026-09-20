//! Texture-backed editor viewport and fly navigation.
use app::{
    schedule_groups::{LateUpdate, Startup, Update},
    App, Plugin,
};
use color::Color;
use ecs::entity::hierarchy::Children;
use ecs::system::NonSendMarker;
use ecs::{
    command::CommandQueue, events::event_reader::EventReader, Component, Entity, Query, Res,
    ResMut, Resource,
};
use essential::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use glam::{Quat, Vec3};
use mesh::{mesh::Aabb, Mesh, MeshComponent};
use render::{
    assets::texture::Texture,
    components::{
        camera::{Camera, RenderTarget},
        light::Light,
        render_entity::SyncWithRenderWorld,
    },
};
use std::collections::HashSet;
use ui::{
    interaction::{HoveredNode, Interactable},
    material::UIMaterial,
    node::{UILayout, UINode},
    transform::UIValue,
    UIViewport,
};
use window::{
    input::{
        actions::{ActionFired, ActionMap},
        Input, KeyCode, MouseButton, PhysicalKey,
    },
    winit_events::WindowEvent,
};

use crate::actions::{FrameAll, FrameSelected, ViewportContext};
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::selection::Selection;

#[derive(Component)]
pub struct ViewportRegion;

#[derive(Component)]
struct EditorCamera;

#[derive(Component)]
pub struct EditorHelper;

#[derive(Resource)]
pub struct EditorViewport {
    pub texture: AssetHandle<Texture>,
}

/// A first-person fly camera with Unreal's controls.
///
/// Navigation only happens with the right button held, which is what keeps
/// `W` typed into a search field from flying the view across the level.
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
    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    fn forward(&self) -> Vec3 {
        self.rotation() * -Vec3::Z
    }
}

/// Radians of rotation per pixel of pointer travel.
const LOOK_SENSITIVITY: f32 = 0.0025;
/// Straight up is a singularity for a yaw/pitch camera; stop just short of it.
const MAX_PITCH: f32 = 1.55;
/// Multiplier per wheel notch while the right button is held.
const SPEED_STEP: f32 = 1.15;
const MIN_SPEED: f32 = 0.05;
const MAX_SPEED: f32 = 500.0;
/// What holding shift does to the fly speed.
const BOOST: f32 = 4.0;
/// Metres of pan per pixel, per metre per second of fly speed.
const PAN_PER_PIXEL: f32 = 0.0015;
/// Metres of dolly per wheel notch, per metre per second of fly speed.
const DOLLY_PER_NOTCH: f32 = 0.25;

#[derive(Resource, Default)]
pub struct ViewportCommands {
    pub frame_all: bool,
    pub(crate) reset: bool,
    pub(crate) release_navigation: bool,
}

pub struct ViewportPlugin;
impl Plugin for ViewportPlugin {
    fn build(&self, app: &mut App) {
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Scene",
            region: Region::Scene,
        });
        let texture = app
            .get_resource::<AssetServer>()
            .expect("ViewportPlugin requires AssetManagerPlugin")
            .add(Texture::render_target(800, 600));
        app.insert_resource(EditorViewport { texture })
            .insert_resource(FlyCamera::default())
            .insert_resource(ViewportCommands::default())
            .add_system(Startup, spawn_camera)
            .add_system(Startup, build_panel)
            .add_system(Update, navigate)
            .add_system(Update, sync_viewport_size)
            .add_system(LateUpdate, sync_viewport_context)
            .add_system(LateUpdate, frame_requested_bounds)
            .add_system(LateUpdate, zoom);
    }
}

pub const PANEL_ID: &str = "rabbithole.scene";

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, viewport: Res<EditorViewport>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &viewport);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, viewport: &EditorViewport) -> Entity {
    let entity = cmd
        .spawn((
            UINode {
                flex_grow: 1.0,
                min_width: UIValue::Px(320.0),
                min_height: UIValue::Px(240.0),
                ..Default::default()
            },
            // White fill so the sampled target comes through unmodified.
            UIMaterial::flat(Color::WHITE),
            UIViewport {
                texture: viewport.texture.clone(),
            },
            Interactable,
            ViewportRegion,
        ))
        .entity();
    cmd.add_child(parent, entity);
    entity
}

/// The sky behind the scene: Nocturne's canvas lifted a shade, so the viewport
/// reads as depth behind the cards rather than as another panel beside them.
const SKY: Color = Color::srgba(0.086, 0.094, 0.157, 1.0);
/// The ground plane, a shade below the sky for the same reason.
const GROUND: Color = Color::srgba(0.055, 0.063, 0.114, 1.0);
/// Grid lines, the palette's border colour at the strength of a hint.
const GRID_LINES: Color = Color::srgba(0.247, 0.259, 0.322, 0.6);

fn spawn_camera(mut cmd: CommandQueue, viewport: Res<EditorViewport>, fly: Res<FlyCamera>) {
    let mut transform = Transform::IDENTITY;
    apply_fly_transform(&fly, &mut transform);
    cmd.spawn((
        Camera {
            aspect: 4.0 / 3.0,
            clear_color: SKY,
            render_target: RenderTarget::texture(viewport.texture.clone()),
            ..Default::default()
        },
        transform,
        SyncWithRenderWorld,
        EditorCamera,
        EditorHelper,
    ));
    // Both of these draw, so both have to reach the render world; without the
    // marker they exist only in the main world and are silently never drawn.
    cmd.spawn((
        world_grid::WorldGrid {
            line_color: GRID_LINES,
            surface_color: GROUND,
            ..Default::default()
        },
        SyncWithRenderWorld,
        EditorHelper,
    ));
    cmd.spawn((
        Light::directional_light().with_intensity(3.0),
        Transform::from_rotation(Quat::from_rotation_x(-0.8) * Quat::from_rotation_y(-0.5)),
        SyncWithRenderWorld,
        EditorHelper,
    ));
}

/// Apply workspace navigation requests. The caller releases native capture on
/// the window thread when this returns true.
pub(crate) fn apply_workspace_navigation(
    commands: &mut ViewportCommands,
    fly: &mut FlyCamera,
) -> bool {
    if !commands.release_navigation && !commands.reset {
        return false;
    }
    let captured = fly.looking;
    fly.looking = false;
    fly.panning = false;
    if commands.reset {
        *fly = FlyCamera::default();
        commands.frame_all = false;
    }
    commands.reset = false;
    commands.release_navigation = false;
    captured
}

fn apply_fly_transform(fly: &FlyCamera, transform: &mut Transform) {
    transform.translation = fly.position;
    transform.rotation = fly.rotation();
}

#[allow(clippy::too_many_arguments)]
fn navigate(
    // Capturing the pointer hands the work to the window's thread and waits
    // for the result; from a worker that wait never ends on Windows.
    _: NonSendMarker,
    input: Res<Input>,
    time: Res<Time>,
    window: Res<window::plugin::Window>,
    hovered: Res<HoveredNode>,
    regions: Query<&ViewportRegion>,
    cameras: Query<(&EditorCamera, &mut Transform)>,
    mut fly: ResMut<FlyCamera>,
    mut commands: ResMut<ViewportCommands>,
) {
    if commands.release_navigation || commands.reset {
        if apply_workspace_navigation(&mut commands, &mut fly) {
            capture_pointer(&window, false);
        }
    }
    let over_viewport = (**hovered).is_some_and(|entity| regions.get_entity(entity).is_some());
    if input.is_mouse_button_just_pressed(MouseButton::Right) && over_viewport {
        fly.looking = true;
        capture_pointer(&window, true);
    }
    if input.is_mouse_button_just_released(MouseButton::Right) {
        fly.looking = false;
        capture_pointer(&window, false);
    }
    if input.is_mouse_button_just_pressed(MouseButton::Middle) && over_viewport {
        fly.panning = true;
    }
    if input.is_mouse_button_just_released(MouseButton::Middle) {
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

    for (_, mut transform) in cameras.iter() {
        apply_fly_transform(&fly, &mut transform);
    }
}

/// WASD in camera space, `E`/`Q` along world up, normalised so a diagonal is
/// not faster than a straight line.
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
    if shift {
        BOOST
    } else {
        1.0
    }
}

/// Confines the pointer while looking around, so a long turn does not end with
/// the cursor outside the window and the view stuck.
fn capture_pointer(window: &window::plugin::Window, capture: bool) {
    let handle = &window.window_handle;
    handle.set_cursor_visible(!capture);
    let mode = if capture {
        winit::window::CursorGrabMode::Confined
    } else {
        winit::window::CursorGrabMode::None
    };
    if handle.set_cursor_grab(mode).is_err() && capture {
        // Wayland and some X setups only offer the locked mode.
        let _ = handle.set_cursor_grab(winit::window::CursorGrabMode::Locked);
    }
}

/// Every entity in `root`'s subtree, including `root`.
fn subtree(root: Entity, children: &Query<&Children>) -> HashSet<Entity> {
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

/// The viewport claims its keys only while the pointer is over it.
fn sync_viewport_context(
    hovered: Res<HoveredNode>,
    regions: Query<&ViewportRegion>,
    mut actions: ResMut<ActionMap>,
) {
    let over = (**hovered).is_some_and(|entity| regions.get_entity(entity).is_some());
    if over {
        actions.push_context(ViewportContext);
    } else {
        actions.pop_context(ViewportContext);
    }
}

#[allow(clippy::too_many_arguments)]
fn frame_requested_bounds(
    mut fired: EventReader<ActionFired>,
    meshes: Res<AssetStore<Mesh>>,
    mesh_nodes: Query<(Entity, &MeshComponent, &GlobalTransform)>,
    children: Query<&Children>,
    cameras: Query<(&EditorCamera, &Camera, &mut Transform)>,
    selection: Res<Selection>,
    mut commands: ResMut<ViewportCommands>,
    mut fly: ResMut<FlyCamera>,
) {
    let mut frame_selected = false;
    for action in fired.read() {
        frame_selected |= action.is(FrameSelected);
        commands.frame_all |= action.is(FrameAll);
    }
    // Framing is something you ask for. Selecting an entity in the tree moves
    // the inspector, not the camera.
    let bounds = if commands.frame_all {
        scene_bounds(&mesh_nodes, &meshes, None)
    } else if frame_selected {
        selection
            .entity()
            .map(|entity| subtree(entity, &children))
            .and_then(|set| scene_bounds(&mesh_nodes, &meshes, Some(&set)))
    } else {
        None
    };
    commands.frame_all = false;

    let Some(bounds) = bounds else {
        return;
    };
    let radius = bounds.extent().length().max(0.02) * 0.5;
    let fovy = cameras
        .iter()
        .next()
        .map(|(_, camera, _)| camera.fovy)
        .unwrap_or(0.78);
    // Pull back along the direction the camera is already looking, so framing
    // changes what fills the view without also changing the angle on it.
    let distance = (radius / (fovy * 0.5).tan() * 1.25).clamp(0.02, 100_000.0);
    fly.position = bounds.center() - fly.forward() * distance;
    for (_, _, mut transform) in cameras.iter() {
        apply_fly_transform(&fly, &mut transform);
    }
}

/// World-space bounds of every loaded mesh in `subtree`, or of the whole world
/// when `subtree` is `None`.
fn scene_bounds(
    nodes: &Query<(Entity, &MeshComponent, &GlobalTransform)>,
    meshes: &AssetStore<Mesh>,
    subtree: Option<&HashSet<Entity>>,
) -> Option<Aabb> {
    let mut result: Option<Aabb> = None;
    for (entity, mesh_component, transform) in nodes.iter() {
        if subtree.is_some_and(|set| !set.contains(&entity)) {
            continue;
        }
        let Some(bounds) = meshes
            .get(&mesh_component.handle)
            .and_then(Mesh::local_aabb)
            .map(|bounds| bounds.transformed(transform.matrix()))
        else {
            continue;
        };
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

/// The wheel means "how fast do I fly" while looking around, and "move me
/// forward" otherwise — Unreal's split.
fn zoom(
    mut events: EventReader<WindowEvent>,
    hovered: Res<HoveredNode>,
    regions: Query<&ViewportRegion>,
    cameras: Query<(&EditorCamera, &mut Transform)>,
    mut fly: ResMut<FlyCamera>,
) {
    if (**hovered).is_none_or(|entity| regions.get_entity(entity).is_none()) {
        return;
    }
    let mut moved = false;
    for event in events.read() {
        if let winit::event::WindowEvent::MouseWheel { delta, .. } = &**event {
            let amount = match delta {
                winit::event::MouseScrollDelta::LineDelta(_, value) => f64::from(*value),
                winit::event::MouseScrollDelta::PixelDelta(value) => value.y / 32.0,
            } as f32;
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
        for (_, mut transform) in cameras.iter() {
            apply_fly_transform(&fly, &mut transform);
        }
    }
}

fn sync_viewport_size(
    layouts: Query<(&ViewportRegion, &UILayout)>,
    cameras: Query<(&EditorCamera, &mut Camera)>,
    viewport: Res<EditorViewport>,
    mut textures: ResMut<AssetStore<Texture>>,
    window: Res<window::plugin::Window>,
) {
    let Some((_, layout)) = layouts.iter().next() else {
        return;
    };
    let scale = window.scale_factor() as f32;
    let width = (layout.rect.size.x * scale).round().max(1.0) as u32;
    let height = (layout.rect.size.y * scale).round().max(1.0) as u32;
    if let Some(texture) = textures.get_mut(&viewport.texture) {
        texture.width = width;
        texture.height = height;
    }
    for (_, mut camera) in cameras.iter() {
        camera.aspect = width as f32 / height as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_navigation_requests_release_capture_and_only_reset_pose_on_replacement() {
        let mut fly = FlyCamera {
            position: Vec3::splat(12.0),
            looking: true,
            panning: true,
            ..Default::default()
        };
        let mut commands = ViewportCommands {
            release_navigation: true,
            ..Default::default()
        };
        assert!(apply_workspace_navigation(&mut commands, &mut fly));
        assert!(!fly.looking && !fly.panning);
        assert_eq!(fly.position, Vec3::splat(12.0));
        assert!(!commands.release_navigation);
        commands.reset = true;
        commands.frame_all = true;
        assert!(!apply_workspace_navigation(&mut commands, &mut fly));
        assert_eq!(fly.position, FlyCamera::default().position);
        assert!(!commands.reset && !commands.frame_all);
    }

    #[test]
    fn the_camera_looks_where_its_angles_point() {
        let fly = FlyCamera {
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            ..Default::default()
        };
        let mut transform = Transform::IDENTITY;
        apply_fly_transform(&fly, &mut transform);
        assert_eq!(transform.translation, fly.position);
        // Yawing a quarter turn from -Z faces -X.
        assert!((fly.forward() - Vec3::NEG_X).length() < 0.001);
    }

    #[test]
    fn movement_is_relative_to_where_the_camera_faces() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut input = Input::new();
        let press = |input: &mut Input, code| {
            input.update_key_input(PhysicalKey::Code(code), winit::event::ElementState::Pressed)
        };
        press(&mut input, KeyCode::KeyW);
        assert!(
            (movement(&input, rotation) - Vec3::NEG_X).length() < 0.001,
            "forward follows the camera, not the world"
        );

        press(&mut input, KeyCode::KeyE);
        let diagonal = movement(&input, rotation);
        assert!(
            (diagonal.length() - 1.0).abs() < 0.001,
            "forward and up together must not fly faster than either alone"
        );
    }
}
