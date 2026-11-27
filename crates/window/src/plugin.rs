use concerto_ecs::{
    resource::{ResMut, Resource},
    IntoSystemConfig,
};
use glam::Vec2;

use crate::{
    clipboard::{answer_paste_requests, ClipboardPasted, WindowClipboard},
    events::{fold_input, fold_window, FoldInput, GamepadConnection, InputGate, WindowEvent},
    input::{
        actions::{resolve_actions, ActionFired, ActionMap},
        update_players, Input, PlayerEvent, Players,
    },
};
use concerto_app::{
    edit_time::AtEditTime,
    plugins::Plugin,
    schedule_groups::{First, LateUpdate, Main},
    App,
};

pub use crate::window::{CursorMode, Window, WindowCommand, WindowSettings};
pub use winit::window::ResizeDirection;

/// What a left press would do to the window, were it to land here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowGesture {
    /// Move the window; a double press toggles maximised instead.
    Move,
    /// Resize the window by dragging an edge or corner.
    Resize {
        /// The edge or corner being dragged.
        direction: ResizeDirection,
    },
}

/// One rectangle of the window's frame, in logical pixels.
pub struct WindowGestureZone {
    /// The top-left corner.
    pub min: Vec2,
    /// The bottom-right corner.
    pub max: Vec2,
    /// What a press here does; `None` means nothing, hiding any zone below.
    pub gesture: Option<WindowGesture>,
}

/// Where the window's own frame lives, kept current by whoever draws it.
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

/// Adds the [`Window`], [`Input`], gamepad players, action map, clipboard and the events and
/// systems that keep them current from the host's [`WindowEvent`]s.
pub struct WindowPlugin;

fn update_input(mut input: ResMut<Input>) {
    input.update();
}

impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<WindowEvent>();
        app.register_event::<GamepadConnection>();
        if app.get_resource::<Window>().is_none() {
            app.insert_resource(Window::new((0, 0), 1.0));
        }
        app.insert_resource(Input::new());
        app.insert_resource(InputGate::default());
        app.insert_resource(ActionMap::default());
        app.register_event::<ActionFired>();
        app.add_system(First, fold_input.at_edit_time().in_set(FoldInput));
        app.add_system(First, fold_window.at_edit_time());
        app.insert_resource(Players::default());
        app.register_event::<PlayerEvent>();
        app.add_system(First, update_players.after(FoldInput));
        app.add_system(LateUpdate, resolve_actions);
        app.insert_resource(WindowClipboard::default());
        app.register_event::<ClipboardPasted>();
        app.add_system(First, answer_paste_requests);
        app.insert_resource(WindowGestureRegion::default());
        app.add_system(Main, update_input.at_edit_time());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(min: Vec2, max: Vec2, gesture: Option<WindowGesture>) -> WindowGestureZone {
        WindowGestureZone { min, max, gesture }
    }

    fn east() -> WindowGesture {
        WindowGesture::Resize {
            direction: ResizeDirection::East,
        }
    }

    struct Hit {
        name: &'static str,
        zones: Vec<WindowGestureZone>,
        point: Vec2,
        gesture: Option<WindowGesture>,
    }

    #[test]
    fn a_point_takes_the_gesture_of_the_first_zone_holding_it() {
        let square = |gesture| zone(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), gesture);
        let cases = [
            Hit {
                name: "inside",
                zones: vec![square(Some(east()))],
                point: Vec2::new(5.0, 5.0),
                gesture: Some(east()),
            },
            Hit {
                name: "past the right",
                zones: vec![square(Some(WindowGesture::Move))],
                point: Vec2::new(11.0, 5.0),
                gesture: None,
            },
            Hit {
                name: "the first of two overlapping zones",
                zones: vec![square(Some(east())), square(Some(WindowGesture::Move))],
                point: Vec2::new(5.0, 5.0),
                gesture: Some(east()),
            },
            Hit {
                name: "a zone without a gesture blocks the one under it",
                zones: vec![
                    square(None),
                    zone(Vec2::ZERO, Vec2::new(20.0, 20.0), Some(WindowGesture::Move)),
                ],
                point: Vec2::new(5.0, 5.0),
                gesture: None,
            },
            Hit {
                name: "past a blocking zone",
                zones: vec![
                    square(None),
                    zone(Vec2::ZERO, Vec2::new(20.0, 20.0), Some(WindowGesture::Move)),
                ],
                point: Vec2::new(15.0, 15.0),
                gesture: Some(WindowGesture::Move),
            },
        ];
        for case in cases {
            let region = WindowGestureRegion { zones: case.zones };
            assert_eq!(region.gesture_at(case.point), case.gesture, "{}", case.name);
        }
    }
}
