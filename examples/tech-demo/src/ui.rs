use concerto::{
    color::Color,
    ecs::{CommandQueue, Component, Query, With},
    physics::ground::{GroundProbe, GroundState},
    ui::{
        material::UIMaterial,
        node::{UINode, UIRect},
        text::{FontFamily, TextComponent},
        transform::UIValue,
    },
};

use crate::character::Player;

/// Marker for the readout's panel, whose colour tracks the grounded state.
#[derive(Component)]
pub(crate) struct GroundedPanel;

/// Marker for the readout's text node.
#[derive(Component)]
pub(crate) struct GroundedText;

fn panel_color(is_grounded: bool) -> Color {
    match is_grounded {
        true => Color::rgba(0.0, 0.35, 0.1, 0.6),
        false => Color::rgba(0.45, 0.06, 0.06, 0.6),
    }
}

pub(crate) fn spawn_grounded_overlay(mut cmd: CommandQueue) {
    // Every UI root lays out from the window origin and a root's own margin does not move it,
    // so this transparent spacer (no UIMaterial, hence never drawn) pads the panel down clear
    // of the frame-stats overlay above it.
    cmd.spawn((UINode {
        width: UIValue::Px(230.0),
        height: UIValue::Px(100.0),
        padding: UIRect {
            top: 54.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
        ..Default::default()
    },))
        .add_child_with(
            (
                UINode {
                    width: UIValue::Px(230.0),
                    height: UIValue::Px(46.0),
                    padding: UIRect::axes(6.0, 10.0),
                    ..Default::default()
                },
                UIMaterial::flat(panel_color(false)),
                GroundedPanel,
            ),
            |panel| {
                panel.add_child((
                    UINode {
                        flex_grow: 1.0,
                        ..Default::default()
                    },
                    TextComponent {
                        text: "grounded: --".to_string(),
                        font_size: 12.0,
                        line_height: 16.0,
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
    texts: Query<&mut TextComponent, With<GroundedText>>,
    panels: Query<&mut UIMaterial, With<GroundedPanel>>,
) {
    let Some(probe) = probes.iter().next() else {
        return;
    };

    let is_grounded = probe.is_grounded();
    let state = match probe.ground() {
        GroundState::InAir => "InAir",
        GroundState::OnGround(_) => "OnGround",
        GroundState::OnSteepGround(_) => "OnSteepGround",
    };

    for mut text in texts.iter() {
        text.text = format!("grounded: {is_grounded}\nstate:    {state}");
    }

    for mut material in panels.iter() {
        material.color = panel_color(is_grounded).to_linear();
    }
}
