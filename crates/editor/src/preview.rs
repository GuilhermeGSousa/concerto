//! Preview `App`s that asset editors host: building them, signalling them, and their orbit camera.
use concerto_app::{
    App, Plugin,
    edit_time::AtEditTime,
    schedule_groups::{First, Startup, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, With, Without, World,
    command::CommandQueue,
    component::SystemTicks,
    events::event_reader::EventReader,
    signal::Signal,
    system::{
        NonSendMarker,
        access::SystemAccess,
        input::{SystemInput, ValidationError},
        meta::SystemMetadata,
    },
    world::UnsafeWorldCell,
};
use concerto_foundation::transform::Transform;
use concerto_render::{
    ExclusiveCamera,
    components::{camera::Camera, camera_environment::CameraEnvironment, light::Light},
};
use concerto_window::{
    events::{ScrollDelta, WindowEvent},
    input::{Input, MouseButton},
    plugin::Window,
};
use concerto_world_grid::plugin::WorldGridPlugin;
use glam::{Quat, Vec2, Vec3};

use crate::{
    asset_editor::{AssetEditorRegistry, EditorDocument},
    hosted::{self, HostRequest, HostRole, HostedApp, PendingApp, input::ViewportShown},
    scene::SceneRecipe,
    tools::camera::{SKY, editor_grid},
};

/// On a document whose preview failed to build; it is not built again until the tab is reopened.
#[derive(Component, Clone, Debug)]
pub struct PreviewFailed(pub String);

type QueuedSignal = Box<dyn FnOnce(&mut HostedApp) + Send + Sync>;

/// Signals sent through [`PreviewSignals`], each waiting for its document's preview.
#[derive(Resource, Default)]
pub struct PreviewSignalQueue(Vec<(Entity, QueuedSignal)>);

/// Sends signals to the preview `App` of a document; a signal waits while the preview builds.
pub struct PreviewSignals<'w> {
    queue: ResMut<'w, PreviewSignalQueue>,
}

impl PreviewSignals<'_> {
    /// Queues `signal` for `document`'s preview; it is dropped if the document has no preview.
    pub fn trigger<S: Signal>(&mut self, document: Entity, signal: S) {
        self.queue
            .0
            .push((document, Box::new(move |hosted| hosted.trigger(signal))));
    }
}

impl SystemInput for PreviewSignals<'_> {
    type State = ();
    type Data<'world, 'state> = PreviewSignals<'world>;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: UnsafeWorldCell<'world>,
        ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares exactly what the inner `ResMut` declares.
        let queue = unsafe {
            <ResMut<'static, PreviewSignalQueue> as SystemInput>::get_data(state, world, ticks)
        }?;
        Ok(PreviewSignals { queue })
    }

    fn fill_access(meta: &mut SystemMetadata, access: &mut SystemAccess) {
        <ResMut<'static, PreviewSignalQueue> as SystemInput>::fill_access(meta, access);
    }
}

pub(crate) fn deliver_preview_signals(
    _: NonSendMarker,
    mut queue: ResMut<PreviewSignalQueue>,
    mut hosted: Query<&mut HostedApp>,
    pending: Query<&PendingApp>,
    unbuilt: Query<&EditorDocument, Without<PreviewFailed>>,
    editors: Res<AssetEditorRegistry>,
) {
    if queue.0.is_empty() {
        return;
    }
    let mut waiting = Vec::new();
    for (document, signal) in std::mem::take(&mut queue.0) {
        if let Some(mut app) = hosted.get_entity_mut(document) {
            if app.role() == HostRole::Preview {
                signal(&mut app);
            }
        } else if pending.get_entity(document).is_some()
            || unbuilt
                .get_entity(document)
                .is_some_and(|doc| editors.preview(doc.asset_type).is_some())
        {
            waiting.push((document, signal));
        }
    }
    queue.0 = waiting;
}

pub(crate) fn open_previews(
    _: NonSendMarker,
    documents: Query<
        (Entity, &EditorDocument),
        (
            Without<HostedApp>,
            Without<PendingApp>,
            Without<PreviewFailed>,
        ),
    >,
    editors: Res<AssetEditorRegistry>,
    recipe: Res<SceneRecipe>,
    shown: Res<ViewportShown>,
    window: Res<Window>,
    mut cmd: CommandQueue,
) {
    for (document, doc) in documents.iter() {
        let Some(spec) = editors.preview(doc.asset_type) else {
            continue;
        };
        let request = HostRequest {
            gpu: recipe.gpu.as_ref(),
            recipe: recipe.recipe,
            size: shown.0.map_or((0, 0), |view| view.size),
            scale_factor: window.scale_factor(),
            role: HostRole::Preview,
            clipboard: recipe.clipboard,
        };
        let pending = hosted::begin(
            request,
            |app| {
                app.register_plugin(PreviewPlugin);
                (spec.build)(app);
            },
            |_| {},
        );
        cmd.insert(pending, document);
    }
}

pub(crate) fn report_preview_failures(
    mut building: Query<(Entity, &mut EditorDocument, &PendingApp), Without<HostedApp>>,
    editors: Res<AssetEditorRegistry>,
    mut cmd: CommandQueue,
) {
    for (document, mut doc, pending) in building.iter_mut() {
        if editors.preview(doc.asset_type).is_none() {
            continue;
        }
        let Some(failure) = pending.failure() else {
            continue;
        };
        let status = format!("Preview failed: {failure}");
        if doc.status != status {
            doc.status = status.clone();
        }
        cmd.remove::<PendingApp>(document);
        cmd.insert(PreviewFailed(status), document);
    }
}

pub(crate) fn keep_preview_failures_shown(
    mut failed: Query<(&mut EditorDocument, &PreviewFailed)>,
) {
    for (mut doc, failure) in failed.iter_mut() {
        if doc.status.is_empty() {
            doc.status = failure.0.clone();
        }
    }
}

/// Builds asset editors' previews and delivers their signals; `EditorPlugin` registers it.
pub struct PreviewHostPlugin;

impl Plugin for PreviewHostPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PreviewSignalQueue::default())
            .add_system(
                Update,
                (
                    open_previews,
                    report_preview_failures,
                    keep_preview_failures_shown,
                ),
            )
            .add_system(
                First,
                deliver_preview_signals
                    .after(hosted::finish_pending)
                    .before(hosted::input::step_viewport_target),
            );
    }
}

/// A camera turning around `target` at `distance`, by `yaw` around world up and then `pitch`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 4.0,
            yaw: 0.6,
            pitch: -0.35,
        }
    }
}

const ORBIT_PER_PIXEL: f32 = 0.005;
const PAN_PER_PIXEL: f32 = 0.0015;
const ZOOM_STEP: f32 = 1.15;
const MIN_DISTANCE: f32 = 0.25;
const MAX_DISTANCE: f32 = 100.0;
const MAX_PITCH: f32 = 1.55;

impl OrbitCamera {
    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    fn translation(&self) -> Vec3 {
        self.target + self.rotation() * Vec3::Z * self.distance
    }

    fn transform(&self) -> Transform {
        Transform::from_translation_rotation(self.translation(), self.rotation())
    }

    fn orbited(mut self, delta: Vec2) -> Self {
        self.yaw -= delta.x * ORBIT_PER_PIXEL;
        self.pitch = (self.pitch - delta.y * ORBIT_PER_PIXEL).clamp(-MAX_PITCH, MAX_PITCH);
        self
    }

    fn panned(mut self, delta: Vec2) -> Self {
        let rotation = self.rotation();
        let scale = self.distance * PAN_PER_PIXEL;
        self.target +=
            (rotation * Vec3::X) * -delta.x * scale + (rotation * Vec3::Y) * delta.y * scale;
        self
    }

    fn zoomed(mut self, notches: f32) -> Self {
        self.distance =
            (self.distance * ZOOM_STEP.powf(-notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self
    }
}

/// What every preview `App` gets: an orbit camera, the grid when there is one, and a light.
pub struct PreviewPlugin;

impl Plugin for PreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Startup, spawn_stage.at_edit_time())
            .add_system(Update, orbit_camera.at_edit_time());
        if app.is_plugin_added::<WorldGridPlugin>() {
            app.add_system(Startup, spawn_grid.at_edit_time());
        }
    }
}

fn spawn_stage(mut cmd: CommandQueue) {
    let orbit = OrbitCamera::default();
    let camera = cmd
        .spawn((
            Camera {
                clear_color: SKY,
                ..Default::default()
            },
            CameraEnvironment::default(),
            orbit.transform(),
            orbit,
        ))
        .entity();
    cmd.insert_resource(ExclusiveCamera(camera));
    cmd.spawn((
        Light::directional_light(),
        Transform::from_translation_rotation(
            Vec3::ZERO,
            Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.9),
        ),
    ));
}

fn spawn_grid(mut cmd: CommandQueue) {
    cmd.spawn(editor_grid());
}

fn wheel_notches(delta: &ScrollDelta) -> f32 {
    match delta {
        ScrollDelta::Lines(lines) => lines.y,
        ScrollDelta::Pixels(pixels) => pixels.y / 32.0,
    }
}

fn orbit_camera(
    input: Res<Input>,
    mut events: EventReader<WindowEvent>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform), With<Camera>>,
) {
    let notches: f32 = events
        .read()
        .filter_map(|event| match event {
            WindowEvent::MouseWheel { delta } => Some(wheel_notches(delta)),
            _ => None,
        })
        .sum();
    let delta = input.mouse_delta();
    let orbiting = input.is_mouse_button_held(MouseButton::Left);
    let panning = input.is_mouse_button_held(MouseButton::Middle);
    for (mut orbit, mut transform) in cameras.iter_mut() {
        let mut next = **orbit;
        if orbiting {
            next = next.orbited(delta);
        }
        if panning {
            next = next.panned(delta);
        }
        if notches != 0.0 {
            next = next.zoomed(notches);
        }
        if **orbit != next {
            **orbit = next;
        }
        let wanted = next.transform();
        if transform.translation != wanted.translation || transform.rotation != wanted.rotation {
            transform.translation = wanted.translation;
            transform.rotation = wanted.rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::{EditTarget, ViewportTarget, testing::*};
    use concerto_ecs::{IntoSystem, System, events::event_channel::EventChannel};
    use concerto_foundation::assets::Asset;

    fn targeted(f: &SceneFixture, document: Entity) -> Targets {
        Targets {
            viewport: f
                .world
                .get_component_for_entity::<ViewportTarget>(document)
                .is_some(),
            edit: f
                .world
                .get_component_for_entity::<EditTarget>(document)
                .is_some(),
        }
    }

    #[derive(Debug, PartialEq)]
    struct Targets {
        viewport: bool,
        edit: bool,
    }

    const PREVIEWED: Targets = Targets {
        viewport: true,
        edit: false,
    };
    const EDITED: Targets = Targets {
        viewport: true,
        edit: true,
    };
    const BEHIND: Targets = Targets {
        viewport: false,
        edit: false,
    };

    fn send(world: &mut World, document: Entity, value: u32) {
        let mut sent = Some(value);
        let mut system = (move |mut previews: PreviewSignals| {
            if let Some(value) = sent.take() {
                previews.trigger(document, Say(value));
            }
        })
        .into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    #[test]
    fn a_document_whose_editor_has_a_preview_hosts_a_preview_app_and_one_without_hosts_none() {
        let mut f = SceneFixture::new();
        let plain = f.request_kind(PlainAsset::name());
        f.frame();
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(plain)
                .is_none()
        );

        let preview = f.request_kind(PreviewedAsset::name());
        f.frame();
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(preview)
                .is_some()
        );
        f.frames_until("the preview was never built", |f| {
            f.world
                .get_component_for_entity::<HostedApp>(preview)
                .is_some()
        });

        assert_eq!(
            f.world
                .get_component_for_entity::<HostedApp>(preview)
                .unwrap()
                .role(),
            HostRole::Preview
        );
        assert!(
            f.world
                .get_component_for_entity::<HostedApp>(plain)
                .is_none()
        );
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(plain)
                .is_none()
        );
    }

    #[test]
    fn a_failed_preview_build_becomes_the_status_and_is_not_retried() {
        let mut f = SceneFixture::new();
        let document = f.request_kind(BrokenPreviewAsset::name());

        f.frames_until("the failure was never reported", |f| {
            f.world
                .get_component_for_entity::<PreviewFailed>(document)
                .is_some()
        });
        for _ in 0..3 {
            f.frame();
        }

        assert_eq!(f.status(document), "Preview failed: the preview broke");
        assert!(
            f.world
                .get_component_for_entity::<PendingApp>(document)
                .is_none()
        );
        assert!(
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_none()
        );
    }

    #[test]
    fn a_preview_tab_takes_only_the_viewport_target_and_a_scene_tab_takes_both_back() {
        let mut f = SceneFixture::new();
        let scene = f.open();
        let preview = f.open_preview();
        assert_eq!(targeted(&f, preview), PREVIEWED);
        assert_eq!(targeted(&f, scene), BEHIND);

        f.activate(Some(scene));
        assert_eq!(targeted(&f, scene), EDITED);
        assert_eq!(targeted(&f, preview), BEHIND);
        f.rename(scene, "first", "renamed");
        assert!(f.scene_names(scene).contains(&"renamed".to_owned()));

        f.activate(Some(preview));
        assert_eq!(targeted(&f, preview), PREVIEWED);
        assert_eq!(targeted(&f, scene), BEHIND);
    }

    #[test]
    fn the_preview_stage_is_an_exclusive_orbit_camera_and_a_light_with_no_grid_without_a_renderer()
    {
        let mut app = App::for_previewing();
        plain_recipe(&mut app);
        app.register_plugin(PreviewPlugin);
        app.finish_plugin_build();
        app.update();
        let world = app.main_mut().world_mut();

        let camera = world.get_resource::<ExclusiveCamera>().unwrap().0;
        assert_eq!(
            world.get_component_for_entity::<OrbitCamera>(camera),
            Some(&OrbitCamera::default())
        );
        assert!(
            world
                .get_component_for_entity::<CameraEnvironment>(camera)
                .is_some()
        );
        assert_eq!(world.query::<&Light, ()>().iter(world).count(), 1);
        assert_eq!(
            world
                .query::<&concerto_world_grid::WorldGrid, ()>()
                .iter(world)
                .count(),
            0
        );
    }

    struct Orbit {
        world: World,
        camera: Entity,
        system: concerto_ecs::system::BoxedSystem,
    }

    impl Orbit {
        fn new() -> Self {
            let mut world = World::new();
            world.insert_resource(Input::new());
            world.insert_resource(EventChannel::<WindowEvent>::new());
            let orbit = OrbitCamera::default();
            let camera = world.spawn((Camera::default(), orbit.transform(), orbit));
            let mut system = orbit_camera.into_boxed_system();
            system.initialize(&mut world);
            let mut orbit = Self {
                world,
                camera,
                system,
            };
            orbit.run();
            orbit
        }

        fn input(&mut self) -> &mut Input {
            self.world.get_resource_mut::<Input>().unwrap()
        }

        fn run(&mut self) {
            self.world.tick();
            self.system.run_and_apply((), &mut self.world);
            self.input().update();
            let channel = self
                .world
                .get_resource_mut::<EventChannel<WindowEvent>>()
                .unwrap();
            channel.update();
        }

        fn drag(&mut self, button: MouseButton, delta: Vec2) -> OrbitCamera {
            self.input().update_mouse_button(button, true);
            self.input()
                .update_mouse_delta((delta.x as f64, delta.y as f64));
            self.run();
            self.input().update_mouse_button(button, false);
            self.run();
            self.orbit()
        }

        fn wheel(&mut self, delta: ScrollDelta) -> OrbitCamera {
            self.world
                .get_resource_mut::<EventChannel<WindowEvent>>()
                .unwrap()
                .push_event(WindowEvent::MouseWheel { delta });
            self.run();
            self.orbit()
        }

        fn orbit(&self) -> OrbitCamera {
            *self
                .world
                .get_component_for_entity::<OrbitCamera>(self.camera)
                .unwrap()
        }
    }

    #[test]
    fn a_left_drag_orbits_a_wheel_zooms_and_a_middle_drag_pans() {
        let mut orbit = Orbit::new();
        let start = orbit.orbit();

        let orbited = orbit.drag(MouseButton::Left, Vec2::new(10.0, 4.0));
        assert!(orbited.yaw < start.yaw && orbited.pitch < start.pitch);
        assert_eq!(
            (orbited.target, orbited.distance),
            (start.target, start.distance)
        );

        let zoomed = orbit.wheel(ScrollDelta::Lines(Vec2::Y));
        assert!(zoomed.distance < orbited.distance);
        assert_eq!((zoomed.yaw, zoomed.target), (orbited.yaw, orbited.target));

        let panned = orbit.drag(MouseButton::Middle, Vec2::new(10.0, 4.0));
        assert_ne!(panned.target, zoomed.target);
        assert_eq!(
            (panned.yaw, panned.pitch, panned.distance),
            (zoomed.yaw, zoomed.pitch, zoomed.distance)
        );
        let transform = orbit
            .world
            .get_component_for_entity::<Transform>(orbit.camera)
            .unwrap();
        assert_eq!(transform.translation, panned.translation());
    }

    fn editor_app() -> (App, Entity) {
        let mut app = App::new();
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .register_plugin(crate::hosted::HostedPlugin)
            .register_plugin(PreviewHostPlugin)
            .insert_resource(Window::new((640, 480), 1.0))
            .insert_resource(crate::asset_editor::ActiveEditor::default())
            .insert_resource(crate::project::ProjectState::default())
            .insert_resource(concerto_foundation::assets::registry::AssetRegistry::default())
            .insert_resource(SceneRecipe {
                recipe: plain_recipe,
                gpu: None,
                clipboard: None,
                scene_plugins: Vec::new(),
            })
            .insert_resource(ViewportShown(Some(crate::hosted::input::ViewportView {
                origin: Vec2::ZERO,
                size: (64, 64),
                over: false,
                typing: false,
            })));
        let mut editors = AssetEditorRegistry::default();
        editors
            .register::<PreviewedAsset, PreviewedEditor>()
            .unwrap();
        app.insert_resource(editors);
        app.finish_plugin_build();
        let world = app.main_mut().world_mut();
        let document = world.spawn(EditorDocument {
            asset_type: PreviewedAsset::name(),
            title: "preview".into(),
            current: None,
            pending: None,
            order: 1,
            status: String::new(),
            failed: None,
        });
        world
            .get_resource_mut::<crate::asset_editor::ActiveEditor>()
            .unwrap()
            .0 = Some(document);
        (app, document)
    }

    fn preview_world(app: &App, document: Entity) -> Option<&World> {
        app.main()
            .world()
            .get_component_for_entity::<HostedApp>(document)
            .map(HostedApp::world_for_test)
    }

    fn update_until_built(app: &mut App, document: Entity) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while preview_world(app, document).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the preview was never built"
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn frames_in(app: &App, document: Entity) -> usize {
        preview_world(app, document)
            .unwrap()
            .get_resource::<Frames>()
            .unwrap()
            .0
    }

    fn said_in(app: &App, document: Entity) -> Vec<Heard> {
        preview_world(app, document)
            .unwrap()
            .get_resource::<Said>()
            .unwrap()
            .0
            .clone()
    }

    #[test]
    fn a_signal_reaches_the_preview_before_its_first_step_and_before_each_next_one() {
        let (mut app, document) = editor_app();
        send(app.main_mut().world_mut(), document, 5);

        update_until_built(&mut app, document);
        assert_eq!(
            said_in(&app, document),
            [Heard {
                value: 5,
                at_frame: 0
            }]
        );
        assert_eq!(frames_in(&app, document), 1);

        app.update();
        let frames = frames_in(&app, document);
        send(app.main_mut().world_mut(), document, 7);
        app.update();
        assert_eq!(
            said_in(&app, document),
            [
                Heard {
                    value: 5,
                    at_frame: 0
                },
                Heard {
                    value: 7,
                    at_frame: frames
                }
            ]
        );
        assert_eq!(frames_in(&app, document), frames + 1);
    }
}
