use std::fmt::Write;

use concerto_animation::{graph::AnimationGraph, player::AnimationPlayer};
use concerto_foundation::assets::asset_store::AssetStore;

use crate::{
    attributes::{MotorCommand, PhysicsAmount},
    character::RagdollBody,
    sensing::{BodySense, describe_body},
};

/// The names of the states active in `player`'s graphs, outermost first.
pub fn active_states(player: &AnimationPlayer, graphs: &AssetStore<AnimationGraph>) -> Vec<String> {
    let mut states = Vec::new();
    player.for_each_node(graphs, |visit| {
        if let Some(current) = visit
            .report
            .state_machine
            .as_ref()
            .and_then(|machine| machine.current)
        {
            states.push(current.to_string());
        }
    });
    states
}

fn behaviours(player: &AnimationPlayer, graphs: &AssetStore<AnimationGraph>) -> Vec<String> {
    let mut behaviours = Vec::new();
    player.for_each_node(graphs, |visit| {
        if let Some(behaviour) = &visit.report.behaviour {
            behaviours.push(behaviour.describe());
        }
    });
    behaviours
}

/// What a character's body is doing, as `body_report` prints it: each limb's physics amount and cover, the active graph states and behaviours when `graphs` is given, and what the body senses.
pub fn report_body(
    body: &RagdollBody,
    player: &AnimationPlayer,
    graphs: Option<&AssetStore<AnimationGraph>>,
    sense: Option<&BodySense>,
) -> String {
    let mut text = String::new();
    for (part, bone) in body.resolved().bones().iter().enumerate() {
        let name = body
            .bone_name(part)
            .map_or_else(|| format!("part {part}"), |name| name.to_string());
        let amount = player.attributes().get::<PhysicsAmount>(*bone).0;
        let cover = player.drive_attributes().get::<MotorCommand>(*bone).cover;
        let _ = writeln!(text, "{name} · physics {amount:.2} · cover {cover:.2}");
    }
    if let Some(graphs) = graphs {
        let states = active_states(player, graphs);
        let _ = writeln!(
            text,
            "graph states: {}",
            if states.is_empty() {
                "none".to_owned()
            } else {
                states.join(", ")
            }
        );
        for behaviour in behaviours(player, graphs) {
            let _ = writeln!(text, "behaviour: {behaviour}");
        }
    }
    text.push_str(&describe_body(sense, body));
    text
}
