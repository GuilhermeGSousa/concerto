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
    sensing::BodySense,
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

/// The names of the current states of the state machines playing in `player`'s graph, outermost first.
///
/// A machine inside a state that is only fading out still counts; one inside a
/// state that is not playing does not.
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

impl RagdollBody {
    /// What the body is doing, as text an agent can read: the editor's `body_report` tool answers with it, and a game can print it.
    ///
    /// One line per limb, `bone · physics amount · cover`: how much of the bone
    /// the last drive pass of `player`'s graph draws from physics, and how much
    /// of it the graph's nodes hold with a [`MotorCommand`]. With `graphs`, then the states of the
    /// graph's playing state machines and a `node:` line for each playing node
    /// that describes itself, such as a behaviour node. Last, what `sense` says,
    /// or that the character has no [`BodySense`].
    pub fn report(
        &self,
        player: &AnimationPlayer,
        graphs: Option<&AssetStore<AnimationGraph>>,
        sense: Option<&BodySense>,
    ) -> String {
        let mut text = String::new();
        for (part, bone) in self.resolved().bones().iter().enumerate() {
            let name = self
                .bone_name(part)
                .map_or_else(|| format!("part {part}"), |name| name.to_string());
            let driven = player.drive_attributes();
            let amount = driven.get::<PhysicsAmount>(*bone).0;
            let cover = driven.get::<MotorCommand>(*bone).cover;
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
        match sense {
            None => text.push_str("sensing: the character has no BodySense\n"),
            Some(sense) => text.push_str(&sense.describe(self)),
        }
        text
    }
}
