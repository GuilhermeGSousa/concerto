use concerto::{
    animation::{graph::AnimationGraph, player::AnimationPlayer},
    color::Color,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, With,
        signal::{
            On,
            listener::{IntoListener, Listener},
        },
    },
    foundation::assets::asset_store::AssetStore,
    motion_matching::{CharacterDisplacement, CharacterLink, MotionDatabase, MotionMatchingNode},
    physics::ground::{GroundProbe, GroundState},
    ui::{
        elements::prelude::*,
        material::UIMaterial,
        node::{UINode, UIRect},
        slider::UISliderChanged,
        text::{FontFamily, UIText},
        theme::UITheme,
        transform::UIValue,
    },
};

use crate::body::{Body, Stat};
use crate::character::{PendingLink, Player};
use crate::footsteps::{Foot, FootstepLog};

const PANEL_WIDTH: f32 = 230.0;
const PANEL_PADDING: UIRect = UIRect {
    top: 6.0,
    right: 10.0,
    bottom: 6.0,
    left: 10.0,
};
const LINE_HEIGHT: f32 = 16.0;
const LINES: usize = 13;
const SLIDER_GAP: f32 = 8.0;
const SLIDER_LABEL_WIDTH: f32 = 110.0;
const SLIDER_WIDTH: f32 = 100.0;
const SLIDER_HEIGHT: f32 = 12.0;
const STATS_CLEARANCE: f32 = 54.0;

/// Marker for the readout's panel, whose colour tracks the grounded state.
#[derive(Component)]
pub(crate) struct GroundedPanel;

/// Marker for the readout's text node.
#[derive(Component)]
pub(crate) struct GroundedText;

struct Matching {
    entry: Option<usize>,
    frame: usize,
    cost: Option<f32>,
}

fn method_name(method: &CharacterDisplacement) -> String {
    match method {
        CharacterDisplacement::Adjustment(_) => "Adjustment".to_string(),
        CharacterDisplacement::Clamp(_) => "Clamp".to_string(),
        CharacterDisplacement::Synchronization(sync) => {
            format!("Synchronization {:.1}", sync.data_factor)
        }
    }
}

fn readout(
    is_grounded: bool,
    state: &str,
    matching: Option<&Matching>,
    method: &CharacterDisplacement,
    faces_camera: bool,
    foot_locking: bool,
) -> String {
    let dashes = || "--".to_string();
    let entry = matching
        .and_then(|matching| matching.entry)
        .map_or_else(dashes, |entry| entry.to_string());
    let frame = matching.map_or_else(dashes, |matching| matching.frame.to_string());
    let cost = matching
        .and_then(|matching| matching.cost)
        .filter(|cost| !cost.is_nan())
        .map_or_else(dashes, |cost| format!("{cost:.3}"));
    let method = method_name(method);
    let faces_camera = if faces_camera { "on" } else { "off" };
    let foot_locking = if foot_locking { "on" } else { "off" };
    format!(
        "grounded: {is_grounded}\n\
         state:    {state}\n\
         entry:    {entry}\n\
         frame:    {frame}\n\
         cost:     {cost}\n\
         WASD/arrows move, Shift run\n\
         1-5 toggle gizmo layers\n\
         F face camera: {faces_camera}\n\
         L foot lock: {foot_locking}\n\
         M method: {method}"
    )
}

fn body_lines(armed: bool, aiming: bool, steps: u32, last: Option<Foot>) -> String {
    let pistol = match (armed, aiming) {
        (false, _) => "holstered",
        (true, false) => "out",
        (true, true) => "aiming",
    };
    let last = last.map_or("--", Foot::letter);
    format!(
        "G pistol: {pistol}\n\
         RMB aim, LMB fire\n\
         steps: {steps} last: {last}"
    )
}

fn stat_label(stat: Stat) -> String {
    let range = stat.range();
    format!("{} {}-{}", stat.label(), range.min, range.max)
}

fn set_stat(stat: Stat) -> Listener<UISliderChanged> {
    (move |on: On<UISliderChanged>, bodies: Query<&mut Body>| {
        for mut body in bodies.iter() {
            body.set_stat(stat, on.signal().value);
        }
    })
    .into_listener()
}

/// Spawns one slider per body stat in a column under the readout; dragging one sets its stat on every `Body`.
pub(crate) fn spawn_stat_sliders(theme: Res<UITheme>, mut cmd: CommandQueue) {
    let readout_height = LINE_HEIGHT * LINES as f32 + PANEL_PADDING.top + PANEL_PADDING.bottom;
    let defaults = Body::default();
    cmd.spawn(
        theme
            .column()
            .width(UIValue::Px(PANEL_WIDTH))
            .padding(UIRect {
                top: STATS_CLEARANCE + readout_height + SLIDER_GAP,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            }),
    )
    .add_child_with(
        theme
            .panel()
            .column()
            .width(UIValue::Px(PANEL_WIDTH))
            .padding(PANEL_PADDING),
        |mut panel| {
            for stat in Stat::ALL {
                let range = stat.range();
                panel.add_child_with(theme.row().fixed(), |mut row| {
                    row.add_child(
                        theme
                            .label(stat_label(stat))
                            .font_size(12.0)
                            .width(UIValue::Px(SLIDER_LABEL_WIDTH))
                            .fixed(),
                    );
                    row.add_child(
                        theme
                            .slider(defaults.stat(stat), range.min, range.max)
                            .size(UIValue::Px(SLIDER_WIDTH), UIValue::Px(SLIDER_HEIGHT))
                            .on_change(set_stat(stat)),
                    );
                });
            }
        },
    );
}

fn panel_color(is_grounded: bool) -> Color {
    match is_grounded {
        true => Color::rgba(0.0, 0.35, 0.1, 0.6),
        false => Color::rgba(0.45, 0.06, 0.06, 0.6),
    }
}

pub(crate) fn spawn_grounded_overlay(mut cmd: CommandQueue) {
    let text_width = PANEL_WIDTH - PANEL_PADDING.left - PANEL_PADDING.right;
    let text_height = LINE_HEIGHT * LINES as f32;
    let panel_height = text_height + PANEL_PADDING.top + PANEL_PADDING.bottom;
    // Every UI root lays out from the window origin and a root's own margin does not move it,
    // so this transparent spacer (no UIMaterial, hence never drawn) pads the panel down clear
    // of the frame-stats overlay above it.
    cmd.spawn((UINode::default()
        .with_size(
            UIValue::Px(PANEL_WIDTH),
            UIValue::Px(STATS_CLEARANCE + panel_height),
        )
        .with_padding(UIRect {
            top: STATS_CLEARANCE,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }),))
        .add_child_with(
            (
                UINode::default()
                    .with_size(UIValue::Px(PANEL_WIDTH), UIValue::Px(panel_height))
                    .with_padding(PANEL_PADDING),
                UIMaterial::flat(panel_color(false)),
                GroundedPanel,
            ),
            |mut panel| {
                panel.add_child((
                    UINode::default()
                        .with_size(UIValue::Px(text_width), UIValue::Px(text_height))
                        .with_flex_shrink(0.0),
                    UIText {
                        text: "grounded: --".to_string(),
                        font_size: 12.0,
                        line_height: LINE_HEIGHT,
                        font_family: FontFamily::Monospace,
                        ..Default::default()
                    },
                    GroundedText,
                ));
            },
        );
}

fn belongs_to(
    capsule: Entity,
    link: Option<&CharacterLink>,
    pending: Option<&PendingLink>,
) -> bool {
    link.and_then(|link| link.capsule.entity())
        .or(pending.map(|pending| pending.capsule))
        == Some(capsule)
}

pub(crate) fn update_grounded_overlay(
    probes: Query<(Entity, &GroundProbe, &Player, Option<&Body>)>,
    footsteps: Option<Res<FootstepLog>>,
    characters: Query<(
        &CharacterDisplacement,
        Option<&CharacterLink>,
        Option<&PendingLink>,
    )>,
    players: Query<&AnimationPlayer>,
    graphs: Res<AssetStore<AnimationGraph>>,
    databases: Res<AssetStore<MotionDatabase>>,
    texts: Query<&mut UIText, With<GroundedText>>,
    panels: Query<&mut UIMaterial, With<GroundedPanel>>,
) {
    let Some((capsule, probe, player_state, body)) = probes.iter().next() else {
        return;
    };
    let Some((method, link, _)) = characters
        .iter()
        .find(|(_, link, pending)| belongs_to(capsule, *link, *pending))
    else {
        return;
    };

    let is_grounded = probe.is_grounded();
    let state = match probe.ground() {
        GroundState::InAir => "InAir",
        GroundState::OnGround(_) => "OnGround",
        GroundState::OnSteepGround(_) => "OnSteepGround",
    };

    let mut matching = None;
    if let Some(player) = link
        .and_then(|link| link.skeleton.entity())
        .and_then(|skeleton| players.get_entity(skeleton))
    {
        player.for_each_state::<MotionMatchingNode>(&graphs, |node, node_state, _| {
            let Some(last) = node_state.last_query() else {
                return;
            };
            if matching.is_some() {
                return;
            }
            matching = Some(Matching {
                entry: databases
                    .get(&node.database)
                    .and_then(|database| database.entry_of(node_state.frame())),
                frame: node_state.frame(),
                cost: last.best_cost,
            });
        });
    }

    let text = readout(
        is_grounded,
        state,
        matching.as_ref(),
        method,
        player_state.faces_camera,
        player_state.foot_locking,
    );
    let text = format!(
        "{text}\n{}",
        body_lines(
            body.is_some_and(|body| body.armed),
            body.is_some_and(|body| body.aiming),
            footsteps.as_ref().map_or(0, |log| log.count),
            footsteps.as_ref().and_then(|log| log.last),
        )
    );
    for mut ui_text in texts.iter() {
        if ui_text.text != text {
            ui_text.text = text.clone();
        }
    }

    for mut material in panels.iter() {
        material.color = panel_color(is_grounded).to_linear();
    }
}
