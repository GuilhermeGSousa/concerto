use std::collections::VecDeque;

use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, With, command::CommandQueue, events::event_writer::EventWriter,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, ToolCalls, ToolError};
use concerto_window::{
    events::{ScrollDelta, WindowEvent},
    input::{Input, KeyCode, MouseButton, PhysicalKey},
};
use glam::Vec2;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::hosted::{
    EditTarget, HostedApp, PlayState,
    input::{InViewport, InputOwner, ViewportShown, ViewportView},
};

/// Sends key, mouse button, pointer and wheel events to the game playing in the viewport, one per frame in order, as the editor window passes them on, and answers once the game has had the last; a key left pressed stays held until a later call releases it.
#[derive(Deserialize, JsonSchema)]
pub struct SendInput {
    /// The events, in order; points are pixels of the viewport's picture from its top-left, at the full size status gives as viewport.
    pub events: Vec<InputEvent>,
}

/// One input event: a key, a mouse button, a pointer move, or a wheel turn.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputEvent {
    Key(KeyInput),
    Button(ButtonInput),
    Move(MoveInput),
    Wheel(WheelInput),
}

/// A key pressed or released.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyInput {
    /// The key's name in the window crate's KeyCode, such as KeyW, Space, ArrowUp, ShiftLeft, Digit1 or F1.
    pub key: String,
    /// Whether the key goes down or comes up.
    pub pressed: bool,
    /// The text the press types, such as "w"; ignored on a release.
    pub text: Option<String>,
}

/// A mouse button pressed or released.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ButtonInput {
    /// The button.
    pub button: ButtonName,
    /// Whether the button goes down or comes up.
    pub pressed: bool,
    /// Where to move the pointer first, [x, y] in the picture's pixels; where it is when left out.
    pub at: Option<[f32; 2]>,
}

/// A mouse button by name.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ButtonName {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// The pointer moved to a point, with the mouse motion that takes it there.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveInput {
    /// Where the pointer goes, [x, y] in the picture's pixels.
    #[serde(rename = "move")]
    pub to: [f32; 2],
}

/// The mouse wheel turned.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WheelInput {
    /// How far, [x, y] in lines, as a wheel reports it.
    pub wheel: [f32; 2],
}

#[derive(Component)]
struct Sending {
    document: Entity,
    events: VecDeque<WindowEvent>,
    sent: usize,
    total: usize,
}

macro_rules! key_names {
    ($($key:ident)*) => {
        &[$((stringify!($key), KeyCode::$key)),*]
    };
}

const KEYS: &[(&str, KeyCode)] = key_names![
    Backquote Backslash BracketLeft BracketRight Comma Digit0 Digit1 Digit2 Digit3 Digit4 Digit5
    Digit6 Digit7 Digit8 Digit9 Equal IntlBackslash IntlRo IntlYen KeyA KeyB KeyC KeyD KeyE KeyF
    KeyG KeyH KeyI KeyJ KeyK KeyL KeyM KeyN KeyO KeyP KeyQ KeyR KeyS KeyT KeyU KeyV KeyW KeyX KeyY
    KeyZ Minus Period Quote Semicolon Slash AltLeft AltRight Backspace CapsLock ContextMenu
    ControlLeft ControlRight Enter SuperLeft SuperRight ShiftLeft ShiftRight Space Tab Convert
    KanaMode Lang1 Lang2 Lang3 Lang4 Lang5 NonConvert Delete End Help Home Insert PageDown PageUp
    ArrowDown ArrowLeft ArrowRight ArrowUp NumLock Numpad0 Numpad1 Numpad2 Numpad3 Numpad4 Numpad5
    Numpad6 Numpad7 Numpad8 Numpad9 NumpadAdd NumpadBackspace NumpadClear NumpadClearEntry
    NumpadComma NumpadDecimal NumpadDivide NumpadEnter NumpadEqual NumpadHash NumpadMemoryAdd
    NumpadMemoryClear NumpadMemoryRecall NumpadMemoryStore NumpadMemorySubtract NumpadMultiply
    NumpadParenLeft NumpadParenRight NumpadStar NumpadSubtract Escape Fn FnLock PrintScreen
    ScrollLock Pause BrowserBack BrowserFavorites BrowserForward BrowserHome BrowserRefresh
    BrowserSearch BrowserStop Eject LaunchApp1 LaunchApp2 LaunchMail MediaPlayPause MediaSelect
    MediaStop MediaTrackNext MediaTrackPrevious Power Sleep AudioVolumeDown AudioVolumeMute
    AudioVolumeUp WakeUp Meta Hyper Turbo Abort Resume Suspend Again Copy Cut Find Open Paste
    Props Select Undo Hiragana Katakana F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12 F13 F14 F15 F16
    F17 F18 F19 F20 F21 F22 F23 F24 F25 F26 F27 F28 F29 F30 F31 F32 F33 F34 F35
];

const RELEASED: &str = "The editor has the window's input, not the game: a click outside the viewport's picture or Shift+F1 gave it back. Call stop, then play, to give the game input again.";

pub(super) fn build(app: &mut App) {
    app.add_tool::<SendInput, _>(McpTool::new("send_input").needs(Needs::Playing), send_input)
        .add_system(LateUpdate, send_queued_input);
}

fn send_input(
    mut calls: ToolCalls<SendInput>,
    states: Query<&PlayState>,
    targets: Query<(Entity, Option<&InputOwner>), (With<EditTarget>, With<InViewport>)>,
    shown: Res<ViewportShown>,
    input: Res<Input>,
    mut cmd: CommandQueue,
) {
    let playing = states.iter().any(|state| state.is_on());
    for (arguments, reply) in calls.read() {
        if !playing {
            continue;
        }
        let (document, view) = match (targets.iter().next(), shown.0) {
            (Some((document, Some(InputOwner::Game))), Some(view)) => (document, view),
            (Some((_, Some(InputOwner::Editor))), Some(_)) => {
                reply.err(ToolError::new("input_released", RELEASED));
                continue;
            }
            _ => {
                reply.err(ToolError::new(
                    "input_released",
                    "The game playing is not in the viewport: it is still starting, or another document is active.",
                ));
                continue;
            }
        };
        if arguments.events.is_empty() {
            reply.err(ToolError::new(
                "invalid_arguments",
                "events is empty; give one or more.",
            ));
            continue;
        }
        match window_events(&arguments.events, &view, input.mouse_position()) {
            Ok(events) => {
                cmd.spawn((
                    PendingCall::new(reply, "the game to receive the input"),
                    Sending {
                        document,
                        total: events.len(),
                        events,
                        sent: 0,
                    },
                ));
            }
            Err(error) => reply.err(error),
        }
    }
}

fn send_queued_input(
    owners: Query<&InputOwner>,
    routed: Query<(), (With<EditTarget>, With<InViewport>)>,
    hosted: Query<&HostedApp>,
    sending: Query<(Entity, &PendingCall, &mut Sending)>,
    mut events: EventWriter<WindowEvent>,
    mut cmd: CommandQueue,
) {
    let Some((entity, pending, mut sending)) = sending.iter().next() else {
        return;
    };
    let document = sending.document;
    let owner = owners.get_entity(document).copied();
    let Some(event) = sending.events.pop_front() else {
        pending
            .reply
            .ok(sent(document, sending.total, owner, &hosted));
        cmd.despawn(entity);
        return;
    };
    let failure = match owner {
        Some(InputOwner::Game) if routed.get_entity(document).is_some() => None,
        Some(InputOwner::Game) => Some(ToolError::new(
            "input_released",
            format!(
                "The game left the viewport after {} of {} window events.",
                sending.sent, sending.total
            ),
        )),
        Some(InputOwner::Editor) => Some(ToolError::new(
            "input_released",
            format!(
                "Sent {} of {} window events. {RELEASED}",
                sending.sent, sending.total
            ),
        )),
        None => Some(ToolError::new(
            "not_playing",
            format!(
                "The game stopped after {} of {} window events.",
                sending.sent, sending.total
            ),
        )),
    };
    match failure {
        Some(error) => {
            pending.reply.err(error);
            cmd.despawn(entity);
        }
        None => {
            events.write(event);
            sending.sent += 1;
        }
    }
}

fn sent(
    document: Entity,
    total: usize,
    owner: Option<InputOwner>,
    hosted: &Query<&HostedApp>,
) -> String {
    let events = if total == 1 { "event" } else { "events" };
    let mut answer = format!(
        "Sent {total} window {events} to the game in document {}",
        document.to_id_string()
    );
    if owner.is_some() {
        let mut held: Vec<String> = hosted
            .get_entity(document)
            .and_then(|app| app.world().get_resource::<Input>())
            .map(|input| {
                input
                    .held_keys()
                    .filter_map(|key| match key {
                        PhysicalKey::Code(code) => Some(format!("{code:?}")),
                        PhysicalKey::Unidentified(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        held.sort_unstable();
        if !held.is_empty() {
            answer.push_str(&format!("; held in the game: {}", held.join(", ")));
        }
    }
    answer.push('.');
    match owner {
        Some(InputOwner::Game) => {}
        Some(InputOwner::Editor) => answer.push_str(" The editor has input now."),
        None => answer.push_str(" The game has stopped."),
    }
    answer
}

fn window_events(
    events: &[InputEvent],
    view: &ViewportView,
    mut pointer: Vec2,
) -> Result<VecDeque<WindowEvent>, ToolError> {
    let mut written = VecDeque::new();
    for (index, event) in events.iter().enumerate() {
        let invalid = |message: String| {
            ToolError::new("invalid_arguments", format!("events[{index}]: {message}"))
        };
        let mut move_to = |at: [f32; 2], written: &mut VecDeque<WindowEvent>| {
            let position = to_window(view, at).map_err(invalid)?;
            written.push_back(WindowEvent::CursorMoved { position });
            if position != pointer {
                written.push_back(WindowEvent::MouseMotion {
                    delta: position - pointer,
                });
            }
            pointer = position;
            Ok(())
        };
        match event {
            InputEvent::Key(key) => {
                let code = key_named(&key.key).map_err(invalid)?;
                written.push_back(WindowEvent::Key {
                    key: PhysicalKey::Code(code),
                    pressed: key.pressed,
                    text: key.text.clone().filter(|_| key.pressed),
                });
            }
            InputEvent::Button(button) => {
                if let Some(at) = button.at {
                    move_to(at, &mut written)?;
                }
                written.push_back(WindowEvent::MouseButton {
                    button: mouse_button(button.button),
                    pressed: button.pressed,
                });
            }
            InputEvent::Move(moved) => move_to(moved.to, &mut written)?,
            InputEvent::Wheel(wheel) => written.push_back(WindowEvent::MouseWheel {
                delta: ScrollDelta::Lines(Vec2::from(wheel.wheel)),
            }),
        }
    }
    Ok(written)
}

fn mouse_button(name: ButtonName) -> MouseButton {
    match name {
        ButtonName::Left => MouseButton::Left,
        ButtonName::Right => MouseButton::Right,
        ButtonName::Middle => MouseButton::Middle,
        ButtonName::Back => MouseButton::Back,
        ButtonName::Forward => MouseButton::Forward,
    }
}

fn to_window(view: &ViewportView, at: [f32; 2]) -> Result<Vec2, String> {
    let (width, height) = view.size;
    let inside = at[0] >= 0.0 && at[1] >= 0.0 && at[0] < width as f32 && at[1] < height as f32;
    if !inside {
        return Err(format!(
            "[{}, {}] is outside the viewport's picture, which is {width}x{height} pixels: [0, 0] is its top-left corner.",
            at[0], at[1]
        ));
    }
    Ok(view.origin + Vec2::from(at))
}

fn key_named(name: &str) -> Result<KeyCode, String> {
    if let Some((_, code)) = KEYS.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)) {
        return Ok(*code);
    }
    let close = close_keys(name);
    let close = if close.is_empty() {
        String::new()
    } else {
        format!("; close matches: {}", close.join(", "))
    };
    Err(format!(
        "no key named {name}{close}. Keys are named as the window crate's KeyCode names them, such as KeyW, Space, ArrowUp, ShiftLeft, ControlLeft, Digit1 and F1."
    ))
}

fn close_keys(name: &str) -> Vec<&'static str> {
    let wanted = name.to_lowercase();
    let mut close: Vec<(bool, usize, &'static str)> = KEYS
        .iter()
        .filter_map(|(key, _)| {
            let lower = key.to_lowercase();
            let distance = edit_distance(&wanted, &lower);
            let contains = lower.contains(&wanted);
            (contains || distance <= 2).then_some((!contains, distance, *key))
        })
        .collect();
    close.sort();
    close.into_iter().take(5).map(|(_, _, key)| key).collect()
}

fn edit_distance(from: &str, to: &str) -> usize {
    let to: Vec<char> = to.chars().collect();
    let mut row: Vec<usize> = (0..=to.len()).collect();
    for (i, a) in from.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in to.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(a != *b));
            diagonal = above;
        }
    }
    row[to.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ViewportView {
        ViewportView {
            origin: Vec2::new(100.0, 50.0),
            size: (640, 480),
            over: true,
            typing: false,
        }
    }

    #[test]
    fn a_point_on_the_picture_is_moved_by_the_viewports_origin_into_the_window() {
        assert_eq!(to_window(&view(), [0.0, 0.0]), Ok(Vec2::new(100.0, 50.0)));
        assert_eq!(
            to_window(&view(), [639.5, 479.0]),
            Ok(Vec2::new(739.5, 529.0))
        );
    }

    #[test]
    fn a_point_off_the_picture_is_refused_with_its_size() {
        for at in [[640.0, 0.0], [0.0, 480.0], [-1.0, 10.0], [f32::NAN, 10.0]] {
            let refused = to_window(&view(), at).unwrap_err();
            assert!(refused.contains("which is 640x480 pixels"), "{refused}");
        }
    }

    #[test]
    fn every_key_name_is_the_name_the_key_enum_gives_it() {
        for (name, code) in KEYS {
            assert_eq!(*name, format!("{code:?}"));
        }
        let mut names: Vec<&str> = KEYS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), KEYS.len());
    }

    #[test]
    fn a_key_is_found_by_its_name_in_any_case_and_an_unknown_one_lists_close_names() {
        assert_eq!(key_named("KeyW"), Ok(KeyCode::KeyW));
        assert_eq!(key_named("space"), Ok(KeyCode::Space));
        let unknown = key_named("Kew").unwrap_err();
        assert!(
            unknown.starts_with("no key named Kew; close matches: KeyW"),
            "{unknown}"
        );
        assert!(key_named("w").unwrap_err().contains("close matches: KeyW"));
        assert!(
            key_named("ArowUp")
                .unwrap_err()
                .contains("close matches: ArrowUp")
        );
    }

    #[test]
    fn a_move_writes_the_pointer_and_the_motion_to_it_and_a_press_at_a_point_moves_there_first() {
        let events = [
            InputEvent::Move(MoveInput { to: [10.0, 20.0] }),
            InputEvent::Button(ButtonInput {
                button: ButtonName::Left,
                pressed: true,
                at: Some([10.0, 20.0]),
            }),
            InputEvent::Key(KeyInput {
                key: "KeyW".into(),
                pressed: false,
                text: Some("w".into()),
            }),
        ];

        let written = window_events(&events, &view(), Vec2::new(100.0, 40.0)).unwrap();

        assert_eq!(
            Vec::from(written),
            vec![
                WindowEvent::CursorMoved {
                    position: Vec2::new(110.0, 70.0)
                },
                WindowEvent::MouseMotion {
                    delta: Vec2::new(10.0, 30.0)
                },
                WindowEvent::CursorMoved {
                    position: Vec2::new(110.0, 70.0)
                },
                WindowEvent::MouseButton {
                    button: MouseButton::Left,
                    pressed: true
                },
                WindowEvent::Key {
                    key: PhysicalKey::Code(KeyCode::KeyW),
                    pressed: false,
                    text: None
                },
            ]
        );
    }
}
