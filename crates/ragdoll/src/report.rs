use std::fmt::Write;

use concerto_animation::{
    graph::{AnimationGraph, NodeVisit},
    node::{AnimationClipNodeState, state_machine::AnimationStateMachineState},
    player::AnimationPlayer,
};
use concerto_foundation::assets::asset_store::AssetStore;
use concerto_name::Name;

use crate::{
    attributes::{MotorCommand, PhysicsAmount},
    character::RagdollBody,
    sensing::{BodySense, describe_body},
};

struct Machine {
    depth: usize,
    playing: Vec<Name>,
}

fn for_each_playing_node(
    player: &AnimationPlayer,
    graphs: &AssetStore<AnimationGraph>,
    mut f: impl FnMut(&NodeVisit<'_>),
) {
    let mut machines: Vec<Machine> = Vec::new();
    player.for_each_node(graphs, |visit| {
        let depth = visit.path.len();
        machines.retain(|machine| machine.depth < depth);
        let playing = visit.path[..depth - 1]
            .iter()
            .enumerate()
            .all(|(index, step)| {
                step.state.is_none_or(|state| {
                    machines
                        .iter()
                        .find(|machine| machine.depth == index + 1)
                        .is_none_or(|machine| machine.playing.contains(&state))
                })
            });
        if let Some(machine) = visit.state.downcast_ref::<AnimationStateMachineState>() {
            let mut names: Vec<Name> = machine.current_state_name().into_iter().collect();
            names.extend(machine.transition().map(|transition| transition.from));
            machines.push(Machine {
                depth,
                playing: names,
            });
        }
        if playing {
            f(visit);
        }
    });
}

/// The names of the states active in `player`'s graphs, outermost first.
pub fn active_states(player: &AnimationPlayer, graphs: &AssetStore<AnimationGraph>) -> Vec<String> {
    let mut states = Vec::new();
    for_each_playing_node(player, graphs, |visit| {
        if let Some(current) = visit
            .state
            .downcast_ref::<AnimationStateMachineState>()
            .and_then(AnimationStateMachineState::current_state_name)
        {
            states.push(current.to_string());
        }
    });
    states
}

fn descriptions(player: &AnimationPlayer, graphs: &AssetStore<AnimationGraph>) -> Vec<String> {
    let mut descriptions = Vec::new();
    for_each_playing_node(player, graphs, |visit| {
        let shown_elsewhere = visit.state.is::<AnimationStateMachineState>()
            || visit.state.is::<AnimationClipNodeState>();
        if !shown_elsewhere && let Some(described) = visit.node.describe(visit.state) {
            descriptions.push(described);
        }
    });
    descriptions
}

/// What a character's body is doing, as `body_report` prints it: each limb's physics amount and cover, the active graph states and each playing node's description when `graphs` is given, and what the body senses.
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
        for described in descriptions(player, graphs) {
            let _ = writeln!(text, "node: {described}");
        }
    }
    text.push_str(&describe_body(sense, body));
    text
}
