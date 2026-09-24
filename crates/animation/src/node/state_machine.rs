use std::any::Any;
use std::collections::HashMap;

use crate::graph::{AnimationGraph, AnimationGraphInstance, AnimationGraphInstances, GraphId};
use crate::pose::{EvaluatedPose, Pose, PosePool};
use crate::transition::{AnimationTransitionBlender, blend_stack::BlendStack};
use crate::{
    evaluation::AnimationGraphContext,
    node::{AnimationNodeInstance, AnimationNodeKind},
};

use concerto_core::{assets::handle::AssetHandle, utils::AsAny};
use derive_more::Deref;
use uuid::Uuid;

pub struct AnimationFSMStateDefinition<'a> {
    pub name: &'a str,
    pub graph: AssetHandle<AnimationGraph>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationFSMState {
    name: String,
    graph: AssetHandle<AnimationGraph>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AnimationFSMTrigger {
    Instant,
    OnAnimationEnd,
    BoolEquals { param: String, value: bool },
    Vec2NonZero { param: String },
}

impl AnimationFSMTrigger {
    pub fn on_bool(param_name: impl Into<String>, cond: bool) -> Self {
        Self::BoolEquals {
            param: param_name.into(),
            value: cond,
        }
    }

    pub fn on_non_zero_vec(param_name: impl Into<String>) -> Self {
        Self::Vec2NonZero {
            param: param_name.into(),
        }
    }
}

pub struct AnimationStateMachineTransitionDefinition<'a> {
    pub target_state: &'a str,
    pub trigger: AnimationFSMTrigger,
    pub transition_time: f32,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationStateMachineTransition {
    next_state: StateId,
    trigger: AnimationFSMTrigger,
    transition_time: f32,
}

#[derive(Clone, Copy, Deref, serde::Serialize, serde::Deserialize)]
pub struct StateId(usize);

impl From<usize> for StateId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl StateId {
    fn as_graph_id(&self) -> GraphId {
        (**self).into()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct AnimationStateMachine {
    initial_state: StateId,
    states: Vec<AnimationFSMState>,
    transitions: Vec<Vec<AnimationStateMachineTransition>>,
}

impl AnimationStateMachine {
    pub fn new(
        initial_state: &str,
        states_definition: Vec<AnimationFSMStateDefinition>,
        transitions_definition: HashMap<&str, Vec<AnimationStateMachineTransitionDefinition>>,
    ) -> Self {
        let mut name_to_index: HashMap<&str, StateId> = HashMap::new();
        let mut transitions = Vec::new();
        let states = states_definition
            .into_iter()
            .enumerate()
            .map(|(index, state_def)| {
                name_to_index.insert(state_def.name, index.into());
                transitions.push(Vec::new());
                AnimationFSMState {
                    name: state_def.name.to_string(),
                    graph: state_def.graph,
                }
            })
            .collect();

        transitions_definition
            .into_iter()
            .for_each(|(from, transition_defs)| {
                let Some(from_index) = name_to_index.get(from) else {
                    return;
                };

                transition_defs.into_iter().for_each(|transition_def| {
                    transitions[**from_index].push(AnimationStateMachineTransition {
                        next_state: *name_to_index
                            .get(transition_def.target_state)
                            .unwrap_or(&0.into()),
                        trigger: transition_def.trigger,
                        transition_time: transition_def.transition_time,
                    });
                });
            });

        Self {
            initial_state: *name_to_index.get(initial_state).unwrap_or(&0.into()),
            states,
            transitions,
        }
    }

    pub fn from_initial_state(
        state_name: &str,
        state_graph: AssetHandle<AnimationGraph>,
        f: impl FnOnce(&mut TransitionBuilder),
    ) -> AnimationStateMachineBuilder {
        let mut builder = AnimationStateMachineBuilder {
            initial_state: state_name.to_string(),
            states: vec![FSMStateEntry {
                name: state_name.to_string(),
                graph: state_graph,
            }],
            transitions: Vec::new(),
        };
        let mut tb = TransitionBuilder {
            builder: &mut builder,
            from: state_name.to_string(),
        };
        f(&mut tb);
        builder
    }

    pub(crate) fn get_state_transitions(
        &self,
        state_index: StateId,
    ) -> Option<&Vec<AnimationStateMachineTransition>> {
        self.transitions.get(*state_index)
    }

    /// Every transition trigger, flattened across states — lets callers inspect
    /// trigger payloads without exposing the private transition table.
    pub fn triggers(&self) -> impl Iterator<Item = &AnimationFSMTrigger> + '_ {
        self.transitions
            .iter()
            .flatten()
            .map(|transition| &transition.trigger)
    }
}

impl AnimationStateMachine {
    pub(crate) fn create_instance_boxed(
        &self,
        creation_context: &AnimationGraphContext,
    ) -> Box<dyn AnimationNodeInstance> {
        let mut instanced_internal_graphs = Vec::new();
        let mut state_names = Vec::new();
        for fsm_state in &self.states {
            let mut instanced_internal_graph = AnimationGraphInstance::default();
            instanced_internal_graph.initialize(fsm_state.graph.clone(), creation_context);
            instanced_internal_graphs.push(instanced_internal_graph);
            state_names.push(fsm_state.name.clone());
        }

        Box::new(AnimationStateMachineInstance::new(
            self.initial_state,
            instanced_internal_graphs,
            state_names,
        ))
    }
}

#[derive(AsAny)]
pub(crate) struct AnimationStateMachineInstance {
    state_graph_instances: AnimationGraphInstances,
    current_state: StateId,
    state_names: Vec<String>,
    blend_stack: BlendStack,
}

impl AnimationStateMachineInstance {
    pub(crate) fn new(
        initial_state: StateId,
        graph_instance: Vec<AnimationGraphInstance>,
        state_names: Vec<String>,
    ) -> Self {
        Self {
            state_graph_instances: AnimationGraphInstances::new(graph_instance),
            current_state: initial_state,
            state_names,
            blend_stack: BlendStack::new(initial_state.as_graph_id()),
        }
    }

    pub(crate) fn current_state_name(&self) -> &str {
        self.state_names
            .get(*self.current_state)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn transition(
        &mut self,
        context: &AnimationGraphContext<'_>,
        transition: &AnimationStateMachineTransition,
    ) {
        self.current_state = transition.next_state;
        self.blend_stack.transition(
            transition.next_state.as_graph_id(),
            &mut self.state_graph_instances,
            transition.transition_time,
            context,
        );

        // The blend stack's newest entry and the FSM's current state are the same fact; they
        // must never diverge, or transition logic would run against a state that isn't playing.
        debug_assert_eq!(
            *self.blend_stack.current(),
            *self.current_state.as_graph_id(),
            "blend stack current graph drifted from the FSM current state",
        );
    }
}

impl AnimationNodeInstance for AnimationStateMachineInstance {
    fn reset(&mut self) {}

    fn update(
        &mut self,
        node: &AnimationNodeKind,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        let AnimationNodeKind::StateMachine(fsm) = node else {
            return;
        };

        let Some(transitions) = fsm.get_state_transitions(self.current_state) else {
            return;
        };

        for transition in transitions {
            match &transition.trigger {
                AnimationFSMTrigger::Instant => {
                    self.transition(context, transition);
                    break;
                }
                AnimationFSMTrigger::BoolEquals { param, value } => {
                    if context
                        .blackboard()
                        .get_bool(param)
                        .is_some_and(|v| v == *value)
                    {
                        self.transition(context, transition);
                        break;
                    }
                }
                AnimationFSMTrigger::Vec2NonZero { param } => {
                    if context
                        .blackboard()
                        .get_vec2(param)
                        .is_some_and(|v| v.length_squared() > f32::EPSILON)
                    {
                        self.transition(context, transition);
                        break;
                    }
                }
                AnimationFSMTrigger::OnAnimationEnd => {
                    if self
                        .state_graph_instances
                        .get(self.current_state.as_graph_id())
                        .is_some_and(|current_graph| current_graph.is_finished())
                    {
                        self.transition(context, transition);
                        break;
                    }
                }
            }
        }

        self.blend_stack
            .update(delta_time, &mut self.state_graph_instances, context);
    }

    fn evaluate(
        &self,
        _node: &AnimationNodeKind,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedPose],
        pool: &mut PosePool,
        output: &mut Pose,
    ) {
        self.blend_stack
            .sample(bone_ids, &self.state_graph_instances, context, pool, output);
    }
}

struct FSMStateEntry {
    name: String,
    graph: AssetHandle<AnimationGraph>,
}

struct FSMTransitionEntry {
    from: String,
    to: String,
    trigger: AnimationFSMTrigger,
    transition_time: f32,
}

pub struct AnimationStateMachineBuilder {
    initial_state: String,
    states: Vec<FSMStateEntry>,
    transitions: Vec<FSMTransitionEntry>,
}

impl AnimationStateMachineBuilder {
    pub fn state(
        mut self,
        state_name: &str,
        state_graph: AssetHandle<AnimationGraph>,
        f: impl FnOnce(&mut TransitionBuilder),
    ) -> Self {
        self.states.push(FSMStateEntry {
            name: state_name.to_string(),
            graph: state_graph,
        });
        let mut tb = TransitionBuilder {
            builder: &mut self,
            from: state_name.to_string(),
        };
        f(&mut tb);
        self
    }

    pub fn build(self) -> AnimationStateMachine {
        let mut name_to_index: HashMap<String, StateId> = HashMap::new();
        let mut states = Vec::new();
        let mut transitions: Vec<Vec<AnimationStateMachineTransition>> = Vec::new();

        for (index, entry) in self.states.into_iter().enumerate() {
            states.push(AnimationFSMState {
                name: entry.name.clone(),
                graph: entry.graph,
            });
            name_to_index.insert(entry.name, index.into());
            transitions.push(Vec::new());
        }

        for entry in self.transitions {
            let Some(&from_index) = name_to_index.get(&entry.from) else {
                continue;
            };
            let target_index = name_to_index.get(&entry.to).copied().unwrap_or(0.into());
            transitions[*from_index].push(AnimationStateMachineTransition {
                next_state: target_index,
                trigger: entry.trigger,
                transition_time: entry.transition_time,
            });
        }

        let initial_state = name_to_index
            .get(&self.initial_state)
            .copied()
            .unwrap_or(0.into());

        AnimationStateMachine {
            initial_state,
            states,
            transitions,
        }
    }
}

pub struct TransitionBuilder<'a> {
    builder: &'a mut AnimationStateMachineBuilder,
    from: String,
}

impl TransitionBuilder<'_> {
    pub fn to(
        &mut self,
        state_name: &str,
        trigger: AnimationFSMTrigger,
        transition_time: f32,
    ) -> &mut Self {
        self.builder.transitions.push(FSMTransitionEntry {
            from: self.from.clone(),
            to: state_name.to_string(),
            trigger,
            transition_time,
        });
        self
    }
}
