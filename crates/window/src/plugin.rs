use ecs::resource::{ResMut, Resource};
use glam::Vec2;
use std::sync::Arc;

use crate::{
    input::{
        actions::{resolve_actions, ActionFired, ActionMap},
        Input,
    },
    winit_events::WindowEvent,
    ApplicationWindowHandler,
};
use app::{
    plugins::{Plugin, PluginsState},
    runner::AppExit,
    schedule_groups::{LateUpdate, Main},
    App,
};

#[cfg(target_arch = "wasm32")]
use winit::platform::web::EventLoopExtWebSys;

use winit::{
    event_loop::{ControlFlow, EventLoop, EventLoopProxy},
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{ResizeDirection, Window as WinitWindow},
};

/// The application's window.
///
/// Reading it (size, scale factor, maximised) is fine from any system. Changing
/// it from a worker thread is not: on Windows several `window_handle` setters
/// wait for the event-loop thread, which is itself waiting for the schedule.
#[derive(Resource)]
pub struct Window {
    pub window_handle: Arc<WinitWindow>,
}

/// Clipboard adapter consumed by UI controls. The portable in-process backing
/// can be replaced by a platform runner without changing widget APIs.
#[derive(Resource, Default)]
pub struct WindowClipboard(String);

impl WindowClipboard {
    pub fn read(&self) -> &str {
        &self.0
    }

    pub fn write(&mut self, value: impl Into<String>) {
        self.0 = value.into();
    }
}

#[allow(dead_code)]
#[derive(Resource)]
pub struct WindowEventLoopProxy(EventLoopProxy<()>);

impl Window {
    pub fn new(window: WinitWindow) -> Self {
        Self {
            window_handle: Arc::new(window),
        }
    }

    pub fn request_redraw(&self) {
        self.window_handle.request_redraw();
    }

    pub fn size(&self) -> (u32, u32) {
        let size = self.window_handle.inner_size();
        (size.width, size.height)
    }

    /// The drawable size in physical pixels.
    pub fn physical_size(&self) -> (u32, u32) {
        self.size()
    }

    /// The size available to UI layout, in logical pixels.
    pub fn logical_size(&self) -> Vec2 {
        let (width, height) = self.size();
        let scale = self.scale_factor() as f32;
        Vec2::new(width as f32 / scale, height as f32 / scale)
    }

    /// Number of physical pixels per logical pixel.
    pub fn scale_factor(&self) -> f64 {
        self.window_handle.scale_factor()
    }

    /// Converts a physical window coordinate to the logical UI coordinate space.
    pub fn physical_to_logical(&self, position: Vec2) -> Vec2 {
        position / self.scale_factor() as f32
    }

    /// Current pointer position in logical UI coordinates.
    pub fn logical_pointer_position(&self, input: &Input) -> Vec2 {
        self.physical_to_logical(input.mouse_position())
    }

    pub fn width(&self) -> u32 {
        self.window_handle.inner_size().width
    }

    pub fn height(&self) -> u32 {
        self.window_handle.inner_size().height
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(
        &self,
    ) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError>
    {
        self.window_handle.display_handle()
    }
}

impl HasWindowHandle for Window {
    fn window_handle(
        &self,
    ) -> Result<winit::raw_window_handle::WindowHandle<'_>, winit::raw_window_handle::HandleError>
    {
        self.window_handle.window_handle()
    }
}

/// Asks the event loop to close the window, the way its close button would.
///
/// An undecorated window has no close button of its own, so the application
/// has to be able to ask. Checked once per event-loop iteration.
#[derive(Resource, Default)]
pub struct CloseRequest(pub bool);

/// What a left press would do to the window, were it to land here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowGesture {
    /// Move the window; a double press toggles maximised instead.
    Move,
    Resize {
        direction: ResizeDirection,
    },
}

/// One rectangle of the window's frame, in logical pixels.
///
/// `gesture: None` blocks rather than starts one: a control standing inside the
/// title band is a button first, and pressing it must not drag the window.
pub struct WindowGestureZone {
    pub min: Vec2,
    pub max: Vec2,
    pub gesture: Option<WindowGesture>,
}

/// Where the window's own frame lives, kept current by whoever draws it — an
/// undecorated application, over its title band and edges.
///
/// Zones rather than "what the pointer is over", because the event loop starts
/// the gesture while it handles the press: a press that lands a frame after the
/// pointer arrives would otherwise be judged against where the pointer used to
/// be, and pressing a grip the moment you reach it would do nothing.
///
/// Moving and resizing are handed to the window manager, which on Windows only
/// takes them while the button is still down, so they cannot wait for a system
/// to notice the press next frame.
#[derive(Resource, Default)]
pub struct WindowGestureRegion {
    /// Topmost first: the first zone containing the press decides.
    pub zones: Vec<WindowGestureZone>,
}

impl WindowGestureRegion {
    /// What a press at `pointer` (logical pixels) starts, if anything.
    pub fn gesture_at(&self, pointer: Vec2) -> Option<WindowGesture> {
        self.zones
            .iter()
            .find(|zone| pointer.cmpge(zone.min).all() && pointer.cmple(zone.max).all())
            .and_then(|zone| zone.gesture)
    }
}

pub struct WindowPlugin;

fn winit_runner(mut app: App, event_loop: EventLoop<()>) -> AppExit {
    profiling::register_thread!("main");

    if app.plugin_state() == PluginsState::Ready {
        app.finish_plugin_build();
    }

    let state = ApplicationWindowHandler::new(app);

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            event_loop.spawn_app(state);
        } else {
            let mut state = state;
            let _ = event_loop.run_app(&mut state);
        }
    }

    AppExit::Success
}

fn update_input(mut input: ResMut<Input>) {
    input.update();
}

#[allow(deprecated)]
#[allow(unused_mut)]
impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<WindowEvent>();

        let mut event_loop_builder = EventLoop::builder();

        let event_loop = event_loop_builder
            .build()
            .expect("Failed to build event loop");
        event_loop.set_control_flow(ControlFlow::Poll);

        let mut win_attr = WinitWindow::default_attributes().with_title("winit example");

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            win_attr = win_attr.with_append(true);
        }

        let window = event_loop
            .create_window(win_attr)
            .expect("create window err.");

        window.set_cursor_visible(true);
        app.insert_resource(Input::new());
        app.insert_resource(ActionMap::default());
        app.register_event::<ActionFired>();
        // Runs before any consumer's LateUpdate systems, because WindowPlugin
        // is registered before them.
        app.add_system(LateUpdate, resolve_actions);
        app.insert_resource(WindowClipboard::default());
        app.insert_resource(CloseRequest::default());
        app.insert_resource(WindowGestureRegion::default());
        app.insert_resource(Window::new(window));
        app.insert_resource(WindowEventLoopProxy(event_loop.create_proxy()));

        // `MainSchedulePlugin` runs Update and LateUpdate from its `Main`
        // system. Advance transient input states afterwards so every input
        // consumer gets one frame in which to observe Pressed/Released.
        app.add_system(Main, update_input);
        app.set_runner(|app| winit_runner(app, event_loop));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::{
        main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::LateUpdate,
    };
    use ecs::{Res, ResMut, Resource};
    use winit::event::{ElementState, MouseButton};

    #[derive(Resource, Default)]
    struct SawPress(bool);

    fn observe_press(input: Res<Input>, mut observed: ResMut<SawPress>) {
        observed.0 = input.is_mouse_button_just_pressed(MouseButton::Left);
    }

    #[test]
    fn transient_input_is_visible_through_late_update() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .insert_resource(Input::new())
            .insert_resource(SawPress::default())
            .add_system(LateUpdate, observe_press)
            .add_system(Main, update_input);
        app.get_resource_mut::<Input>()
            .unwrap()
            .update_mouse_button(MouseButton::Left, ElementState::Pressed);
        app.finish_plugin_build();

        app.update();

        assert!(app.get_resource::<SawPress>().unwrap().0);
        assert!(matches!(
            app.get_resource::<Input>()
                .unwrap()
                .get_mouse_button_state(MouseButton::Left),
            crate::input::InputState::Down
        ));
    }
}
