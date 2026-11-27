//! Routes the editor window's input to the hosted `App` the viewport shows, and gives it back to
//! the editor.
use std::collections::HashSet;

use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource, With,
    command::CommandQueue,
    events::event_reader::EventReader,
    signal::{
        On, Signal,
        lifecycle::{Add, Remove},
    },
    system::NonSendMarker,
};
use concerto_window::{
    InputGate,
    events::WindowEvent,
    input::{
        Gamepad, GamepadAxis, GamepadButton, GamepadId, Input, KeyCode, MouseButton, PhysicalKey,
        actions::{Modifiers, Shortcut},
    },
    plugin::{CursorMode, Window},
};
use glam::Vec2;

use super::{HostedApp, ParkedEditApp, StopPlay, target::ViewportTarget};

/// Where a hosted `App` is told the pointer is once it has left the viewport.
pub const POINTER_GONE: Vec2 = Vec2::splat(-1.0);

/// Where the viewport is this frame, in the editor window's physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportView {
    /// The top-left corner of the picture.
    pub origin: Vec2,
    /// The picture's size.
    pub size: (u32, u32),
    /// Whether the pointer is over the viewport.
    pub over: bool,
    /// Whether a text field of the editor has focus, so keys and clicks stay with the editor.
    pub typing: bool,
}

/// The source of this frame's [`ViewportView`]; the viewport panel writes it.
#[derive(Resource, Default)]
pub struct ViewportShown(pub Option<ViewportView>);

/// The buttons whose presses the host passed on.
#[derive(Resource, Default)]
pub struct Routing {
    buttons: HashSet<MouseButton>,
    pointer_on_picture: Option<bool>,
    keys: HashSet<KeyCode>,
    playing: bool,
    pad_target: Option<Entity>,
    told_pads: HashSet<GamepadId>,
    sent_pads: HashSet<GamepadId>,
}

impl Routing {
    /// The keys the editor window holds down, as the host has seen them pressed and not released.
    pub fn held_keys(&self) -> impl Iterator<Item = KeyCode> + '_ {
        self.keys.iter().copied()
    }

    /// The pads MCP `send_input` connected and has not disconnected, as opposed to real gamepads.
    pub fn sent_pads(&self) -> impl Iterator<Item = GamepadId> + '_ {
        self.sent_pads.iter().copied()
    }

    pub(crate) fn note_sent(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::GamepadConnected { id, .. } => {
                self.sent_pads.insert(*id);
            }
            WindowEvent::GamepadDisconnected { id } => {
                self.sent_pads.remove(id);
            }
            _ => {}
        }
    }
}

/// Marks the viewport-target document while the viewport shows its picture.
#[derive(Component)]
pub struct InViewport;

/// Keeps [`InViewport`] on the viewport target exactly while the viewport is shown.
pub(crate) fn sync_in_viewport(
    shown: Res<ViewportShown>,
    targets: Query<Entity, With<ViewportTarget>>,
    marked: Query<Entity, With<InViewport>>,
    mut cmd: CommandQueue,
) {
    let wanted = shown.0.and_then(|_| targets.iter().next());
    for document in marked.iter() {
        if Some(document) != wanted {
            cmd.remove::<InViewport>(document);
        }
    }
    if let Some(document) = wanted
        && marked.get_entity(document).is_none()
    {
        cmd.insert(InViewport, document);
    }
}

impl Routing {
    fn forget_routing(&mut self) {
        self.buttons.clear();
        self.pointer_on_picture = None;
    }
}

pub(crate) fn focus_shown_app(
    on: On<Add<InViewport>>,
    mut hosted: Query<&mut HostedApp>,
    input: Res<Input>,
    mut routing: ResMut<Routing>,
) {
    let document = on.signal().entity;
    if let Some(mut app) = hosted.get_entity_mut(document) {
        app.push(WindowEvent::Focused(true));
        if routing.pad_target == Some(document) {
            for pad in input
                .gamepads()
                .filter(|pad| routing.told_pads.contains(&pad.id()))
            {
                push_pad_state(pad, &mut app);
            }
        }
    }
    routing.forget_routing();
}

pub(crate) fn blur_hidden_app(
    on: On<Remove<InViewport>>,
    mut hosted: Query<&mut HostedApp>,
    parked: Query<&ParkedEditApp>,
    mut routing: ResMut<Routing>,
    mut cmd: CommandQueue,
) {
    let document = on.signal().entity;
    if let Some(mut app) = hosted.get_entity_mut(document) {
        app.push(WindowEvent::Focused(false));
        app.step();
        if app.drop_window_commands() && parked.get_entity(document).is_some() {
            cmd.trigger(StopPlay(document));
        }
    }
    routing.forget_routing();
}

pub(crate) fn focus_new_target(
    on: On<Add<ViewportTarget>>,
    shown: Query<&InViewport>,
    mut hosted: Query<&mut HostedApp>,
    mut routing: ResMut<Routing>,
) {
    let document = on.signal().entity;
    if shown.get_entity(document).is_some()
        && let Some(mut app) = hosted.get_entity_mut(document)
    {
        app.push(WindowEvent::Focused(true));
    }
    routing.forget_routing();
}

pub(crate) fn blur_old_target(
    on: On<Remove<ViewportTarget>>,
    shown: Query<&InViewport>,
    mut hosted: Query<&mut HostedApp>,
) {
    let document = on.signal().entity;
    if shown.get_entity(document).is_some()
        && let Some(mut app) = hosted.get_entity_mut(document)
    {
        app.push(WindowEvent::Focused(false));
    }
}

/// Who has the editor window's input while a document's game plays.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOwner {
    /// The game gets keys, the pointer and gamepads, and hides the cursor if it asks to.
    Game,
    /// The editor gets everything but gamepads, until a click on the picture gives it to the game.
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
        logo: held(KeyCode::MetaLeft, KeyCode::MetaRight),
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

impl ViewportView {
    fn on_picture(&self, pointer: Vec2) -> bool {
        self.over && contains(self, pointer)
    }
}

fn release(owner: &mut InputOwner, app: &mut HostedApp) {
    *owner = InputOwner::Editor;
    app.push(WindowEvent::Focused(false));
}

impl InputOwner {
    fn hand_over(&mut self, app: &mut HostedApp, held: Option<&Input>, pads_since: &[WindowEvent]) {
        release(self, app);
        for pad in held.into_iter().flat_map(Input::gamepads) {
            push_pad_state(pad, app);
        }
        for event in pads_since {
            app.push(event.clone());
        }
    }
}

pub(crate) fn release_input(
    on: On<ReleaseInput>,
    mut owners: Query<&mut InputOwner>,
    mut hosted: Query<&mut HostedApp>,
    input: Res<Input>,
) {
    let document = on.signal().0;
    if let Some(mut owner) = owners.get_entity_mut(document)
        && **owner == InputOwner::Game
        && let Some(mut app) = hosted.get_entity_mut(document)
    {
        owner.hand_over(&mut app, Some(&input), &[]);
    }
}

fn forward_pad(event: &WindowEvent, told: &mut HashSet<GamepadId>, app: &mut HostedApp) {
    match event {
        WindowEvent::GamepadConnected { id, .. } => {
            told.insert(*id);
        }
        WindowEvent::GamepadDisconnected { id } => {
            told.remove(id);
        }
        _ => {}
    }
    app.push(event.clone());
}

fn is_pad(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::GamepadConnected { .. }
            | WindowEvent::GamepadDisconnected { .. }
            | WindowEvent::GamepadButton { .. }
            | WindowEvent::GamepadAxis { .. }
    )
}

fn catch_up_pads(input: &Input, told: &mut HashSet<GamepadId>, app: &mut HostedApp) {
    let gone: Vec<GamepadId> = told
        .iter()
        .copied()
        .filter(|id| input.gamepad(*id).is_none())
        .collect();
    for id in gone {
        told.remove(&id);
        app.push(WindowEvent::GamepadDisconnected { id });
    }
    for pad in input.gamepads() {
        let id = pad.id();
        if !told.insert(id) {
            continue;
        }
        app.push(WindowEvent::GamepadConnected {
            id,
            name: pad.name().to_string(),
        });
        push_pad_state(pad, app);
    }
}

fn push_pad_state(pad: &Gamepad, app: &mut HostedApp) {
    let id = pad.id();
    for button in GamepadButton::ALL {
        let value = pad.button_value(button);
        if value != 0.0 {
            app.push(WindowEvent::GamepadButton { id, button, value });
        }
    }
    for axis in GamepadAxis::ALL {
        let value = pad.raw_axis(axis);
        if value != 0.0 {
            app.push(WindowEvent::GamepadAxis { id, axis, value });
        }
    }
}

pub(crate) fn route_play_input(
    _: NonSendMarker,
    targets: Query<Entity, (With<ViewportTarget>, With<InViewport>)>,
    viewport_targets: Query<Entity, With<ViewportTarget>>,
    shown: Res<ViewportShown>,
    mut events: EventReader<WindowEvent>,
    mut hosted: Query<&mut HostedApp>,
    mut owners: Query<&mut InputOwner>,
    input: Res<Input>,
    mut routing: ResMut<Routing>,
    mut gate: ResMut<InputGate>,
    mut cmd: CommandQueue,
) {
    let player = viewport_targets
        .iter()
        .next()
        .filter(|document| owners.contains_entity(*document));
    if routing.pad_target != player {
        routing.pad_target = player;
        routing.told_pads.clear();
    }
    let target = targets
        .iter()
        .next()
        .zip(shown.0)
        .and_then(|(document, view)| {
            let owner = owners.get_entity_mut(document)?;
            let app = hosted.get_entity_mut(document)?;
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

    catch_up_pads(&input, &mut routing.told_pads, &mut app);
    let mut pointer = input.mouse_position();
    let mut current = **owner;
    let mut stopping = false;
    let mut held = Some(&*input);
    let mut pads_since = Vec::new();
    for event in events.read() {
        let pressed = shortcut(event, &mut routing.keys);
        if let WindowEvent::CursorMoved { position } = event {
            pointer = *position;
        }
        if stopping {
            continue;
        }
        if matches!(event, WindowEvent::Focused(false)) {
            held = None;
            pads_since.clear();
        }
        if is_pad(event) {
            pads_since.push(event.clone());
        }
        match current {
            InputOwner::Game => match event {
                _ if pressed == Some(STOP) => {
                    stopping = true;
                    cmd.trigger(StopPlay(document));
                }
                _ if pressed == Some(RELEASE) => current.hand_over(&mut app, held, &pads_since),
                WindowEvent::MouseButton { pressed: true, .. } if !view.on_picture(pointer) => {
                    current.hand_over(&mut app, held, &pads_since);
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
                WindowEvent::GamepadConnected { .. }
                | WindowEvent::GamepadDisconnected { .. }
                | WindowEvent::GamepadButton { .. }
                | WindowEvent::GamepadAxis { .. } => {
                    forward_pad(event, &mut routing.told_pads, &mut app)
                }
                WindowEvent::Resized { .. }
                | WindowEvent::ScaleFactorChanged(_)
                | WindowEvent::Maximised(_)
                | WindowEvent::CloseRequested => {}
            },
            InputOwner::Editor => match event {
                WindowEvent::GamepadConnected { .. }
                | WindowEvent::GamepadDisconnected { .. }
                | WindowEvent::GamepadButton { .. }
                | WindowEvent::GamepadAxis { .. } => {
                    forward_pad(event, &mut routing.told_pads, &mut app)
                }
                WindowEvent::Focused(false) => app.push(event.clone()),
                WindowEvent::MouseButton { pressed: true, .. } if view.on_picture(pointer) => {
                    current = InputOwner::Game;
                    app.push(WindowEvent::Focused(true));
                    app.push(WindowEvent::CursorMoved {
                        position: pointer - view.origin,
                    });
                }
                _ => {}
            },
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
        | WindowEvent::CloseRequested
        | WindowEvent::GamepadConnected { .. }
        | WindowEvent::GamepadDisconnected { .. }
        | WindowEvent::GamepadButton { .. }
        | WindowEvent::GamepadAxis { .. } => None,
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

pub(crate) fn step_viewport_target(
    _: NonSendMarker,
    targets: Query<Entity, (With<ViewportTarget>, With<InViewport>)>,
    shown: Res<ViewportShown>,
    mut events: EventReader<WindowEvent>,
    mut hosted: Query<&mut HostedApp>,
    parked: Query<&ParkedEditApp>,
    owners: Query<&InputOwner>,
    mut routing: ResMut<Routing>,
    mut cursor: ResMut<HostCursor>,
    mut window: ResMut<Window>,
    input: Res<Input>,
    mut cmd: CommandQueue,
) {
    let target = targets
        .iter()
        .next()
        .zip(shown.0)
        .and_then(|(document, view)| {
            hosted
                .get_entity_mut(document)
                .map(|app| (document, view, app))
        });
    let Some((document, view, mut app)) = target else {
        events.clear();
        if cursor.copied {
            cursor.copied = false;
            window.settings.cursor = CursorMode::default();
        }
        return;
    };

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
    fn a_release_reaches_the_viewport_wherever_the_pointer_is_and_typing_sends_it_no_keys() {
        let mut routing = Routing::default();
        assert!(route(&press(MouseButton::Right), &view(true), &mut routing).is_some());
        assert!(route(&key(true), &view(true), &mut routing).is_some());

        assert!(route(&release(MouseButton::Right), &view(false), &mut routing).is_some());
        assert!(route(&key(false), &view(false), &mut routing).is_some());

        let typing = ViewportView {
            typing: true,
            ..view(true)
        };
        assert!(route(&key(true), &typing, &mut routing).is_none());
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
            input::{
                Gamepad,
                actions::{ActionFired, ActionMap},
            },
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
            let (mut app, document) = editor_editing(setup);
            play_now(app.main_mut().world_mut(), document, recipe, |_| {});
            app.update();
            (app, document)
        }

        fn editor_editing(setup: impl FnOnce(&mut App)) -> (App, Entity) {
            let mut app = App::new();
            app.register_plugin(MainSchedulePlugin)
                .register_plugin(TimePlugin)
                .register_plugin(WindowPlugin)
                .insert_resource(crate::asset_editor::ActiveEditor::default())
                .insert_resource(Stops::default())
                .register_plugin(HostedPlugin);
            let world = app.main_mut().world_mut();
            world.add_listener(count_stop);
            let document = world.spawn(hosted_now(plain_recipe, |_| {}));
            world
                .get_resource_mut::<crate::asset_editor::ActiveEditor>()
                .unwrap()
                .0 = Some(document);
            world.trigger(crate::hosted::SetEditTarget(Some(document)));
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
        fn the_game_keeps_input_until_the_editor_takes_it_and_gets_it_back_from_a_press_inside() {
            let (mut app, document) = editor_playing(plain_recipe);
            assert_eq!(owner(&app, document), Some(InputOwner::Game));
            assert!(!gate_open(&app));
            point(&mut app, OUTSIDE);
            press_key(&mut app, KeyCode::KeyW);
            app.update();
            assert!(game_input(&app, document).is_held(KEY));
            assert_eq!(
                game_input(&app, document).mouse_position(),
                OUTSIDE - ORIGIN
            );
            assert!(!editor_input(&app).is_held(KEY));

            send(&mut app, press(MouseButton::Left));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
            assert!(editor_input(&app).is_mouse_button_held(MouseButton::Left));
            assert!(!game_input(&app, document).is_held(KEY));
            assert!(!game_focused(&app, document));
            send(&mut app, release(MouseButton::Left));
            release_key(&mut app, KeyCode::KeyW);
            app.update();

            point(&mut app, INSIDE);
            send(&mut app, press(MouseButton::Left));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Game));
            assert!(!gate_open(&app));
            assert!(game_focused(&app, document));
            assert!(!game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            send(&mut app, release(MouseButton::Left));
            app.update();
            send(&mut app, press(MouseButton::Left));
            app.update();
            assert!(game_input(&app, document).is_mouse_button_held(MouseButton::Left));
            send(&mut app, release(MouseButton::Left));
            app.update();

            release_with_shift_f1(&mut app);
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            press_key(&mut app, KeyCode::KeyW);
            app.update();
            assert!(!game_input(&app, document).is_held(KEY));
            release_key(&mut app, KeyCode::KeyW);
            send(&mut app, press(MouseButton::Left));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Game));

            app.main_mut().world_mut().trigger(ReleaseInput(document));
            app.update();
            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(gate_open(&app));
        }

        fn point_under(app: &mut App, position: Vec2) {
            if let Some(view) = &mut app.get_resource_mut::<ViewportShown>().unwrap().0 {
                view.over = false;
            }
            send(app, WindowEvent::CursorMoved { position });
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
        fn ctrl_p_while_the_game_has_input_stops_once_and_leaves_no_key_held() {
            let (mut app, document) = editor_playing(plain_recipe);
            point(&mut app, INSIDE);
            press_key(&mut app, KeyCode::KeyW);
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
            assert!(!editor_input(&app).is_held(KEY));
            assert!(!game_input(&app, document).is_held(KEY));
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

        const PAD: GamepadId = GamepadId::new(0);

        fn connect_pad(app: &mut App) {
            send(
                app,
                WindowEvent::GamepadConnected {
                    id: PAD,
                    name: "Test pad".to_string(),
                },
            );
        }

        fn game_pad(app: &App, document: Entity) -> Option<&Gamepad> {
            game_input(app, document).gamepad(PAD)
        }

        #[test]
        fn a_pad_event_reaches_the_game_while_the_editor_has_input() {
            let (mut app, document) = editor_playing(plain_recipe);
            release_with_shift_f1(&mut app);

            connect_pad(&mut app);
            send(
                &mut app,
                WindowEvent::GamepadButton {
                    id: PAD,
                    button: GamepadButton::South,
                    value: 1.0,
                },
            );
            app.update();

            assert_eq!(owner(&app, document), Some(InputOwner::Editor));
            assert!(game_pad(&app, document).is_some_and(|pad| pad.is_held(GamepadButton::South)));
        }

        #[test]
        fn a_new_play_target_is_told_about_the_pads_already_connected() {
            let (mut app, document) = editor_editing(|_| {});
            connect_pad(&mut app);
            send(
                &mut app,
                WindowEvent::GamepadButton {
                    id: PAD,
                    button: GamepadButton::South,
                    value: 1.0,
                },
            );
            send(
                &mut app,
                WindowEvent::GamepadAxis {
                    id: PAD,
                    axis: GamepadAxis::LeftStickX,
                    value: 0.5,
                },
            );
            app.update();

            play_now(app.main_mut().world_mut(), document, plain_recipe, |_| {});
            app.update();
            app.update();

            let editor_pad = editor_input(&app).gamepad(PAD).unwrap();
            let pad = game_pad(&app, document).unwrap();
            assert_eq!(pad.name(), "Test pad");
            assert!(pad.is_held(GamepadButton::South));
            assert_eq!(
                pad.axis(GamepadAxis::LeftStickX),
                editor_pad.axis(GamepadAxis::LeftStickX)
            );
            assert!(pad.axis(GamepadAxis::LeftStickX) > 0.0);
        }
    }
}
