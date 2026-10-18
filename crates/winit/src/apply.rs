use concerto_app::App;
use concerto_ecs::events::event_channel::EventChannel;
use concerto_window::{
    events::WindowEvent,
    plugin::{CursorMode, Window, WindowCommand, WindowSettings},
};
use glam::Vec2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WindowAction {
    Title(String),
    Decorations(bool),
    MinSize(Option<Vec2>),
    Cursor(CursorMode),
}

pub(crate) fn settings_actions(
    applied: Option<&WindowSettings>,
    wanted: &WindowSettings,
) -> Vec<WindowAction> {
    let mut actions = Vec::new();
    if applied.map(|applied| &applied.title) != Some(&wanted.title) {
        actions.push(WindowAction::Title(wanted.title.clone()));
    }
    if applied.map(|applied| applied.decorations) != Some(wanted.decorations) {
        actions.push(WindowAction::Decorations(wanted.decorations));
    }
    if applied.map(|applied| applied.min_size) != Some(wanted.min_size) {
        actions.push(WindowAction::MinSize(wanted.min_size));
    }
    if applied.map(|applied| applied.cursor) != Some(wanted.cursor) {
        actions.push(WindowAction::Cursor(wanted.cursor));
    }
    actions
}

pub(crate) fn exits_on_close(window: Option<&Window>) -> bool {
    !window.is_some_and(|window| window.settings.intercept_close)
}

pub(crate) fn take_requests(app: &mut App) -> Option<(WindowSettings, Vec<WindowCommand>)> {
    let window = app.get_resource_mut::<Window>()?;
    Some((window.settings.clone(), window.drain_commands()))
}

pub(crate) fn size_correction(window: Option<&Window>, actual: (u32, u32)) -> Option<WindowEvent> {
    let window = window?;
    (window.size() != actual).then_some(WindowEvent::Resized {
        width: actual.0,
        height: actual.1,
    })
}

pub(crate) fn deliver(app: &mut App, pending: &mut Vec<WindowEvent>) {
    let Some(channel) = app.get_resource_mut::<EventChannel<WindowEvent>>() else {
        pending.clear();
        return;
    };
    for event in pending.drain(..) {
        channel.push_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_app::{
        main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
    };
    use concerto_ecs::{
        events::event_reader::EventReader,
        resource::{ResMut, Resource},
    };

    #[derive(Resource, Default)]
    struct Seen(Vec<WindowEvent>);

    fn observe(mut events: EventReader<WindowEvent>, mut seen: ResMut<Seen>) {
        seen.0.extend(events.read().cloned());
    }

    fn pending() -> Vec<WindowEvent> {
        vec![WindowEvent::Focused(false), WindowEvent::CloseRequested]
    }

    #[test]
    fn nothing_applied_yet_applies_every_setting() {
        let wanted = WindowSettings::default();
        assert_eq!(
            settings_actions(None, &wanted),
            vec![
                WindowAction::Title("Concerto".to_string()),
                WindowAction::Decorations(true),
                WindowAction::MinSize(None),
                WindowAction::Cursor(CursorMode::default()),
            ]
        );
    }

    #[test]
    fn unchanged_settings_need_no_action() {
        let settings = WindowSettings::default();
        assert!(settings_actions(Some(&settings), &settings).is_empty());
    }

    #[test]
    fn only_the_changed_settings_are_applied() {
        let applied = WindowSettings::default();
        let mut wanted = applied.clone();
        wanted.title = "Showcase".to_string();
        wanted.cursor = CursorMode {
            visible: false,
            grabbed: true,
        };
        assert_eq!(
            settings_actions(Some(&applied), &wanted),
            vec![
                WindowAction::Title("Showcase".to_string()),
                WindowAction::Cursor(CursorMode {
                    visible: false,
                    grabbed: true,
                }),
            ]
        );
    }

    #[test]
    fn the_close_button_exits_unless_intercepted() {
        assert!(exits_on_close(None));
        let mut window = Window::new((800, 600), 1.0);
        assert!(exits_on_close(Some(&window)));
        window.settings.intercept_close = true;
        assert!(!exits_on_close(Some(&window)));
    }

    #[test]
    fn an_app_without_a_window_has_no_requests() {
        let mut app = App::new();
        assert!(take_requests(&mut app).is_none());
    }

    #[test]
    fn requests_are_read_once() {
        let mut app = App::new();
        let mut window = Window::new((800, 600), 1.0);
        window.settings.title = "Game".to_string();
        window.send(WindowCommand::Focus);
        app.insert_resource(window);

        let (settings, commands) = take_requests(&mut app).unwrap();
        assert_eq!(settings.title, "Game");
        assert_eq!(commands, vec![WindowCommand::Focus]);

        let (settings, commands) = take_requests(&mut app).unwrap();
        assert_eq!(settings.title, "Game");
        assert!(commands.is_empty());
    }

    #[test]
    fn an_app_without_a_window_needs_no_size_correction() {
        assert_eq!(size_correction(None, (800, 600)), None);
    }

    #[test]
    fn a_window_of_the_actual_size_needs_no_correction() {
        let window = Window::new((800, 600), 1.0);
        assert_eq!(size_correction(Some(&window), (800, 600)), None);
    }

    #[test]
    fn a_window_of_another_size_is_told_the_actual_size() {
        let window = Window::new((0, 0), 1.0);
        assert_eq!(
            size_correction(Some(&window), (1024, 768)),
            Some(WindowEvent::Resized {
                width: 1024,
                height: 768,
            })
        );
    }

    #[test]
    fn an_app_without_an_event_channel_drops_pending_events() {
        let mut app = App::new();
        let mut pending = pending();

        deliver(&mut app, &mut pending);

        assert!(pending.is_empty());
    }

    #[test]
    fn pending_events_reach_the_event_channel_in_order() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_event::<WindowEvent>()
            .insert_resource(Seen::default())
            .add_system(Update, observe);
        app.finish_plugin_build();
        let mut pending = pending();

        deliver(&mut app, &mut pending);
        app.update();

        assert!(pending.is_empty());
        assert_eq!(app.get_resource::<Seen>().unwrap().0, self::pending());
    }
}
