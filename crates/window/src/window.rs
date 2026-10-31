use concerto_ecs::resource::Resource;
use glam::Vec2;

use crate::{events::WindowEvent, input::Input};

/// How the pointer behaves over the drawing area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorMode {
    pub visible: bool,
    pub grabbed: bool,
}

impl Default for CursorMode {
    fn default() -> Self {
        Self {
            visible: true,
            grabbed: false,
        }
    }
}

/// What a game wants of its window; a host applies what makes sense for it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSettings {
    pub title: String,
    pub decorations: bool,
    /// The smallest size a person can resize down to, in logical pixels.
    pub min_size: Option<Vec2>,
    pub cursor: CursorMode,
    /// When set, the window's own close button only emits `WindowEvent::CloseRequested`.
    pub intercept_close: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            title: "Concerto".to_string(),
            decorations: true,
            min_size: None,
            cursor: CursorMode::default(),
            intercept_close: false,
        }
    }
}

/// A one-shot request that the host carries out once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowCommand {
    Minimise,
    SetMaximised(bool),
    /// A new size in logical pixels; the result arrives as `WindowEvent::Resized`.
    RequestSize(Vec2),
    Focus,
    Close,
}

/// An `App`'s drawing area: facts its host reports, and what the game asks of it.
#[derive(Resource)]
pub struct Window {
    size: (u32, u32),
    scale_factor: f64,
    focused: bool,
    maximised: bool,
    pub settings: WindowSettings,
    commands: Vec<WindowCommand>,
}

impl Window {
    pub fn new(size: (u32, u32), scale_factor: f64) -> Self {
        Self {
            size,
            scale_factor,
            focused: true,
            maximised: false,
            settings: WindowSettings::default(),
            commands: Vec::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The drawable size in physical pixels.
    pub fn physical_size(&self) -> (u32, u32) {
        self.size
    }

    pub fn width(&self) -> u32 {
        self.size.0
    }

    pub fn height(&self) -> u32 {
        self.size.1
    }

    /// Number of physical pixels per logical pixel.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// The size available to UI layout, in logical pixels.
    pub fn logical_size(&self) -> Vec2 {
        let scale = self.scale_factor as f32;
        Vec2::new(self.size.0 as f32 / scale, self.size.1 as f32 / scale)
    }

    /// Converts a physical coordinate to the logical UI coordinate space.
    pub fn physical_to_logical(&self, position: Vec2) -> Vec2 {
        position / self.scale_factor as f32
    }

    /// Current pointer position in logical UI coordinates.
    pub fn logical_pointer_position(&self, input: &Input) -> Vec2 {
        self.physical_to_logical(input.mouse_position())
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_maximised(&self) -> bool {
        self.maximised
    }

    /// Queues a request for the host.
    pub fn send(&mut self, command: WindowCommand) {
        self.commands.push(command);
    }

    /// Takes the queued requests; a host calls this once per frame.
    pub fn drain_commands(&mut self) -> Vec<WindowCommand> {
        std::mem::take(&mut self.commands)
    }

    pub(crate) fn reports(event: &WindowEvent) -> bool {
        matches!(
            event,
            WindowEvent::Resized { .. }
                | WindowEvent::ScaleFactorChanged(_)
                | WindowEvent::Focused(_)
                | WindowEvent::Maximised(_)
        )
    }

    pub(crate) fn apply(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::Resized { width, height } => self.size = (*width, *height),
            WindowEvent::ScaleFactorChanged(scale_factor) => self.scale_factor = *scale_factor,
            WindowEvent::Focused(focused) => self.focused = *focused,
            WindowEvent::Maximised(maximised) => self.maximised = *maximised,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_measures_divide_by_the_scale_factor() {
        let window = Window::new((1600, 1200), 2.0);
        assert_eq!(window.logical_size(), Vec2::new(800.0, 600.0));
        assert_eq!(
            window.physical_to_logical(Vec2::new(100.0, 50.0)),
            Vec2::new(50.0, 25.0)
        );
        assert_eq!((window.width(), window.height()), (1600, 1200));
    }

    #[test]
    fn events_update_the_facts() {
        let mut window = Window::new((800, 600), 1.0);
        window.apply(&WindowEvent::Resized {
            width: 1024,
            height: 768,
        });
        window.apply(&WindowEvent::ScaleFactorChanged(1.5));
        window.apply(&WindowEvent::Focused(false));
        window.apply(&WindowEvent::Maximised(true));
        assert_eq!(window.size(), (1024, 768));
        assert_eq!(window.scale_factor(), 1.5);
        assert!(!window.is_focused());
        assert!(window.is_maximised());
    }

    #[test]
    fn an_input_event_reports_no_fact() {
        assert!(!Window::reports(&WindowEvent::CloseRequested));
        assert!(Window::reports(&WindowEvent::Focused(true)));
    }
}
