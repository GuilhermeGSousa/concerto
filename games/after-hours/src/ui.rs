//! Screens and HUD.
use concerto::{
    color::Color,
    ecs::{CommandQueue, Component, Entity, Query, Res, With, Without},
    ui::{
        material::UIMaterial,
        node::{AlignContent, AlignItems, FlexDirection, Position, UIInset, UINode},
        text::{FontFamily, TextComponent},
        transform::UIValue,
    },
};
use glam::Vec2;

use crate::{
    game::{Game, Phase},
    night::{self, NightState},
    player::{self, Flashlight, Player},
    scare::SCARE_SECONDS,
};

const PALE: Color = Color::srgba(0.86, 0.84, 0.78, 1.0);
const DIM: Color = Color::srgba(0.55, 0.54, 0.5, 1.0);
const RED: Color = Color::srgba(0.85, 0.12, 0.1, 1.0);
const GREEN: Color = Color::srgba(0.35, 0.85, 0.45, 1.0);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Title,
    Intro,
    Hud,
    Pause,
    Dead,
    Escaped,
}

/// A text node whose content is refreshed each frame.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Best,
    Prompt,
    NightTitle,
    NightClock,
    NightGoal,
    Keys,
    Clock,
    Message,
    DeadDetail,
    EscapedTitle,
    EscapedDetail,
    EscapedPrompt,
    PauseHint,
    Settings,
}

#[derive(Component)]
pub struct BatteryFill;

#[derive(Component)]
pub struct StaminaFill;

#[derive(Component)]
pub struct Fade;

const BAR_WIDTH: f32 = 140.0;
/// The last night of the "week"; after it the game carries on endlessly.
pub const FINAL_NIGHT: u32 = 5;

fn text(value: &str, size: f32, color: Color) -> TextComponent {
    TextComponent {
        text: value.to_string(),
        font_size: size,
        line_height: size * 1.35,
        font_family: FontFamily::Monospace,
        color,
        ..Default::default()
    }
}

fn full_screen(z: i32) -> UINode {
    UINode {
        width: UIValue::Percent(100.0),
        height: UIValue::Percent(100.0),
        position: Position::Absolute,
        inset: UIInset {
            top: UIValue::Px(0.0),
            left: UIValue::Px(0.0),
            ..Default::default()
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(AlignContent::Center),
        gap: Vec2::splat(10.0),
        z_index: z,
        ..Default::default()
    }
}

fn line(
    cmd: &mut CommandQueue,
    parent: Entity,
    value: &str,
    size: f32,
    color: Color,
    label: Option<Label>,
) {
    let mut entity = cmd.spawn((UINode::default(), text(value, size, color)));
    if let Some(label) = label {
        entity.insert(label);
    }
    let child = entity.entity();
    cmd.add_child(parent, child);
}

fn spacer(cmd: &mut CommandQueue, parent: Entity, height: f32) {
    let child = cmd
        .spawn(UINode {
            height: UIValue::Px(height),
            ..Default::default()
        })
        .entity();
    cmd.add_child(parent, child);
}

fn bar(cmd: &mut CommandQueue, parent: Entity, label: &str, fill: impl Component, color: Color) {
    let row = cmd
        .spawn(UINode {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Vec2::splat(8.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(parent, row);
    let name = cmd
        .spawn((
            UINode {
                width: UIValue::Px(64.0),
                ..Default::default()
            },
            text(label, 11.0, DIM),
        ))
        .entity();
    cmd.add_child(row, name);
    let track = cmd
        .spawn((
            UINode {
                width: UIValue::Px(BAR_WIDTH),
                height: UIValue::Px(6.0),
                ..Default::default()
            },
            UIMaterial::flat(Color::srgba(1.0, 1.0, 1.0, 0.12)),
        ))
        .entity();
    cmd.add_child(row, track);
    let fill = cmd
        .spawn((
            UINode {
                width: UIValue::Px(BAR_WIDTH),
                height: UIValue::Px(6.0),
                ..Default::default()
            },
            UIMaterial::flat(color),
            fill,
        ))
        .entity();
    cmd.add_child(track, fill);
}

const CONTROLS: &str = "WASD move    MOUSE look    SHIFT run\nF flashlight    ESC pause";

pub fn spawn_ui(mut cmd: CommandQueue) {
    let title = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            Screen::Title,
        ))
        .entity();
    line(&mut cmd, title, "AFTER HOURS", 64.0, PALE, None);
    line(
        &mut cmd,
        title,
        "the mannequins only move when you are not looking",
        15.0,
        DIM,
        None,
    );
    spacer(&mut cmd, title, 40.0);
    line(
        &mut cmd,
        title,
        "[ click to clock in ]",
        18.0,
        PALE,
        Some(Label::Prompt),
    );
    spacer(&mut cmd, title, 24.0);
    line(&mut cmd, title, CONTROLS, 12.0, DIM, None);
    line(&mut cmd, title, "", 12.0, DIM, Some(Label::Settings));
    line(&mut cmd, title, "", 12.0, DIM, Some(Label::Best));

    let intro = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::BLACK),
            Screen::Intro,
        ))
        .entity();
    line(
        &mut cmd,
        intro,
        "NIGHT 1",
        52.0,
        PALE,
        Some(Label::NightTitle),
    );
    line(
        &mut cmd,
        intro,
        "11:52 PM",
        18.0,
        DIM,
        Some(Label::NightClock),
    );
    spacer(&mut cmd, intro, 24.0);
    line(&mut cmd, intro, "", 14.0, DIM, Some(Label::NightGoal));

    let hud = cmd
        .spawn((
            UINode {
                width: UIValue::Percent(100.0),
                height: UIValue::Percent(100.0),
                position: Position::Absolute,
                inset: UIInset {
                    top: UIValue::Px(0.0),
                    left: UIValue::Px(0.0),
                    ..Default::default()
                },
                z_index: 1,
                ..Default::default()
            },
            Screen::Hud,
        ))
        .entity();
    let top_left = cmd
        .spawn(UINode {
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(18.0),
                left: UIValue::Px(22.0),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            gap: Vec2::splat(2.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(hud, top_left);
    line(
        &mut cmd,
        top_left,
        "KEYS 0/3",
        16.0,
        PALE,
        Some(Label::Keys),
    );
    line(
        &mut cmd,
        top_left,
        "11:52 PM",
        12.0,
        DIM,
        Some(Label::Clock),
    );
    let bottom_left = cmd
        .spawn(UINode {
            position: Position::Absolute,
            inset: UIInset {
                bottom: UIValue::Px(20.0),
                left: UIValue::Px(22.0),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            gap: Vec2::splat(6.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(hud, bottom_left);
    bar(
        &mut cmd,
        bottom_left,
        "BATTERY",
        BatteryFill,
        Color::srgba(0.95, 0.85, 0.5, 0.8),
    );
    bar(
        &mut cmd,
        bottom_left,
        "BREATH",
        StaminaFill,
        Color::srgba(0.7, 0.75, 0.8, 0.5),
    );
    let center = cmd
        .spawn(UINode {
            width: UIValue::Percent(100.0),
            height: UIValue::Percent(100.0),
            position: Position::Absolute,
            inset: UIInset {
                top: UIValue::Px(0.0),
                left: UIValue::Px(0.0),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::Center),
            justify_content: Some(AlignContent::Center),
            ..Default::default()
        })
        .entity();
    cmd.add_child(hud, center);
    let dot = cmd
        .spawn((
            UINode {
                width: UIValue::Px(3.0),
                height: UIValue::Px(3.0),
                ..Default::default()
            },
            UIMaterial::flat(Color::srgba(1.0, 1.0, 1.0, 0.35)),
        ))
        .entity();
    cmd.add_child(center, dot);
    let message_row = cmd
        .spawn(UINode {
            width: UIValue::Percent(100.0),
            position: Position::Absolute,
            inset: UIInset {
                bottom: UIValue::Px(70.0),
                left: UIValue::Px(0.0),
                ..Default::default()
            },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .entity();
    cmd.add_child(hud, message_row);
    line(&mut cmd, message_row, "", 15.0, PALE, Some(Label::Message));

    let pause = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            Screen::Pause,
        ))
        .entity();
    line(&mut cmd, pause, "PAUSED", 40.0, PALE, None);
    line(&mut cmd, pause, "", 16.0, DIM, Some(Label::PauseHint));
    spacer(&mut cmd, pause, 20.0);
    line(&mut cmd, pause, CONTROLS, 12.0, DIM, None);
    spacer(&mut cmd, pause, 14.0);
    line(&mut cmd, pause, "", 12.0, DIM, Some(Label::Settings));

    let dead = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::BLACK),
            Screen::Dead,
        ))
        .entity();
    line(&mut cmd, dead, "YOU WERE CAUGHT", 44.0, RED, None);
    line(&mut cmd, dead, "", 15.0, DIM, Some(Label::DeadDetail));
    spacer(&mut cmd, dead, 30.0);
    line(
        &mut cmd,
        dead,
        "[ click to try the night again ]",
        16.0,
        PALE,
        None,
    );

    let escaped = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::BLACK),
            Screen::Escaped,
        ))
        .entity();
    line(
        &mut cmd,
        escaped,
        "YOU MADE IT OUT",
        44.0,
        GREEN,
        Some(Label::EscapedTitle),
    );
    line(&mut cmd, escaped, "", 15.0, DIM, Some(Label::EscapedDetail));
    spacer(&mut cmd, escaped, 30.0);
    line(
        &mut cmd,
        escaped,
        "",
        16.0,
        PALE,
        Some(Label::EscapedPrompt),
    );

    cmd.spawn((
        full_screen(20),
        UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.0)),
        Fade,
    ));
}

fn visible_screen(game: &Game) -> Option<Screen> {
    match game.phase {
        Phase::Title => Some(Screen::Title),
        Phase::Intro => Some(Screen::Intro),
        Phase::Playing if game.paused => Some(Screen::Pause),
        Phase::Playing => Some(Screen::Hud),
        Phase::Caught => None,
        Phase::Dead => Some(Screen::Dead),
        Phase::Escaped => Some(Screen::Escaped),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_ui(
    screens: Query<(&Screen, &mut UINode), Without<Label>>,
    labels: Query<(&Label, &mut TextComponent)>,
    batteries: Query<&mut UINode, (With<BatteryFill>, Without<Screen>, Without<Label>)>,
    staminas: Query<
        &mut UINode,
        (
            With<StaminaFill>,
            Without<Screen>,
            Without<Label>,
            Without<BatteryFill>,
        ),
    >,
    fades: Query<&mut UIMaterial, With<Fade>>,
    players: Query<&Player>,
    flashlights: Query<&Flashlight>,
    game: Res<Game>,
    state: Res<NightState>,
    settings: Res<crate::player::Settings>,
) {
    let shown = if crate::platform::debug_flag("noui") {
        None
    } else {
        visible_screen(&game)
    };
    for (screen, mut node) in screens.iter() {
        let visible = Some(*screen) == shown;
        if node.visible != visible {
            node.visible = visible;
        }
    }

    let player = players.iter().next();
    let keys = player.map(|p| p.keys).unwrap_or(0);
    let t = game.phase_time;
    let blink = (t * 2.2).sin() > -0.3;
    for (label, mut component) in labels.iter() {
        let (value, color): (String, Option<Color>) = match label {
            Label::Best => (
                if game.best_night > 1 {
                    format!("furthest: night {}", game.best_night)
                } else {
                    String::new()
                },
                None,
            ),
            Label::Prompt => (
                "[ click to clock in ]".into(),
                Some(if blink { PALE } else { DIM }),
            ),
            Label::NightTitle => (format!("NIGHT {}", game.night), None),
            Label::NightClock => (night::clock_text(0.0), None),
            Label::NightGoal => (
                format!(
                    "find the {} register keys\nlock up. get out through the staff exit.",
                    state.keys_total
                ),
                None,
            ),
            Label::Keys => (
                if state.unlocked {
                    "EXIT UNLOCKED".into()
                } else {
                    format!("KEYS {keys}/{}", state.keys_total)
                },
                Some(if state.unlocked { GREEN } else { PALE }),
            ),
            Label::Clock => (night::clock_text(game.night_time), None),
            Label::Message => (
                state
                    .message
                    .as_ref()
                    .map(|(m, _)| m.clone())
                    .unwrap_or_default(),
                None,
            ),
            Label::DeadDetail => (
                format!(
                    "night {} · caught at {}",
                    game.night,
                    night::clock_text(game.night_time)
                ),
                None,
            ),
            Label::EscapedTitle => (
                if game.night == FINAL_NIGHT {
                    "YOU SURVIVED THE WEEK".into()
                } else {
                    "YOU MADE IT OUT".into()
                },
                None,
            ),
            Label::Settings => (
                format!(
                    "[ ] look speed {:.2}    - = volume {:.0}%    I invert look: {}",
                    settings.sensitivity_scale(),
                    settings.volume * 100.0,
                    if settings.invert_y { "on" } else { "off" }
                ),
                None,
            ),
            Label::EscapedDetail if game.night == FINAL_NIGHT => (
                "five nights. the store opens at nine.\nthe mannequins will be back in the window by then.".into(),
                None,
            ),
            Label::EscapedDetail => (
                format!(
                    "night {} survived · clocked out at {}",
                    game.night,
                    night::clock_text(game.night_time)
                ),
                None,
            ),
            Label::EscapedPrompt if game.night >= FINAL_NIGHT => (
                format!("[ click to keep working nights: night {} ]", game.night + 1),
                None,
            ),
            Label::EscapedPrompt => (
                format!("[ click to clock in for night {} ]", game.night + 1),
                None,
            ),
            Label::PauseHint => ("click to return to the floor".into(), None),
        };
        if component.text != value {
            component.text = value;
        }
        if let Some(color) = color
            && component.color != color
        {
            component.color = color;
        }
    }

    let battery = flashlights.iter().next().map(|f| f.battery).unwrap_or(1.0);
    for mut node in batteries.iter() {
        let width = UIValue::Px(BAR_WIDTH * battery);
        if node.width != width {
            node.width = width;
        }
    }
    let stamina = player.map(player::stamina_fraction).unwrap_or(1.0);
    for mut node in staminas.iter() {
        let width = UIValue::Px(BAR_WIDTH * stamina);
        if node.width != width {
            node.width = width;
        }
    }

    let alpha = match game.phase {
        Phase::Playing => (1.0 - t / 1.2).max(0.0),
        Phase::Caught => ((t - SCARE_SECONDS + 0.15) / 0.15).clamp(0.0, 1.0),
        _ => 0.0,
    };
    for mut material in fades.iter() {
        let color = Color::srgba(0.0, 0.0, 0.0, alpha).to_linear();
        if material.color != color {
            material.color = color;
        }
    }
}
