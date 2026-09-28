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
    story,
};

pub const FONT: &str = "IM FELL English";
const PALE: Color = Color::srgba(0.88, 0.82, 0.7, 1.0);
const DIM: Color = Color::srgba(0.58, 0.53, 0.45, 1.0);
const RED: Color = Color::srgba(0.7, 0.1, 0.08, 1.0);
const GOLD: Color = Color::srgba(0.85, 0.7, 0.4, 1.0);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Title,
    Intro,
    Hud,
    Reading,
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
    NightHeading,
    NightStory,
    NightGoal,
    NightContinue,
    Lots,
    Clock,
    Message,
    Action,
    Room,
    Reading,
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
pub struct HoldFill;

#[derive(Component)]
pub struct Fade;

const BAR_WIDTH: f32 = 140.0;
const HOLD_WIDTH: f32 = 120.0;
/// The last night of the story; after it the house keeps you.
pub const FINAL_NIGHT: u32 = 5;

fn text(value: &str, size: f32, color: Color) -> TextComponent {
    TextComponent {
        text: value.to_string(),
        font_size: size,
        line_height: size * 1.3,
        font_family: FontFamily::Name(FONT.into()),
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

fn anchored(cmd: &mut CommandQueue, parent: Entity, inset: UIInset, center: bool) -> Entity {
    let node = cmd
        .spawn(UINode {
            width: if center {
                UIValue::Percent(100.0)
            } else {
                UIValue::Auto
            },
            position: Position::Absolute,
            inset,
            flex_direction: FlexDirection::Column,
            align_items: center.then_some(AlignItems::Center),
            gap: Vec2::splat(4.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(parent, node);
    node
}

fn bar(
    cmd: &mut CommandQueue,
    parent: Entity,
    label: &str,
    width: f32,
    fill: impl Component,
    color: Color,
) {
    let row = cmd
        .spawn(UINode {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Vec2::splat(8.0),
            ..Default::default()
        })
        .entity();
    cmd.add_child(parent, row);
    if !label.is_empty() {
        let name = cmd
            .spawn((
                UINode {
                    width: UIValue::Px(56.0),
                    ..Default::default()
                },
                text(label, 14.0, DIM),
            ))
            .entity();
        cmd.add_child(row, name);
    }
    let track = cmd
        .spawn((
            UINode {
                width: UIValue::Px(width),
                height: UIValue::Px(4.0),
                ..Default::default()
            },
            UIMaterial::flat(Color::srgba(1.0, 0.9, 0.7, if label.is_empty() { 0.0 } else { 0.1 })),
        ))
        .entity();
    cmd.add_child(row, track);
    let fill = cmd
        .spawn((
            UINode {
                width: UIValue::Px(width),
                height: UIValue::Px(4.0),
                ..Default::default()
            },
            UIMaterial::flat(color),
            fill,
        ))
        .entity();
    cmd.add_child(track, fill);
}

const CONTROLS: &str = "WASD walk     mouse look     shift hurry\nE catalogue, read     F lantern shutter     esc pause";

pub fn spawn_ui(mut cmd: CommandQueue) {
    let title = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            Screen::Title,
        ))
        .entity();
    line(&mut cmd, title, story::TITLE, 84.0, PALE, None);
    line(&mut cmd, title, story::TAGLINE, 20.0, DIM, None);
    spacer(&mut cmd, title, 40.0);
    line(
        &mut cmd,
        title,
        "click to enter Marrow House",
        22.0,
        PALE,
        Some(Label::Prompt),
    );
    spacer(&mut cmd, title, 24.0);
    line(&mut cmd, title, CONTROLS, 15.0, DIM, None);
    line(&mut cmd, title, "", 14.0, DIM, Some(Label::Settings));
    line(&mut cmd, title, "", 14.0, DIM, Some(Label::Best));

    let intro = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.02, 0.015, 0.01, 1.0)),
            Screen::Intro,
        ))
        .entity();
    line(&mut cmd, intro, "", 44.0, PALE, Some(Label::NightTitle));
    line(&mut cmd, intro, "", 18.0, DIM, Some(Label::NightHeading));
    spacer(&mut cmd, intro, 20.0);
    line(&mut cmd, intro, "", 19.0, PALE, Some(Label::NightStory));
    spacer(&mut cmd, intro, 20.0);
    line(&mut cmd, intro, "", 16.0, GOLD, Some(Label::NightGoal));
    spacer(&mut cmd, intro, 20.0);
    line(&mut cmd, intro, "", 16.0, DIM, Some(Label::NightContinue));

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
    let top_left = anchored(
        &mut cmd,
        hud,
        UIInset {
            top: UIValue::Px(18.0),
            left: UIValue::Px(22.0),
            ..Default::default()
        },
        false,
    );
    line(&mut cmd, top_left, "", 20.0, PALE, Some(Label::Lots));
    line(&mut cmd, top_left, "", 15.0, DIM, Some(Label::Clock));
    let top = anchored(
        &mut cmd,
        hud,
        UIInset {
            top: UIValue::Px(22.0),
            left: UIValue::Px(0.0),
            ..Default::default()
        },
        true,
    );
    line(&mut cmd, top, "", 24.0, DIM, Some(Label::Room));
    let bottom_left = anchored(
        &mut cmd,
        hud,
        UIInset {
            bottom: UIValue::Px(20.0),
            left: UIValue::Px(22.0),
            ..Default::default()
        },
        false,
    );
    bar(
        &mut cmd,
        bottom_left,
        "oil",
        BAR_WIDTH,
        BatteryFill,
        Color::srgba(0.95, 0.75, 0.4, 0.8),
    );
    bar(
        &mut cmd,
        bottom_left,
        "breath",
        BAR_WIDTH,
        StaminaFill,
        Color::srgba(0.7, 0.68, 0.62, 0.5),
    );
    let center = cmd
        .spawn(full_screen(0))
        .entity();
    cmd.add_child(hud, center);
    let dot = cmd
        .spawn((
            UINode {
                width: UIValue::Px(3.0),
                height: UIValue::Px(3.0),
                ..Default::default()
            },
            UIMaterial::flat(Color::srgba(1.0, 0.9, 0.7, 0.3)),
        ))
        .entity();
    cmd.add_child(center, dot);
    let under = anchored(
        &mut cmd,
        hud,
        UIInset {
            top: UIValue::Percent(56.0),
            left: UIValue::Px(0.0),
            ..Default::default()
        },
        true,
    );
    line(&mut cmd, under, "", 17.0, PALE, Some(Label::Action));
    bar(
        &mut cmd,
        under,
        "",
        HOLD_WIDTH,
        HoldFill,
        Color::srgba(0.9, 0.8, 0.55, 0.9),
    );
    let message_row = anchored(
        &mut cmd,
        hud,
        UIInset {
            bottom: UIValue::Px(70.0),
            left: UIValue::Px(0.0),
            ..Default::default()
        },
        true,
    );
    line(&mut cmd, message_row, "", 19.0, PALE, Some(Label::Message));

    let reading = cmd
        .spawn((
            full_screen(5),
            UIMaterial::flat(Color::srgba(0.05, 0.035, 0.02, 0.88)),
            Screen::Reading,
        ))
        .entity();
    line(&mut cmd, reading, "", 21.0, PALE, Some(Label::Reading));
    spacer(&mut cmd, reading, 24.0);
    line(
        &mut cmd,
        reading,
        "E  put the page down",
        15.0,
        DIM,
        None,
    );

    let pause = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            Screen::Pause,
        ))
        .entity();
    line(&mut cmd, pause, "Paused", 44.0, PALE, None);
    line(&mut cmd, pause, "", 18.0, DIM, Some(Label::PauseHint));
    spacer(&mut cmd, pause, 20.0);
    line(&mut cmd, pause, CONTROLS, 15.0, DIM, None);
    spacer(&mut cmd, pause, 14.0);
    line(&mut cmd, pause, "", 14.0, DIM, Some(Label::Settings));

    let dead = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::BLACK),
            Screen::Dead,
        ))
        .entity();
    line(&mut cmd, dead, "You looked away.", 46.0, RED, None);
    line(&mut cmd, dead, "", 18.0, DIM, Some(Label::DeadDetail));
    spacer(&mut cmd, dead, 30.0);
    line(
        &mut cmd,
        dead,
        "click to begin the night again",
        18.0,
        PALE,
        None,
    );

    let escaped = cmd
        .spawn((
            full_screen(10),
            UIMaterial::flat(Color::srgba(0.02, 0.015, 0.01, 1.0)),
            Screen::Escaped,
        ))
        .entity();
    line(&mut cmd, escaped, "", 46.0, PALE, Some(Label::EscapedTitle));
    line(&mut cmd, escaped, "", 18.0, DIM, Some(Label::EscapedDetail));
    spacer(&mut cmd, escaped, 30.0);
    line(&mut cmd, escaped, "", 18.0, PALE, Some(Label::EscapedPrompt));

    cmd.spawn((
        full_screen(20),
        UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 0.0)),
        Fade,
    ));
}

fn visible_screens(game: &Game, state: &NightState) -> [Option<Screen>; 2] {
    match game.phase {
        Phase::Title => [Some(Screen::Title), None],
        Phase::Intro => [Some(Screen::Intro), None],
        Phase::Playing if game.paused => [Some(Screen::Pause), None],
        Phase::Playing if state.reading.is_some() => [Some(Screen::Hud), Some(Screen::Reading)],
        Phase::Playing => [Some(Screen::Hud), None],
        Phase::Caught => [None, None],
        Phase::Dead => [Some(Screen::Dead), None],
        Phase::Escaped => [Some(Screen::Escaped), None],
    }
}

fn number_word(n: u32) -> &'static str {
    ["none", "one", "two", "three", "four", "five", "six"]
        .get(n as usize)
        .copied()
        .unwrap_or("many")
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
    holds: Query<
        (&mut UINode, &mut UIMaterial),
        (
            With<HoldFill>,
            Without<Screen>,
            Without<Label>,
            Without<BatteryFill>,
            Without<StaminaFill>,
        ),
    >,
    fades: Query<&mut UIMaterial, (With<Fade>, Without<HoldFill>)>,
    (players, flashlights): (Query<&Player>, Query<&Flashlight>),
    (game, state, settings): (Res<Game>, Res<NightState>, Res<crate::player::Settings>),
) {
    let shown = if crate::platform::debug_flag("noui") {
        [None, None]
    } else {
        visible_screens(&game, &state)
    };
    for (screen, mut node) in screens.iter() {
        let visible = shown.contains(&Some(*screen));
        if node.visible != visible {
            node.visible = visible;
        }
    }

    let t = game.phase_time;
    let blink = (t * 2.2).sin() > -0.3;
    let (heading, tale) = story::intro(game.night);
    let final_night = game.night == FINAL_NIGHT;
    for (label, mut component) in labels.iter() {
        let (value, color): (String, Option<Color>) = match label {
            Label::Best => (
                if game.best_night > 1 {
                    format!("furthest: night {}", game.best_night.min(99))
                } else {
                    String::new()
                },
                None,
            ),
            Label::Prompt => (
                "click to enter Marrow House".into(),
                Some(if blink { PALE } else { DIM }),
            ),
            Label::NightTitle => (format!("Night {}", game.night), None),
            Label::NightHeading => (heading.into(), None),
            Label::NightStory => (tale.into(), None),
            Label::NightGoal => (story::goal(state.lots_total), None),
            Label::NightContinue => (
                if game.phase_time > 1.5 {
                    "click to go in".into()
                } else {
                    String::new()
                },
                Some(if blink { PALE } else { DIM }),
            ),
            Label::Lots => (
                if state.unlocked {
                    "Every lot catalogued. Sign the ledger.".into()
                } else {
                    format!(
                        "Lots catalogued: {} of {}",
                        number_word(state.lots_done as u32),
                        number_word(state.lots_total as u32)
                    )
                },
                Some(if state.unlocked { GOLD } else { PALE }),
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
            Label::Action => (
                if state.reading.is_some() {
                    String::new()
                } else {
                    state.prompt.clone().unwrap_or_default()
                },
                None,
            ),
            Label::Room => (
                state
                    .room
                    .map(|(name, _)| name.to_string())
                    .unwrap_or_default(),
                None,
            ),
            Label::Reading => (state.reading.unwrap_or_default().into(), None),
            Label::DeadDetail => (
                format!(
                    "Night {}, a little after {}",
                    game.night,
                    night::clock_text(game.night_time)
                ),
                None,
            ),
            Label::EscapedTitle if final_night => (story::ENDING_TITLE.into(), Some(RED)),
            Label::EscapedTitle => ("The ledger is signed.".into(), Some(PALE)),
            Label::EscapedDetail if final_night => (story::ENDING.into(), None),
            Label::EscapedDetail => (
                format!(
                    "Night {} survived. You let yourself out at {}.",
                    game.night,
                    night::clock_text(game.night_time)
                ),
                None,
            ),
            Label::EscapedPrompt if game.night >= FINAL_NIGHT => (
                format!("click to go back in: night {}", game.night + 1),
                None,
            ),
            Label::EscapedPrompt => (
                format!("click to return for night {}", game.night + 1),
                None,
            ),
            Label::PauseHint => ("click to return to the house".into(), None),
            Label::Settings => (
                format!(
                    "[ ] look speed {:.2}     - = volume {:.0}%     I invert look: {}",
                    settings.sensitivity_scale(),
                    settings.volume * 100.0,
                    if settings.invert_y { "on" } else { "off" }
                ),
                None,
            ),
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

    if let Some(player) = players.iter().next() {
        let stamina = player::stamina_fraction(player);
        for mut node in staminas.iter() {
            let w = UIValue::Px(BAR_WIDTH * stamina);
            if node.width != w {
                node.width = w;
            }
        }
    }
    if let Some(flashlight) = flashlights.iter().next() {
        for mut node in batteries.iter() {
            let w = UIValue::Px(BAR_WIDTH * flashlight.battery);
            if node.width != w {
                node.width = w;
            }
        }
    }
    for (mut node, mut material) in holds.iter() {
        let w = UIValue::Px(HOLD_WIDTH * state.hold.clamp(0.0, 1.0));
        if node.width != w {
            node.width = w;
        }
        let alpha = if state.hold > 0.0 { 0.9 } else { 0.0 };
        let color = Color::srgba(0.9, 0.8, 0.55, alpha).to_linear();
        if material.color != color {
            material.color = color;
        }
    }

    let alpha = match game.phase {
        Phase::Playing => (1.0 - t / 1.2).max(0.0),
        Phase::Caught => ((t - crate::scare::SCARE_SECONDS + 0.15) / 0.15).clamp(0.0, 1.0),
        _ => 0.0,
    };
    for mut material in fades.iter() {
        let color = Color::srgba(0.0, 0.0, 0.0, alpha).to_linear();
        if material.color != color {
            material.color = color;
        }
    }
}
