use std::{cell::RefCell, rc::Rc, sync::Arc};

#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use concerto_app::{plugins::PluginsState, App};
#[cfg(not(target_arch = "wasm32"))]
use concerto_window::plugin::{WindowGesture, WindowGestureRegion};
use concerto_window::{
    events::WindowEvent,
    plugin::{Window, WindowCommand, WindowSettings},
};
use glam::Vec2;
#[cfg(not(target_arch = "wasm32"))]
use winit::event::{ElementState, MouseButton};
use winit::{
    application::ApplicationHandler, event::WindowEvent as WinitWindowEvent,
    window::Window as WinitWindow,
};

use crate::{
    apply::{self, WindowAction},
    translate::{self, Waiting},
};

/// How close together two presses on a move region count as a double press.
#[cfg(not(target_arch = "wasm32"))]
const DOUBLE_PRESS: Duration = Duration::from_millis(400);

pub(crate) struct Handler {
    app: Rc<RefCell<Option<App>>>,
    handle: Arc<WinitWindow>,
    pending: Vec<WindowEvent>,
    cursor: Vec2,
    applied: Option<WindowSettings>,
    maximised: bool,
    /// The last press that started a move, for spotting a double press.
    #[cfg(not(target_arch = "wasm32"))]
    last_move_press: Option<Instant>,
}

impl Handler {
    pub(crate) fn new(app: Rc<RefCell<Option<App>>>, handle: Arc<WinitWindow>) -> Self {
        Self {
            app,
            handle,
            pending: Vec::new(),
            cursor: Vec2::ZERO,
            applied: None,
            maximised: false,
            #[cfg(not(target_arch = "wasm32"))]
            last_move_press: None,
        }
    }

    fn apply_requests(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let slot = Rc::clone(&self.app);
        let mut slot = slot.borrow_mut();
        let Some(app) = slot.as_mut() else {
            return;
        };
        let Some((settings, commands)) = apply::take_requests(app) else {
            return;
        };

        for action in apply::settings_actions(self.applied.as_ref(), &settings) {
            match action {
                WindowAction::Title(title) => self.handle.set_title(&title),
                WindowAction::Decorations(decorations) => self.handle.set_decorations(decorations),
                WindowAction::MinSize(size) => self.handle.set_min_inner_size(
                    size.map(|size| winit::dpi::LogicalSize::new(size.x as f64, size.y as f64)),
                ),
                WindowAction::Cursor(cursor) => {
                    self.handle.set_cursor_visible(cursor.visible);
                    if cursor.grabbed {
                        if self
                            .handle
                            .set_cursor_grab(winit::window::CursorGrabMode::Confined)
                            .is_err()
                        {
                            let _ = self
                                .handle
                                .set_cursor_grab(winit::window::CursorGrabMode::Locked);
                        }
                    } else {
                        let _ = self
                            .handle
                            .set_cursor_grab(winit::window::CursorGrabMode::None);
                    }
                }
            }
        }
        self.applied = Some(settings);

        for command in commands {
            match command {
                WindowCommand::Minimise => self.handle.set_minimized(true),
                WindowCommand::SetMaximised(maximised) => self.handle.set_maximized(maximised),
                WindowCommand::RequestSize(size) => {
                    let requested = winit::dpi::LogicalSize::new(size.x as f64, size.y as f64);
                    if let Some(applied) = self.handle.request_inner_size(requested) {
                        self.pending.push(WindowEvent::Resized {
                            width: applied.width,
                            height: applied.height,
                        });
                    }
                }
                WindowCommand::Focus => self.handle.focus_window(),
                WindowCommand::Close => event_loop.exit(),
            }
        }
    }

    /// Starts whatever [`WindowGestureRegion`] puts under this press.
    ///
    /// The position is the pointer's as of this press: winit delivers the move
    /// that lands on the grip before the press itself, so pressing a grip the
    /// instant you reach it still counts.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_window_gesture(&mut self) {
        let slot = Rc::clone(&self.app);
        let slot = slot.borrow();
        let Some(app) = slot.as_ref() else {
            return;
        };
        let Some(window) = app.get_resource::<Window>() else {
            return;
        };
        let handle = Arc::clone(&self.handle);
        let pointer = window.physical_to_logical(self.cursor);
        let Some(gesture) = app
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
#[cfg(not(target_arch = "wasm32"))]
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

#[cfg(not(target_arch = "wasm32"))]
#[cfg(not(windows))]
fn left_button_held() -> bool {
    true
}

impl ApplicationHandler for Handler {
    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        self.app.borrow_mut().take();
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

        let slot = Rc::clone(&self.app);
        let mut slot = slot.borrow_mut();
        let Some(app) = slot.as_mut() else {
            match translate::while_waiting(&event) {
                Some(Waiting::Redraw) => self.handle.request_redraw(),
                Some(Waiting::Exit) => event_loop.exit(),
                None => (),
            }
            return;
        };

        if matches!(event, WinitWindowEvent::Resized(_)) {
            let maximised = self.handle.is_maximized();
            if maximised != self.maximised {
                self.maximised = maximised;
                self.pending.push(WindowEvent::Maximised(maximised));
            }
        }

        if let Some(translated) = translate::translate(&event) {
            if let WindowEvent::CursorMoved { position } = &translated {
                self.cursor = *position;
            }
            self.pending.push(translated);
        }

        match event {
            WinitWindowEvent::CloseRequested => {
                if apply::exits_on_close(app.get_resource::<Window>()) {
                    event_loop.exit();
                }
            }
            WinitWindowEvent::RedrawRequested => {
                if app.plugin_state() == PluginsState::Ready {
                    app.finish_plugin_build();
                }
                let actual = self.handle.inner_size();
                if let Some(resized) = apply::size_correction(
                    app.get_resource::<Window>(),
                    (actual.width, actual.height),
                ) {
                    self.pending.push(resized);
                }
                apply::deliver(app, &mut self.pending);

                if app.plugin_state() == PluginsState::Finished {
                    app.update();
                    profiling::finish_frame!();
                }

                drop(slot);
                self.apply_requests(event_loop);
                self.handle.request_redraw();
            }
            #[cfg(not(target_arch = "wasm32"))]
            WinitWindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                drop(slot);
                self.start_window_gesture();
            }
            _ => (),
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
        if self.app.borrow().is_none() {
            return;
        }

        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            self.pending.push(WindowEvent::MouseMotion {
                delta: Vec2::new(delta.0 as f32, delta.1 as f32),
            });
        }
    }
}
