use std::collections::HashSet;

use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource,
    command::CommandQueue,
    events::event_reader::EventReader,
    signal::{On, Signal},
    system::NonSendMarker,
    world_id::WorldId,
};
use concerto_window::{
    InputGate,
    events::WindowEvent,
    input::{
        Input, KeyCode, MouseButton, PhysicalKey,
        actions::{Modifiers, Shortcut},
    },
    plugin::{CursorMode, Window},
};
use glam::Vec2;

use super::{HostedApp, ParkedEditApp, StopPlay};
use crate::asset_editor::ActiveEditor;

/// Where a hosted `App` is told the pointer is once it has left the viewport.
pub const POINTER_GONE: Vec2 = Vec2::splat(-1.0);

/// Where the viewport is this frame, in the editor window's physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportView {
    pub origin: Vec2,
    pub size: (u32, u32),
    pub over: bool,
    pub typing: bool,
}

/// The source of this frame's [`ViewportView`]; the viewport panel writes it.
#[derive(Resource, Default)]
pub struct ViewportShown(pub Option<ViewportView>);

/// The buttons whose presses the host passed on, and the `App` it stepped last.
#[derive(Resource, Default)]
pub struct Routing {
    buttons: HashSet<MouseButton>,
    stepped: Option<(Entity, WorldId)>,
    pointer_on_picture: Option<bool>,
    keys: HashSet<KeyCode>,
    playing: bool,
}

/// Who has the editor window's input while a document's game plays.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOwner {
    Game,
    Editor,
}

/// Gives the editor window's input back to the editor from the game playing on a document.
pub struct ReleaseInput(pub Entity);
impl Signal for ReleaseInput {}

const STOP: Shortcut = Shortcut {
    key: KeyCode::KeyP,
    modifiers: Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    },
};

const RELEASE: Shortcut = Shortcut {
    key: KeyCode::F1,
    modifiers: Modifiers {
        shift: true,
        ..Modifiers::NONE
    },
};

fn modifiers(keys: &HashSet<KeyCode>) -> Modifiers {
    let held = |left, right| keys.contains(&left) || keys.contains(&right);
    Modifiers {
        ctrl: held(KeyCode::ControlLeft, KeyCode::ControlRight),
        shift: held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        alt: held(KeyCode::AltLeft, KeyCode::AltRight),
        logo: held(KeyCode::SuperLeft, KeyCode::SuperRight),
    }
}

fn shortcut(event: &WindowEvent, keys: &mut HashSet<KeyCode>) -> Option<Shortcut> {
    match event {
        WindowEvent::Key {
            key: PhysicalKey::Code(key),
            pressed,
            ..
        } => {
            let modifiers = modifiers(keys);
            let fresh = if *pressed {
                keys.insert(*key)
            } else {
                keys.remove(key);
                false
            };
            fresh.then_some(Shortcut {
                key: *key,
                modifiers,
            })
        }
        WindowEvent::Focused(false) => {
            keys.clear();
            None
        }
        _ => None,
    }
}

fn contains(view: &ViewportView, position: Vec2) -> bool {
    let size = Vec2::new(view.size.0 as f32, view.size.1 as f32);
    position.cmpge(view.origin).all() && position.cmplt(view.origin + size).all()
}

fn on_picture(view: &ViewportView, pointer: Vec2) -> bool {
    view.over && contains(view, pointer)
}

fn release(owner: &mut InputOwner, app: &mut HostedApp) {
    *owner = InputOwner::Editor;
    app.push(WindowEvent::Focused(false));
}

pub(crate) fn release_input(
    on: On<ReleaseInput>,
    owners: Query<&mut InputOwner>,
    hosted: Query<&mut HostedApp>,
) {
    let document = on.signal().0;
    if let Some(mut owner) = owners.get_entity(document)
        && **owner == InputOwner::Game
        && let Some(mut app) = hosted.get_entity(document)
    {
        release(&mut owner, &mut app);
    }
}

pub(crate) fn route_play_input(
    _: NonSendMarker,
    active: Res<ActiveEditor>,
    shown: Res<ViewportShown>,
    mut events: EventReader<WindowEvent>,
    hosted: Query<&mut HostedApp>,
    owners: Query<&mut InputOwner>,
    input: Res<Input>,
    mut routing: ResMut<Routing>,
    mut gate: ResMut<InputGate>,
    mut cmd: CommandQueue,
) {
    let target = active.0.zip(shown.0).and_then(|(document, view)| {
        let owner = owners.get_entity(document)?;
        let app = hosted.get_entity(document)?;
        Some((document, view, owner, app))
    });
    routing.playing = target.is_some();
    let Some((document, view, mut owner, mut app)) = target else {
        for event in events.read() {
            shortcut(event, &mut routing.keys);
        }
        if !gate.open {
            gate.open = true;
        }
        return;
    };

    let mut pointer = input.mouse_position();
    let mut current = **owner;
    let mut stopping = false;
    for event in events.read() {
        let pressed = shortcut(event, &mut routing.keys);
        if let WindowEvent::CursorMoved { position } = event {
            pointer = *position;
        }
        if stopping {
            continue;
        }
        match current {
            InputOwner::Game => match event {
                _ if pressed == Some(STOP) => {
                    stopping = true;
                    cmd.trigger(StopPlay(document));
                }
                _ if pressed == Some(RELEASE) => release(&mut current, &mut app),
                WindowEvent::MouseButton { pressed: true, .. } if !on_picture(&view, pointer) => {
                    release(&mut current, &mut app);
                }
                WindowEvent::Focused(false) => release(&mut current, &mut app),
                WindowEvent::CursorMoved { position } => app.push(WindowEvent::CursorMoved {
                    position: *position - view.origin,
                }),
                WindowEvent::Key { .. }
                | WindowEvent::MouseButton { .. }
                | WindowEvent::MouseMotion { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Focused(true) => app.push(event.clone()),
                WindowEvent::Resized { .. }
                | WindowEvent::ScaleFactorChanged(_)
                | WindowEvent::Maximised(_)
                | WindowEvent::CloseRequested => {}
            },
            InputOwner::Editor => {
                if matches!(event, WindowEvent::MouseButton { pressed: true, .. })
                    && on_picture(&view, pointer)
                {
                    current = InputOwner::Game;
                    app.push(WindowEvent::Focused(true));
                    app.push(WindowEvent::CursorMoved {
                        position: pointer - view.origin,
                    });
                }
            }
        }
    }
    if **owner != current {
        **owner = current;
    }
    let open = current == InputOwner::Editor;
    if gate.open != open {
        gate.open = open;
    }
}

/// Whether the editor window's cursor capture was copied from a hosted `App`.
#[derive(Resource, Default)]
pub struct HostCursor {
    copied: bool,
}

/// The event the hosted `App` should receive for an editor window event, if any.
pub fn route(
    event: &WindowEvent,
    view: &ViewportView,
    routing: &mut Routing,
) -> Option<WindowEvent> {
    let dragging = !routing.buttons.is_empty();
    match event {
        WindowEvent::CursorMoved { position } => {
            (view.over || dragging).then(|| WindowEvent::CursorMoved {
                position: *position - view.origin,
            })
        }
        WindowEvent::MouseMotion { .. } => (view.over || dragging).then(|| event.clone()),
        WindowEvent::MouseWheel { .. } => view.over.then(|| event.clone()),
        WindowEvent::MouseButton {
            button,
            pressed: true,
        } => {
            if !view.over {
                return None;
            }
            routing.buttons.insert(*button);
            Some(event.clone())
        }
        WindowEvent::MouseButton {
            button,
            pressed: false,
        } => {
            routing.buttons.remove(button);
            Some(event.clone())
        }
        WindowEvent::Key { pressed: true, .. } => {
            (dragging || (view.over && !view.typing)).then(|| event.clone())
        }
        WindowEvent::Key { pressed: false, .. } => Some(event.clone()),
        WindowEvent::Focused(focused) => {
            if !focused {
                routing.buttons.clear();
            }
            Some(event.clone())
        }
        WindowEvent::Resized { .. }
        | WindowEvent::ScaleFactorChanged(_)
        | WindowEvent::Maximised(_)
        | WindowEvent::CloseRequested => None,
    }
}

/// A `Resized` when the hosted window and the viewport disagree.
pub fn size_correction(view: &ViewportView, hosted: (u32, u32)) -> Option<WindowEvent> {
    let (width, height) = view.size;
    (width > 0 && height > 0 && view.size != hosted)
        .then_some(WindowEvent::Resized { width, height })
}

/// A `ScaleFactorChanged` when the hosted window's scale is not the editor window's.
pub fn scale_correction(editor: f64, hosted: f64) -> Option<WindowEvent> {
    (editor != hosted).then_some(WindowEvent::ScaleFactorChanged(editor))
}

pub(crate) fn step_edit_target(
    _: NonSendMarker,
    active: Res<ActiveEditor>,
    shown: Res<ViewportShown>,
    mut events: EventReader<WindowEvent>,
    hosted: Query<&mut HostedApp>,
    parked: Query<&ParkedEditApp>,
    owners: Query<&InputOwner>,
    mut routing: ResMut<Routing>,
    mut cursor: ResMut<HostCursor>,
    mut window: ResMut<Window>,
    input: Res<Input>,
    mut cmd: CommandQueue,
) {
    let entering = active.0.zip(shown.0).and_then(|(document, _)| {
        hosted
            .get_entity(document)
            .map(|app| (document, app.world().id()))
    });

    let entered = routing.stepped != entering;
    if entered {
        if let Some((document, id)) = routing.stepped
            && let Some(mut left) = hosted.get_entity(document)
            && left.world().id() == id
        {
            left.push(WindowEvent::Focused(false));
            left.step();
            let closing = left.drop_window_commands();
            if closing && parked.get_entity(document).is_some() {
                cmd.trigger(StopPlay(document));
            }
        }
        routing.buttons.clear();
        routing.pointer_on_picture = None;
        routing.stepped = entering;
    }

    let target = active
        .0
        .zip(shown.0)
        .and_then(|(document, view)| hosted.get_entity(document).map(|app| (document, view, app)));
    let Some((document, view, mut app)) = target else {
        events.clear();
        if cursor.copied {
            cursor.copied = false;
            window.settings.cursor = CursorMode::default();
        }
        return;
    };

    if entered {
        app.push(WindowEvent::Focused(true));
    }
    if let Some(rescale) = app.window().and_then(|hosted_window| {
        scale_correction(window.scale_factor(), hosted_window.scale_factor())
    }) {
        app.push(rescale);
    }
    if let Some(resize) = app
        .window()
        .and_then(|hosted_window| size_correction(&view, hosted_window.size()))
    {
        app.push(resize);
    }
    let owner = owners.get_entity(document).copied();
    if routing.playing || owner.is_some() {
        events.clear();
    } else {
        route_editing(&mut events, &view, &mut routing, &input, &mut app);
    }
    app.step();
    let closing = app.drop_window_commands();
    if closing && parked.get_entity(document).is_some() {
        cmd.trigger(StopPlay(document));
    }

    let grab = match owner {
        Some(InputOwner::Editor) => Some(CursorMode::default()),
        _ => app
            .window()
            .map(|hosted_window| hosted_window.settings.cursor),
    };
    if let Some(wanted) = grab {
        let default = CursorMode::default();
        if wanted != default || cursor.copied {
            if window.settings.cursor != wanted {
                window.settings.cursor = wanted;
            }
            cursor.copied = wanted != default;
        }
    }
}

fn route_editing(
    events: &mut EventReader<WindowEvent>,
    view: &ViewportView,
    routing: &mut Routing,
    input: &Input,
    app: &mut HostedApp,
) {
    if view.over && routing.pointer_on_picture != Some(true) {
        routing.pointer_on_picture = Some(true);
        app.push(WindowEvent::CursorMoved {
            position: input.mouse_position() - view.origin,
        });
    }
    for event in events.read() {
        let dragging = !routing.buttons.is_empty();
        if let Some(routed) = route(event, view, routing) {
            if !dragging && !routing.buttons.is_empty() && !view.typing {
                let unseen: Vec<_> = input
                    .held_keys()
                    .filter(|key| {
                        !app.world()
                            .get_resource::<Input>()
                            .is_some_and(|hosted_input| hosted_input.is_held(*key))
                    })
                    .collect();
                for key in unseen {
                    app.push(WindowEvent::Key {
                        key,
                        pressed: true,
                        text: None,
                    });
                }
            }
            app.push(routed);
        }
    }
    if !view.over && routing.pointer_on_picture != Some(false) && routing.buttons.is_empty() {
        routing.pointer_on_picture = Some(false);
        app.push(WindowEvent::CursorMoved {
            position: POINTER_GONE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_window::input::{KeyCode, MouseButton, PhysicalKey};
    use glam::Vec2;

    const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyW);

    fn view(over: bool) -> ViewportView {
        ViewportView {
            origin: Vec2::new(100.0, 50.0),
            size: (640, 480),
            over,
            typing: false,
        }
    }

    fn press(button: MouseButton) -> WindowEvent {
        WindowEvent::MouseButton {
            button,
            pressed: true,
        }
    }

    fn release(button: MouseButton) -> WindowEvent {
        WindowEvent::MouseButton {
            button,
            pressed: false,
        }
    }

    fn key(pressed: bool) -> WindowEvent {
        WindowEvent::Key {
            key: KEY,
            pressed,
            text: None,
        }
    }

    #[test]
    fn a_pointer_position_is_made_relative_to_the_viewport() {
        let mut routing = Routing::default();
        let routed = route(
            &WindowEvent::CursorMoved {
                position: Vec2::new(130.0, 70.0),
            },
            &view(true),
            &mut routing,
        );

        assert!(matches!(
            routed,
            Some(WindowEvent::CursorMoved { position }) if position == Vec2::new(30.0, 20.0)
        ));
    }

    #[test]
    fn a_press_outside_the_viewport_is_not_forwarded() {
        let mut routing = Routing::default();

        assert!(route(&press(MouseButton::Left), &view(false), &mut routing).is_none());
    }

    #[test]
    fn a_release_is_forwarded_wherever_the_pointer_is() {
        let mut routing = Routing::default();
        assert!(route(&press(MouseButton::Right), &view(true), &mut routing).is_some());
        assert!(route(&key(true), &view(true), &mut routing).is_some());

        assert!(route(&release(MouseButton::Right), &view(false), &mut routing).is_some());
        assert!(route(&key(false), &view(false), &mut routing).is_some());
    }

    #[test]
    fn a_drag_that_leaves_the_viewport_keeps_receiving_moves_and_keys() {
        let mut routing = Routing::default();
        route(&press(MouseButton::Right), &view(true), &mut routing);

        assert!(
            route(
                &WindowEvent::CursorMoved {
                    position: Vec2::ZERO
                },
                &view(false),
                &mut routing
            )
            .is_some()
        );
        assert!(route(&key(true), &view(false), &mut routing).is_some());

        route(&release(MouseButton::Right), &view(false), &mut routing);
        assert!(
            route(
                &WindowEvent::CursorMoved {
                    position: Vec2::ZERO
                },
                &view(false),
                &mut routing
            )
            .is_none()
        );
    }

    #[test]
    fn a_key_press_is_not_forwarded_while_a_text_field_has_focus() {
        let mut routing = Routing::default();
        let typing = ViewportView {
            typing: true,
            ..view(true)
        };

        assert!(route(&key(true), &typing, &mut routing).is_none());
    }

    #[test]
    fn a_key_press_is_forwarded_to_a_drag_while_a_text_field_has_focus() {
        let mut routing = Routing::default();
        let typing = ViewportView {
            typing: true,
            ..view(true)
        };
        route(&press(MouseButton::Right), &typing, &mut routing);

        assert!(route(&key(true), &typing, &mut routing).is_some());
    }

    #[test]
    fn the_editor_windows_focus_is_forwarded_and_losing_it_ends_a_drag() {
        let mut routing = Routing::default();
        route(&press(MouseButton::Right), &view(true), &mut routing);

        assert_eq!(
            route(&WindowEvent::Focused(false), &view(false), &mut routing),
            Some(WindowEvent::Focused(false))
        );
        assert!(
            route(
                &WindowEvent::CursorMoved {
                    position: Vec2::ZERO
                },
                &view(false),
                &mut routing
            )
            .is_none()
        );
        assert_eq!(
            route(&WindowEvent::Focused(true), &view(false), &mut routing),
            Some(WindowEvent::Focused(true))
        );
    }

    #[test]
    fn window_level_events_are_not_routed() {
        let mut routing = Routing::default();
        for event in [
            WindowEvent::Resized {
                width: 10,
                height: 10,
            },
            WindowEvent::ScaleFactorChanged(2.0),
            WindowEvent::Maximised(true),
            WindowEvent::CloseRequested,
        ] {
            assert!(route(&event, &view(true), &mut routing).is_none());
        }
    }

    #[test]
    fn a_size_correction_is_sent_only_when_the_sizes_differ_and_the_viewport_has_area() {
        assert!(matches!(
            size_correction(&view(true), (320, 240)),
            Some(WindowEvent::Resized {
                width: 640,
                height: 480
            })
        ));
        assert!(size_correction(&view(true), (640, 480)).is_none());
        let flat = ViewportView {
            size: (0, 480),
            ..view(true)
        };
        assert!(size_correction(&flat, (320, 240)).is_none());
    }

    #[test]
    fn a_scale_correction_is_sent_only_when_the_scales_differ() {
        assert_eq!(
            scale_correction(2.0, 1.0),
            Some(WindowEvent::ScaleFactorChanged(2.0))
        );
        assert!(scale_correction(1.5, 1.5).is_none());
    }

    mod playing {
        use super::*;
        use crate::{
            actions::TogglePlay,
            hosted::{
                HostedPlugin, PlayState,
                testing::{hosted_now, plain_recipe, play_now, play_state},
            },
        };
        use concerto_app::{
            App, main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
        };
        use concerto_ecs::{
            events::{event_channel::EventChannel, event_cursor::EventCursor},
            signal::On,
        };
        use concerto_window::{
            InputGate,
            input::actions::{ActionFired, ActionMap},
            plugin::WindowPlugin,
        };

        const ORIGIN: Vec2 = Vec2::new(100.0, 50.0);
        const OUTSIDE: Vec2 = Vec2::new(900.0, 600.0);
        const INSIDE: Vec2 = Vec2::new(300.0, 200.0);

        #[derive(Resource, Default)]
        struct Stops(usize);

        fn count_stop(_: On<StopPlay>, mut stops: ResMut<Stops>) {
            stops.0 += 1;
        }

        fn grab(mut window: ResMut<Window>) {
            window.settings.cursor.grabbed = true;
        }

        fn grabbing_recipe(app: &mut App) {
            plain_recipe(app);
            app.add_system(Update, grab);
        }

        fn editor_playing(recipe: fn(&mut App)) -> (App, Entity) {
            editor_playing_with(recipe, |_| {})
        }

        fn editor_playing_with(
            recipe: fn(&mut App),
            setup: impl FnOnce(&mut App),
        ) -> (App, Entity) {
            let mut app = App::new();
            app.register_plugin(MainSchedulePlugin)
                .register_plugin(TimePlugin)
                .register_plugin(WindowPlugin)
                .insert_resource(ActiveEditor::default())
                .insert_resource(Stops::default())
                .register_plugin(HostedPlugin);
            let world = app.main_mut().world_mut();
            world.add_listener(count_stop);
            let document = world.spawn(hosted_now(plain_recipe, |_| {}));
            world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
            app.get_resource_mut::<ViewportShown>().unwrap().0 = Some(ViewportView {
                origin: ORIGIN,
                size: (640, 480),
                over: true,
                typing: false,
            });
            app.get_resource_mut::<ActionMap>()
                .unwrap()
                .bind_global(TogglePlay, Shortcut::ctrl(KeyCode::KeyP));
            setup(&mut app);
            app.finish_plugin_build();
            app.update();
            play_now(app.main_mut().world_mut(), document, recipe, |_| {});
            app.update();
            (app, document)
        }

        fn send(app: &mut App, event: WindowEvent) {
            app.get_resource_mut::<EventChannel<WindowEvent>>()
                .unwrap()
                .push_event(event);
        }

        fn press_key(app: &mut App, key: KeyCode) {
            send(
                app,
                WindowEvent::Key {
                    key: PhysicalKey::Code(key),
                    pressed: true,
                    text: None,
                },
            );
        }

        fn release_key(app: &mut App, key: KeyCode) {
            send(
                app,
                WindowEvent::Key {
                    key: PhysicalKey::Code(key),
                    pressed: false,
                    text: None,
                },
            );
        }

        fn point(app: &mut App, position: Vec2) {
            if let Some(view) = &mut app.get_resource_mut::<ViewportShown>().unwrap().0 {
                view.over = position.cmpge(ORIGIN).all()
                    && position.cmplt(ORIGIN + Vec2::new(640.0, 480.0)).all();
            }
            send(app, WindowEvent::CursorMoved { position });
        }

        fn release_with_shift_f1(app: &mut App) {
            press_key(app, KeyCode::ShiftLeft);
            press_key(app, KeyCode::F1);
            app.update();
            release_key(app, KeyCode::F1);
            release_key(app, KeyCode::ShiftLeft);
            app.update();
        }

        fn owner(app: &App, document: Entity) -> Option<InputOwner> {
            app.main()
                .world()
                .get_component_for_entity::<InputOwner>(document)
                .copied()
        }

        fn gate_open(app: &App) -> bool {
            app.get_resource::<InputGate>().unwrap().open
        }

        fn editor_input(app: &App) -> &Input {
            app.get_resource::<Input>().unwrap()
        }

        fn hosted(app: &App, document: Entity) -> &HostedApp {
            app.main()
                .world()
                .get_component_for_entity::<HostedApp>(document)
                .unwrap()
        }

        fn game_input(app: &App, document: Entity) -> &Input {
            hosted(app, document)
                .world()
                .get_resource::<Input>()
                .unwrap()
        }

        fn game_focused(app: &App, document: Entity) -> bool {
            hosted(app, document).window().unwrap().is_focused()
        }

        fn toggles_fired(app: &App) -> usize {
            app.get_resource::<EventChannel<ActionFired>>()
                .unwrap()
                .read_from(&mut EventCursor::default())
                .filter(|fired| fired.is(TogglePlay))
                .count()
        }

        fn stops(app: &App) -> usize {
            app.get_resource::<Stops>().unwrap().0
        }

        fn grabbed(app: &App) -> bool {
            app.get_resource::<Window>()
                .unwrap()
                .settings
                .cursor
                .grabbed
        }

        #[test]
        fn playing_gives_the_game_input_and_closes_the_editors_gate() {
            let (app, document) = editor_playing(plain_recipe);

            assert_eq!(owner(&app, document), Some(InputOwner::Game));
            assert!(!gate_open(&app));
        }

        #[test]
        fn while_the_game_has_input_a_key_reaches_it_wherever_the_pointer_is_and_the_editor_sees_none()
         {
            let (mut app, document) = editor_playing(plain_recipe);
            point(&mut app, OUTSIDE);
            press_key(&mut app, KeyCode::KeyW);
            app.update();

            assert!(game_input(&app, document).is_held(KEY));
            assert_eq!(
                game_input(&app, document).mouse_position(),
                OUTSIDE - ORIGIN
            );
            assert!(!editor_input(&app).is_held(KEY));
            assert_eq!(owner(&app, document), Some(InputOwner::Game));
        }

        #[test]
        fn a_press_outside_the_viewport_gives_the_editor_input_and_reaches_its_panel() {
            let (mut app, document) = editor_playing(plain_recipe);
            point(&mut app, INSIDE);
            press_key(&mut app, KeyCode::KeyW);
            app.update();
            assert!(game_input(&app, document).is_held(KEY));

            point(&mut app, OUTSIDE);
            send(&mut app, press(MouseButton::Left));
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(editor_input(&app).is_mouse_button_held(MouseButton::Left));
            assert!(!game_input(&app, document).is_held(KEY));
            assert!(!game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            assert!(!game_focused(&app, document));
        }

        fn point_under(app: &mut App, position: Vec2) {
            if let Some(view) = &mut app.get_resource_mut::<ViewportShown>().unwrap().0 {
                view.over = false;
            }
            send(app, WindowEvent::CursorMoved { position });
        }

        #[test]
        fn a_press_on_a_node_drawn_above_the_viewport_gives_the_editor_input_and_reaches_it() {
            let (mut app, document) = editor_playing(plain_recipe);
            point_under(&mut app, INSIDE);
            app.update();

            send(&mut app, press(MouseButton::Left));
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(editor_input(&app).is_mouse_button_held(MouseButton::Left));
            assert!(!game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            assert!(!game_focused(&app, document));

            send(&mut app, release(MouseButton::Left));
            app.update();

            assert!(!editor_input(&app).is_mouse_button_held(MouseButton::Left));
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
        }

        fn click_the_hovered_node(
            input: Res<Input>,
            hovered: Res<concerto_ui::interaction::HoveredNode>,
            mut cmd: CommandQueue,
        ) {
            if input.is_mouse_button_just_released(MouseButton::Left)
                && let Some(node) = **hovered
            {
                cmd.entity(node).trigger(concerto_ui::interaction::UIClick {
                    position: Vec2::ZERO,
                    button: MouseButton::Left,
                });
            }
        }

        #[test]
        fn a_click_on_the_stop_button_while_the_game_has_input_stops_play() {
            let (mut app, document) = editor_playing_with(plain_recipe, |app| {
                app.insert_resource(concerto_ui::interaction::HoveredNode::default());
                app.add_system(
                    concerto_app::schedule_groups::LateUpdate,
                    click_the_hovered_node,
                );
            });
            let world = app.main_mut().world_mut();
            let stop = world.spawn((
                crate::tabs::EditorTabPlay { document },
                concerto_ecs::signal::listener::IntoListener::into_listener(
                    crate::tabs::press_play,
                ),
            ));
            **world
                .get_resource_mut::<concerto_ui::interaction::HoveredNode>()
                .unwrap() = Some(stop);
            point_under(&mut app, INSIDE);
            app.update();

            send(&mut app, press(MouseButton::Left));
            app.update();
            send(&mut app, release(MouseButton::Left));
            app.update();

            assert_eq!(stops(&app), 1);
            assert_eq!(
                play_state(app.main().world(), document),
                Some(PlayState::Editing)
            );
            assert!(!game_input(&app, document).is_mouse_button_held(MouseButton::Left));
        }

        #[test]
        fn a_press_on_the_bare_viewport_keeps_input_with_the_game() {
            let (mut app, document) = editor_playing(plain_recipe);
            point(&mut app, INSIDE);
            app.update();

            send(&mut app, press(MouseButton::Left));
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Game));
            assert!(!gate_open(&app));
            assert!(game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            assert!(!editor_input(&app).is_mouse_button_held(MouseButton::Left));
        }

        #[test]
        fn shift_f1_gives_the_editor_input_without_a_press() {
            let (mut app, document) = editor_playing(plain_recipe);
            press_key(&mut app, KeyCode::KeyW);
            app.update();

            press_key(&mut app, KeyCode::ShiftLeft);
            press_key(&mut app, KeyCode::F1);
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(!game_input(&app, document).is_held(KEY));
            assert!(!game_focused(&app, document));
        }

        #[test]
        fn while_the_editor_has_input_the_game_gets_none() {
            let (mut app, document) = editor_playing(plain_recipe);
            release_with_shift_f1(&mut app);

            point(&mut app, INSIDE);
            press_key(&mut app, KeyCode::KeyW);
            app.update();

            assert!(!game_input(&app, document).is_held(KEY));
            assert!(editor_input(&app).is_held(KEY));
        }

        #[test]
        fn a_press_in_the_viewport_gives_the_game_input_back_without_pressing_in_it() {
            let (mut app, document) = editor_playing(plain_recipe);
            release_with_shift_f1(&mut app);

            point(&mut app, INSIDE);
            send(&mut app, press(MouseButton::Left));
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Game));
            assert!(!gate_open(&app));
            assert!(game_focused(&app, document));
            assert!(!game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            assert_eq!(game_input(&app, document).mouse_position(), INSIDE - ORIGIN);
            assert!(!editor_input(&app).is_mouse_button_held(MouseButton::Left));
        }

        #[test]
        fn losing_the_windows_focus_gives_the_editor_input_and_regaining_it_does_not_give_it_back()
        {
            let (mut app, document) = editor_playing(plain_recipe);

            send(&mut app, WindowEvent::Focused(false));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(!game_focused(&app, document));

            send(&mut app, WindowEvent::Focused(true));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
        }

        #[test]
        fn ctrl_p_while_the_game_has_input_stops_once_and_is_no_action() {
            let (mut app, document) = editor_playing(plain_recipe);

            press_key(&mut app, KeyCode::ControlLeft);
            press_key(&mut app, KeyCode::KeyP);
            app.update();
            app.update();

            assert_eq!(stops(&app), 1);
            assert_eq!(toggles_fired(&app), 0);
            assert_eq!(
                play_state(app.main().world(), document),
                Some(PlayState::Editing)
            );
            assert!(gate_open(&app));
        }

        #[test]
        fn ctrl_p_while_the_editor_has_input_is_the_action_and_the_host_does_not_stop() {
            let (mut app, _) = editor_playing(plain_recipe);
            release_with_shift_f1(&mut app);

            press_key(&mut app, KeyCode::ControlLeft);
            press_key(&mut app, KeyCode::KeyP);
            app.update();

            assert_eq!(stops(&app), 0);
            assert_eq!(toggles_fired(&app), 1);
        }

        #[test]
        fn a_repeated_ctrl_p_press_does_not_stop_the_game() {
            let (mut app, _) = editor_playing(plain_recipe);
            release_with_shift_f1(&mut app);
            press_key(&mut app, KeyCode::ControlLeft);
            press_key(&mut app, KeyCode::KeyP);
            app.update();
            point(&mut app, INSIDE);
            send(&mut app, press(MouseButton::Left));
            app.update();

            press_key(&mut app, KeyCode::KeyP);
            app.update();

            assert_eq!(stops(&app), 0);
        }

        #[test]
        fn a_key_held_in_the_game_at_stop_is_held_on_neither_side_afterwards() {
            let (mut app, document) = editor_playing(plain_recipe);
            point(&mut app, INSIDE);
            press_key(&mut app, KeyCode::KeyW);
            press_key(&mut app, KeyCode::ControlLeft);
            press_key(&mut app, KeyCode::KeyP);
            app.update();
            app.update();

            assert_eq!(
                play_state(app.main().world(), document),
                Some(PlayState::Editing)
            );
            assert_eq!(owner(&app, document), None);
            assert!(gate_open(&app));
            assert!(!editor_input(&app).is_held(KEY));
            assert!(!game_input(&app, document).is_held(KEY));
        }

        #[test]
        fn the_games_cursor_grab_is_copied_only_while_it_has_input() {
            let (mut app, _) = editor_playing(grabbing_recipe);
            app.update();
            assert!(grabbed(&app));

            press_key(&mut app, KeyCode::ShiftLeft);
            press_key(&mut app, KeyCode::F1);
            app.update();
            assert!(!grabbed(&app));
            release_key(&mut app, KeyCode::F1);
            release_key(&mut app, KeyCode::ShiftLeft);
            app.update();
            assert!(!grabbed(&app));

            point(&mut app, INSIDE);
            send(&mut app, press(MouseButton::Left));
            app.update();
            assert!(grabbed(&app));
        }

        #[test]
        fn the_release_signal_gives_the_editor_input() {
            let (mut app, document) = editor_playing(plain_recipe);

            app.main_mut().world_mut().trigger(ReleaseInput(document));
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(!game_focused(&app, document));
        }
    }
}
