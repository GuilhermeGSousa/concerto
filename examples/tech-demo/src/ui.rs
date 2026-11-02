use concerto::{
    animation::{graph::AnimationGraph, player::AnimationPlayer},
    color::Color,
    ecs::{CommandQueue, Component, Query, Res, With},
    foundation::assets::asset_store::AssetStore,
    motion_matching::{CharacterDisplacement, CharacterLink, MotionDatabase, MotionMatchingNode},
    physics::ground::{GroundProbe, GroundState},
    ui::{
        material::UIMaterial,
        node::{UINode, UIRect},
        text::{FontFamily, UIText},
        transform::UIValue,
    },
};

use crate::character::Player;

const PANEL_WIDTH: f32 = 230.0;
const PANEL_PADDING: UIRect = UIRect {
    top: 6.0,
    right: 10.0,
    bottom: 6.0,
    left: 10.0,
};
const LINE_HEIGHT: f32 = 16.0;
const LINES: usize = 8;
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
            format!("Synchronization {}", sync.data_factor)
        }
    }
}

fn readout(
    is_grounded: bool,
    state: &str,
    matching: Option<&Matching>,
    method: &CharacterDisplacement,
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
    format!(
        "grounded: {is_grounded}\n\
         state:    {state}\n\
         entry:    {entry}\n\
         frame:    {frame}\n\
         cost:     {cost}\n\
         WASD/arrows move, Shift run\n\
         1-4 toggle gizmo layers\n\
         M method: {method}"
    )
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

pub(crate) fn update_grounded_overlay(
    probes: Query<&GroundProbe, With<Player>>,
    characters: Query<(&CharacterDisplacement, Option<&CharacterLink>)>,
    players: Query<&mut AnimationPlayer>,
    graphs: Res<AssetStore<AnimationGraph>>,
    databases: Res<AssetStore<MotionDatabase>>,
    texts: Query<&mut UIText, With<GroundedText>>,
    panels: Query<&mut UIMaterial, With<GroundedPanel>>,
) {
    let Some(probe) = probes.iter().next() else {
        return;
    };
    let Some((method, link)) = characters.iter().next() else {
        return;
    };

    let is_grounded = probe.is_grounded();
    let state = match probe.ground() {
        GroundState::InAir => "InAir",
        GroundState::OnGround(_) => "OnGround",
        GroundState::OnSteepGround(_) => "OnSteepGround",
    };

    let mut matching = None;
    if let Some(mut player) = link.and_then(|link| players.get_entity(link.skeleton)) {
        player.for_each_instance_mut::<MotionMatchingNode>(&graphs, |node, instance, _| {
            let Some(last) = &instance.last_query else {
                return;
            };
            if matching.is_some() {
                return;
            }
            matching = Some(Matching {
                entry: databases
                    .get(&node.database)
                    .and_then(|database| database.entry_of(instance.frame)),
                frame: instance.frame,
                cost: last.best_cost,
            });
        });
    }

    let text = readout(is_grounded, state, matching.as_ref(), method);
    for mut ui_text in texts.iter() {
        if ui_text.text != text {
            ui_text.text = text.clone();
        }
    }

    for mut material in panels.iter() {
        material.color = panel_color(is_grounded).to_linear();
    }
}

#[cfg(test)]
mod tests {
    use concerto::motion_matching::{Adjustment, CharacterDisplacement, Clamp, Synchronization};

    use super::{Matching, readout};

    #[test]
    fn the_readout_shows_the_entry_frame_and_cost_of_the_last_search() {
        let matching = Matching {
            entry: Some(3),
            frame: 120,
            cost: Some(0.5),
        };

        assert_eq!(
            readout(
                true,
                "OnGround",
                Some(&matching),
                &CharacterDisplacement::default()
            ),
            "grounded: true\n\
             state:    OnGround\n\
             entry:    3\n\
             frame:    120\n\
             cost:     0.500\n\
             WASD/arrows move, Shift run\n\
             1-4 toggle gizmo layers\n\
             M method: Adjustment"
        );
    }

    #[test]
    fn the_readout_shows_dashes_before_any_search() {
        let text = readout(false, "InAir", None, &CharacterDisplacement::default());

        assert!(text.contains("entry:    --\nframe:    --\ncost:     --\n"));
    }

    #[test]
    fn a_nan_cost_and_a_frame_outside_every_entry_read_as_dashes() {
        let matching = Matching {
            entry: None,
            frame: 7,
            cost: Some(f32::NAN),
        };
        let text = readout(
            true,
            "OnGround",
            Some(&matching),
            &CharacterDisplacement::default(),
        );

        assert!(text.contains("entry:    --\nframe:    7\ncost:     --\n"));
    }

    #[test]
    fn the_help_line_names_the_current_method() {
        let methods = [
            CharacterDisplacement::Adjustment(Adjustment::default()),
            CharacterDisplacement::Clamp(Clamp::default()),
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.0 }),
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 1.0 }),
        ];
        let lines: Vec<String> = methods
            .iter()
            .map(|method| {
                readout(true, "OnGround", None, method)
                    .lines()
                    .last()
                    .unwrap()
                    .to_string()
            })
            .collect();

        assert_eq!(
            lines,
            [
                "M method: Adjustment",
                "M method: Clamp",
                "M method: Synchronization 0",
                "M method: Synchronization 1",
            ]
        );
    }
}
