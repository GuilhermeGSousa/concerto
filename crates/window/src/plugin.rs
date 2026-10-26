use concerto_ecs::resource::{ResMut, Resource};
use glam::Vec2;

use crate::{
    clipboard::{answer_paste_requests, ClipboardPasted, WindowClipboard},
    events::{fold_input, fold_window, FoldInput, InputGate, WindowEvent},
    input::{
        actions::{resolve_actions, ActionFired, ActionMap},
        Input,
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
    Resize {
        direction: ResizeDirection,
    },
}

/// One rectangle of the window's frame, in logical pixels.
pub struct WindowGestureZone {
    pub min: Vec2,
    pub max: Vec2,
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

pub struct WindowPlugin;

fn update_input(mut input: ResMut<Input>) {
    input.update();
}

impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_event::<WindowEvent>();
        if app.get_resource::<Window>().is_none() {
            app.insert_resource(Window::new((0, 0), 1.0));
        }
        app.insert_resource(Input::new());
        app.insert_resource(InputGate::default());
        app.insert_resource(ActionMap::default());
        app.register_event::<ActionFired>();
        app.add_system(First, fold_input.at_edit_time().in_set(FoldInput));
        app.add_system(First, fold_window.at_edit_time());
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

    #[test]
    fn a_point_inside_a_zone_returns_its_gesture() {
        let region = WindowGestureRegion {
            zones: vec![zone(
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 10.0),
                Some(east()),
            )],
        };
        assert_eq!(region.gesture_at(Vec2::new(5.0, 5.0)), Some(east()));
        assert_eq!(region.gesture_at(Vec2::new(10.0, 0.0)), Some(east()));
    }

    #[test]
    fn a_point_outside_every_zone_returns_none() {
        let region = WindowGestureRegion {
            zones: vec![zone(
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 10.0),
                Some(WindowGesture::Move),
            )],
        };
        assert_eq!(region.gesture_at(Vec2::new(11.0, 5.0)), None);
        assert_eq!(region.gesture_at(Vec2::new(5.0, -1.0)), None);
    }

    #[test]
    fn the_first_of_two_overlapping_zones_decides() {
        let region = WindowGestureRegion {
            zones: vec![
                zone(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), Some(east())),
                zone(
                    Vec2::new(0.0, 0.0),
                    Vec2::new(10.0, 10.0),
                    Some(WindowGesture::Move),
                ),
            ],
        };
        assert_eq!(region.gesture_at(Vec2::new(5.0, 5.0)), Some(east()));
    }

    #[test]
    fn a_zone_without_a_gesture_blocks_the_zone_under_it() {
        let region = WindowGestureRegion {
            zones: vec![
                zone(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), None),
                zone(
                    Vec2::new(0.0, 0.0),
                    Vec2::new(20.0, 20.0),
                    Some(WindowGesture::Move),
                ),
            ],
        };
        assert_eq!(region.gesture_at(Vec2::new(5.0, 5.0)), None);
        assert_eq!(
            region.gesture_at(Vec2::new(15.0, 15.0)),
            Some(WindowGesture::Move)
        );
    }
}
