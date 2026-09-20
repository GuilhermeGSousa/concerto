use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{plugins::PluginsState, App};
use ecs::events::event_channel::EventChannel;
use input::Input;
use plugin::{Window, WindowGesture, WindowGestureRegion};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton},
    keyboard::PhysicalKey,
};

use winit::event::WindowEvent as WinitWindowEvent;

use crate::winit_events::WindowEvent;

pub mod input;
pub mod plugin;
pub mod winit_events;

pub fn run() {}

/// How close together two presses on a move region count as a double press.
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

pub struct ApplicationWindowHandler {
    app: App,
    winit_events: Vec<winit::event::WindowEvent>,
    /// The last press that started a move, for spotting a double press.
    last_move_press: Option<Instant>,
}

impl ApplicationWindowHandler {
    pub fn new(app: App) -> Self {
        Self {
            app,
            winit_events: Vec::new(),
            last_move_press: None,
        }
    }

    /// Starts whatever [`WindowGestureRegion`] puts under this press.
    ///
    /// The position is the pointer's as of this press: winit delivers the move
    /// that lands on the grip before the press itself, so pressing a grip the
    /// instant you reach it still counts.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_window_gesture(&mut self) {
        let Some(window) = self.app.get_resource::<Window>() else {
            return;
        };
        let handle = Arc::clone(&window.window_handle);
        let Some(input) = self.app.get_resource::<Input>() else {
            return;
        };
        let pointer = window.logical_pointer_position(input);
        let Some(gesture) = self
            .app
            .get_resource::<WindowGestureRegion>()
            .and_then(|region| region.gesture_at(pointer))
        else {
            return;
        };
        match gesture {
            WindowGesture::Move => {
                let now = Instant::now();
                let doubled = self
                    .last_move_press
                    .is_some_and(|previous| now.duration_since(previous) < DOUBLE_PRESS);
                self.last_move_press = (!doubled).then_some(now);
                if doubled {
                    handle.set_maximized(!handle.is_maximized());
                } else if left_button_held() {
                    let _ = handle.drag_window();
                }
            }
            WindowGesture::Resize { direction } => {
                if left_button_held() {
                    let _ = handle.drag_resize_window(direction);
                }
            }
        }
    }
}

/// Whether the left button is physically down right now, not as of the press
/// being handled.
///
/// A frame long enough to outlast a click delivers the press and the release
/// together. Asking Windows to move the window after the release has already
/// happened would jam winit's drag state, so a press whose button is already up
/// is left as the click it was.
#[cfg(windows)]
fn left_button_held() -> bool {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
        WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON},
    };
    // GetAsyncKeyState reads the physical buttons; with them swapped, the
    // logical left one is the physical right.
    let key = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    };
    let state = unsafe { GetAsyncKeyState(i32::from(key)) };
    // The high bit is set while the key is down.
    state < 0
}

#[cfg(not(windows))]
fn left_button_held() -> bool {
    true
}

impl ApplicationHandler for ApplicationWindowHandler {
    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        drop(std::mem::take(&mut self.app));
    }

    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let _ = event_loop;
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let _ = window_id;

        self.winit_events.push(event.clone());

        match event {
            WinitWindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WinitWindowEvent::RedrawRequested => {
                let event_channel = self
                    .app
                    .get_resource_mut::<EventChannel<WindowEvent>>()
                    .unwrap();

                self.winit_events.drain(..).for_each(|e| {
                    event_channel.push_event(WindowEvent::new(e.clone()));
                });

                if self.app.plugin_state() == PluginsState::Finished {
                    self.app.update();
                }

                let window = self.app.get_resource::<Window>().unwrap();
                window.request_redraw();
            }
            WinitWindowEvent::KeyboardInput { event, .. } => {
                let input = self.app.get_resource_mut::<Input>().unwrap();
                // Capture typed text (handles modifier keys, dead keys, etc.).
                // Only on press — not release — and only printable characters.
                if event.state == winit::event::ElementState::Pressed {
                    if let Some(text) = &event.text {
                        for c in text.chars() {
                            if !c.is_control() {
                                input.push_typed_char(c);
                            }
                        }
                    }
                }
                if let PhysicalKey::Code(keycode) = event.physical_key {
                    input.update_key_input(PhysicalKey::Code(keycode), event.state);
                }
            }
            WinitWindowEvent::CursorMoved { position, .. } => {
                let input = self.app.get_resource_mut::<Input>().unwrap();
                input.update_mouse_position(position.x, position.y);
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                let input = self.app.get_resource_mut::<Input>().unwrap();
                input.update_mouse_button(button, state);
                #[cfg(not(target_arch = "wasm32"))]
                if state == ElementState::Pressed && button == MouseButton::Left {
                    self.start_window_gesture();
                }
            }
            _ => (),
        }
    }

    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        let _ = (event_loop, cause);

        if self.app.plugin_state() == PluginsState::Ready {
            self.app.finish_plugin_build();
        }
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: ()) {
        let _ = (event_loop, event);
    }

    fn device_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            let input_state = self.app.get_resource_mut::<Input>().unwrap();
            input_state.update_mouse_delta(delta);
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self
            .app
            .get_resource::<plugin::CloseRequest>()
            .is_some_and(|request| request.0)
        {
            event_loop.exit();
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_window() {
        super::run();
    }
}
