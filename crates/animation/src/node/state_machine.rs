use std::collections::HashMap;
use std::fmt;

use crate::graph::{AnimationGraph, AnimationGraphState, AnimationGraphStates, GraphId};
use crate::param::Param;
use crate::pose::{AnimationFrame, EvaluatedFrame, FramePool};
use crate::transition::{
    AnimationTransitionBlender, TransitionBlender, blend_stack::BlendStack,
    inertialization_blender::InertializationBlender,
};
use crate::{
    evaluation::AnimationGraphContext,
    node::{
        AnimationNode, InputSlots, NodeReport, ReflectAnimationNode, StateMachineReport,
        TransitionReport,
    },
};

use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::Name;
use concerto_reflect::Reflect;
use derive_more::Deref;
use glam::Vec2;
use uuid::Uuid;

pub struct AnimationFSMStateDefinition {
    pub name: Name,
    pub graph: AssetHandle<AnimationGraph>,
}

#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationFSMState {
    name: Name,
    graph: AssetHandle<AnimationGraph>,
}

#[derive(Debug, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub enum AnimationFSMTrigger {
    #[default]
    Instant,
    OnAnimationEnd,
    BoolEquals {
        param: Param<bool>,
        value: bool,
    },
    Vec2NonZero {
        param: Param<Vec2>,
    },
}

impl AnimationFSMTrigger {
    pub fn on_bool(param: impl Into<Param<bool>>, cond: bool) -> Self {
        Self::BoolEquals {
            param: param.into(),
            value: cond,
        }
    }

    pub fn on_non_zero_vec(param: impl Into<Param<Vec2>>) -> Self {
        Self::Vec2NonZero {
            param: param.into(),
        }
    }
}

pub struct AnimationStateMachineTransitionDefinition {
    pub target_state: Name,
    pub trigger: AnimationFSMTrigger,
    pub blend_time: f32,
}

#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationStateMachineTransition {
    next_state: StateId,
    trigger: AnimationFSMTrigger,
    blend_time: f32,
}

#[derive(Clone, Copy, Default, Deref, Reflect, serde::Serialize, serde::Deserialize)]
pub struct StateId(usize);

/// One state of an [`AnimationStateMachine`], as its read access shows it.
pub struct AnimationFSMStateView<'a> {
    pub name: Name,
    pub graph: &'a AssetHandle<AnimationGraph>,
}

/// One transition of an [`AnimationStateMachine`]; `index` is its place among its source state's transitions.
pub struct AnimationFSMTransitionView<'a> {
    pub index: usize,
    pub target: Name,
    pub trigger: &'a AnimationFSMTrigger,
    pub blend_time: f32,
}

/// Why an edit of an [`AnimationStateMachine`] was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateMachineEditError {
    UnknownState { name: Name },
    DuplicateState { name: Name },
    TransitionOutOfRange { from: Name, index: usize },
}

impl fmt::Display for StateMachineEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateMachineEditError::UnknownState { name } => {
                write!(f, "the state machine has no state named {name}")
            }
            StateMachineEditError::DuplicateState { name } => {
                write!(f, "the state machine already has a state named {name}")
            }
            StateMachineEditError::TransitionOutOfRange { from, index } => {
                write!(f, "state {from} has no transition at index {index}")
            }
        }
    }
}

impl std::error::Error for StateMachineEditError {}

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

#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct AnimationStateMachine {
    initial_state: StateId,
    states: Vec<AnimationFSMState>,
    transitions: Vec<Vec<AnimationStateMachineTransition>>,
    #[serde(default)]
    blender: TransitionBlender,
}

impl AnimationStateMachine {
    pub fn new(
        initial_state: Name,
        states_definition: Vec<AnimationFSMStateDefinition>,
        transitions_definition: HashMap<Name, Vec<AnimationStateMachineTransitionDefinition>>,
    ) -> Self {
        let mut name_to_index: HashMap<Name, StateId> = HashMap::new();
        let mut transitions = Vec::new();
        let states = states_definition
            .into_iter()
            .enumerate()
            .map(|(index, state_def)| {
                name_to_index.insert(state_def.name, index.into());
                transitions.push(Vec::new());
                AnimationFSMState {
                    name: state_def.name,
                    graph: state_def.graph,
                }
            })
            .collect();

        transitions_definition
            .into_iter()
            .for_each(|(from, transition_defs)| {
                let Some(from_index) = name_to_index.get(&from) else {
                    return;
                };

                transition_defs.into_iter().for_each(|transition_def| {
                    transitions[**from_index].push(AnimationStateMachineTransition {
                        next_state: *name_to_index
                            .get(&transition_def.target_state)
                            .unwrap_or(&0.into()),
                        trigger: transition_def.trigger,
                        blend_time: transition_def.blend_time,
                    });
                });
            });

        Self {
            initial_state: *name_to_index.get(&initial_state).unwrap_or(&0.into()),
            states,
            transitions,
            blender: TransitionBlender::default(),
        }
    }

    pub fn from_initial_state(
        state_name: Name,
        state_graph: AssetHandle<AnimationGraph>,
        f: impl FnOnce(&mut TransitionBuilder),
    ) -> AnimationStateMachineBuilder {
        let mut builder = AnimationStateMachineBuilder {
            initial_state: state_name,
            states: vec![FSMStateEntry {
                name: state_name,
                graph: state_graph,
            }],
            transitions: Vec::new(),
            blender: TransitionBlender::default(),
        };
        let mut tb = TransitionBuilder {
            builder: &mut builder,
            from: state_name,
        };
        f(&mut tb);
        builder
    }

    /// The states in order.
    pub fn states(&self) -> impl Iterator<Item = AnimationFSMStateView<'_>> + '_ {
        self.states.iter().map(|state| AnimationFSMStateView {
            name: state.name,
            graph: &state.graph,
        })
    }

    /// The state the machine starts in; none for a machine without states.
    pub fn initial_state(&self) -> Option<Name> {
        self.states.get(*self.initial_state).map(|state| state.name)
    }

    /// The transitions out of `from`, in the order the machine tries them.
    pub fn transitions(
        &self,
        from: Name,
    ) -> Result<impl Iterator<Item = AnimationFSMTransitionView<'_>> + '_, StateMachineEditError>
    {
        let from = self.state_index(from)?;
        Ok(self
            .transitions
            .get(from)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, transition)| {
                Some(AnimationFSMTransitionView {
                    index,
                    target: self.states.get(*transition.next_state)?.name,
                    trigger: &transition.trigger,
                    blend_time: transition.blend_time,
                })
            }))
    }

    /// Adds a state after the others; the first state added is the initial one.
    pub fn add_state(
        &mut self,
        name: Name,
        graph: AssetHandle<AnimationGraph>,
    ) -> Result<(), StateMachineEditError> {
        if self.state_index(name).is_ok() {
            return Err(StateMachineEditError::DuplicateState { name });
        }
        self.states.push(AnimationFSMState { name, graph });
        self.transitions.push(Vec::new());
        Ok(())
    }

    /// Removes a state with every transition to or from it; a removed initial state hands over to the first state.
    pub fn remove_state(&mut self, name: Name) -> Result<(), StateMachineEditError> {
        let removed = self.state_index(name)?;
        self.states.remove(removed);
        if removed < self.transitions.len() {
            self.transitions.remove(removed);
        }
        let renumbered = |state: StateId| StateId(*state - usize::from(*state > removed));
        for transitions in &mut self.transitions {
            transitions.retain(|transition| *transition.next_state != removed);
            for transition in transitions {
                transition.next_state = renumbered(transition.next_state);
            }
        }
        self.initial_state = if *self.initial_state == removed {
            StateId(0)
        } else {
            renumbered(self.initial_state)
        };
        Ok(())
    }

    /// Renames a state; its transitions follow it.
    pub fn rename_state(&mut self, name: Name, to: Name) -> Result<(), StateMachineEditError> {
        let index = self.state_index(name)?;
        if to != name && self.state_index(to).is_ok() {
            return Err(StateMachineEditError::DuplicateState { name: to });
        }
        self.states[index].name = to;
        Ok(())
    }

    /// Replaces the graph a state plays.
    pub fn set_state_graph(
        &mut self,
        name: Name,
        graph: AssetHandle<AnimationGraph>,
    ) -> Result<(), StateMachineEditError> {
        let index = self.state_index(name)?;
        self.states[index].graph = graph;
        Ok(())
    }

    /// Adds a transition after the others out of `from` and returns its index among them.
    pub fn add_transition(
        &mut self,
        from: Name,
        to: Name,
        trigger: AnimationFSMTrigger,
        blend_time: f32,
    ) -> Result<usize, StateMachineEditError> {
        let from = self.state_index(from)?;
        let next_state = StateId(self.state_index(to)?);
        if self.transitions.len() <= from {
            self.transitions.resize_with(from + 1, Vec::new);
        }
        let transitions = &mut self.transitions[from];
        transitions.push(AnimationStateMachineTransition {
            next_state,
            trigger,
            blend_time,
        });
        Ok(transitions.len() - 1)
    }

    /// Removes the transition at `index` among those out of `from`.
    pub fn remove_transition(
        &mut self,
        from: Name,
        index: usize,
    ) -> Result<(), StateMachineEditError> {
        let state = self.state_index(from)?;
        match self.transitions.get_mut(state) {
            Some(transitions) if index < transitions.len() => {
                transitions.remove(index);
                Ok(())
            }
            _ => Err(StateMachineEditError::TransitionOutOfRange { from, index }),
        }
    }

    /// Replaces the trigger and blend time of the transition at `index` among those out of `from`.
    pub fn set_transition(
        &mut self,
        from: Name,
        index: usize,
        trigger: AnimationFSMTrigger,
        blend_time: f32,
    ) -> Result<(), StateMachineEditError> {
        let state = self.state_index(from)?;
        let transition = self
            .transitions
            .get_mut(state)
            .and_then(|transitions| transitions.get_mut(index))
            .ok_or(StateMachineEditError::TransitionOutOfRange { from, index })?;
        transition.trigger = trigger;
        transition.blend_time = blend_time;
        Ok(())
    }

    /// Makes a state the one the machine starts in.
    pub fn set_initial(&mut self, name: Name) -> Result<(), StateMachineEditError> {
        self.initial_state = StateId(self.state_index(name)?);
        Ok(())
    }

    fn state_index(&self, name: Name) -> Result<usize, StateMachineEditError> {
        self.states
            .iter()
            .position(|state| state.name == name)
            .ok_or(StateMachineEditError::UnknownState { name })
    }

    pub(crate) fn state_graphs(&self) -> impl Iterator<Item = &AssetHandle<AnimationGraph>> {
        self.states.iter().map(|state| &state.graph)
    }

    /// The blending backend every transition of this machine uses.
    pub fn blender(&self) -> TransitionBlender {
        self.blender
    }

    /// Every transition's blend time, flattened across states.
    pub fn blend_times(&self) -> impl Iterator<Item = f32> + '_ {
        self.transitions
            .iter()
            .flatten()
            .map(|transition| transition.blend_time)
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

pub struct AnimationStateMachineState {
    state_graphs: AnimationGraphStates,
    current_state: StateId,
    state_names: Vec<Name>,
    blender: Box<dyn AnimationTransitionBlender>,
}

impl AnimationStateMachineState {
    pub(crate) fn new(
        initial_state: StateId,
        state_graphs: Vec<AnimationGraphState>,
        state_names: Vec<Name>,
        blender: TransitionBlender,
    ) -> Self {
        let initial_graph = initial_state.as_graph_id();
        let blender: Box<dyn AnimationTransitionBlender> = match blender {
            TransitionBlender::CrossFade => Box::new(BlendStack::new(initial_graph)),
            TransitionBlender::Inertialization => {
                Box::new(InertializationBlender::new(initial_graph))
            }
        };
        Self {
            state_graphs: AnimationGraphStates::new(state_graphs),
            current_state: initial_state,
            state_names,
            blender,
        }
    }

    pub fn current_state_name(&self) -> Option<Name> {
        self.state_names.get(*self.current_state).copied()
    }

    fn transition(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        transition: &AnimationStateMachineTransition,
    ) {
        self.current_state = transition.next_state;
        self.blender.transition(
            transition.next_state.as_graph_id(),
            &mut self.state_graphs,
            transition.blend_time,
            graphs,
        );

        // The blend stack's newest entry and the FSM's current state are the same fact; they
        // must never diverge, or transition logic would run against a state that isn't playing.
        debug_assert_eq!(
            *self.blender.current(),
            *self.current_state.as_graph_id(),
            "blend stack current graph drifted from the FSM current state",
        );
    }
}

impl AnimationNode for AnimationStateMachine {
    type State = AnimationStateMachineState;
    type CreateInput = Res<'static, AssetStore<AnimationGraph>>;
    type UpdateInput = Res<'static, AssetStore<AnimationGraph>>;
    type EvaluateInput = Res<'static, AssetStore<AnimationGraph>>;

    fn input_slots(&self) -> InputSlots {
        InputSlots::None
    }

    fn create_state(
        &self,
        graphs: &Res<AssetStore<AnimationGraph>>,
        context: &AnimationGraphContext<'_>,
    ) -> AnimationStateMachineState {
        let mut state_graphs = Vec::new();
        let mut state_names = Vec::new();
        for fsm_state in &self.states {
            let mut state_graph = AnimationGraphState::default();
            state_graph.initialize(fsm_state.graph.clone(), graphs, context);
            state_graphs.push(state_graph);
            state_names.push(fsm_state.name);
        }

        AnimationStateMachineState::new(self.initial_state, state_graphs, state_names, self.blender)
    }

    fn update(
        &self,
        state: &mut AnimationStateMachineState,
        delta_time: f32,
        graphs: &Res<AssetStore<AnimationGraph>>,
        context: &AnimationGraphContext<'_>,
    ) {
        let Some(transitions) = self.get_state_transitions(state.current_state) else {
            return;
        };

        for transition in transitions {
            match &transition.trigger {
                AnimationFSMTrigger::Instant => {
                    state.transition(graphs, transition);
                    break;
                }
                AnimationFSMTrigger::BoolEquals { param, value } => {
                    if context.blackboard().read(param).is_some_and(|v| v == value) {
                        state.transition(graphs, transition);
                        break;
                    }
                }
                AnimationFSMTrigger::Vec2NonZero { param } => {
                    if context
                        .blackboard()
                        .read(param)
                        .is_some_and(|v| v.length_squared() > f32::EPSILON)
                    {
                        state.transition(graphs, transition);
                        break;
                    }
                }
                AnimationFSMTrigger::OnAnimationEnd => {
                    if state
                        .state_graphs
                        .get(state.current_state.as_graph_id())
                        .is_some_and(|current_graph| current_graph.is_finished(graphs))
                    {
                        state.transition(graphs, transition);
                        break;
                    }
                }
            }
        }

        state
            .blender
            .update(delta_time, &mut state.state_graphs, graphs, context);
    }

    fn evaluate(
        &self,
        state: &mut AnimationStateMachineState,
        graphs: &Res<AssetStore<AnimationGraph>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        state.blender.sample(
            bone_ids,
            &mut state.state_graphs,
            graphs,
            context,
            pool,
            output,
        );
    }

    fn reset(&self, _state: &mut AnimationStateMachineState) {}

    fn inner_graphs(&self, visit: &mut dyn FnMut(&AssetHandle<AnimationGraph>)) {
        self.state_graphs().for_each(visit);
    }

    fn inner_states(
        &self,
        state: &AnimationStateMachineState,
        visit: &mut dyn FnMut(Option<Name>, &AnimationGraphState),
    ) {
        for (graph, name) in state.state_graphs.iter().zip(&state.state_names) {
            visit(Some(*name), graph);
        }
    }

    fn report(&self, state: &AnimationStateMachineState) -> NodeReport {
        let named = |graph: GraphId| state.state_names.get(*graph).copied();
        let transition = state.blender.in_flight().and_then(|blend| {
            Some(TransitionReport {
                from: named(blend.from)?,
                to: named(blend.to)?,
                progress: blend.progress,
            })
        });
        NodeReport {
            clip_time: None,
            state_machine: Some(StateMachineReport {
                current: state.current_state_name(),
                transition,
            }),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut AnimationStateMachineState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    ) {
        state.state_graphs.iter_mut().for_each(visit);
    }
}

struct FSMStateEntry {
    name: Name,
    graph: AssetHandle<AnimationGraph>,
}

struct FSMTransitionEntry {
    from: Name,
    to: Name,
    trigger: AnimationFSMTrigger,
    blend_time: f32,
}

pub struct AnimationStateMachineBuilder {
    initial_state: Name,
    states: Vec<FSMStateEntry>,
    transitions: Vec<FSMTransitionEntry>,
    blender: TransitionBlender,
}

impl AnimationStateMachineBuilder {
    /// Picks the blending backend for every transition of the machine.
    pub fn blender(mut self, blender: TransitionBlender) -> Self {
        self.blender = blender;
        self
    }

    pub fn state(
        mut self,
        state_name: Name,
        state_graph: AssetHandle<AnimationGraph>,
        f: impl FnOnce(&mut TransitionBuilder),
    ) -> Self {
        self.states.push(FSMStateEntry {
            name: state_name,
            graph: state_graph,
        });
        let mut tb = TransitionBuilder {
            builder: &mut self,
            from: state_name,
        };
        f(&mut tb);
        self
    }

    pub fn build(self) -> AnimationStateMachine {
        let mut name_to_index: HashMap<Name, StateId> = HashMap::new();
        let mut states = Vec::new();
        let mut transitions: Vec<Vec<AnimationStateMachineTransition>> = Vec::new();

        for (index, entry) in self.states.into_iter().enumerate() {
            states.push(AnimationFSMState {
                name: entry.name,
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
                blend_time: entry.blend_time,
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
            blender: self.blender,
        }
    }
}

pub struct TransitionBuilder<'a> {
    builder: &'a mut AnimationStateMachineBuilder,
    from: Name,
}

impl TransitionBuilder<'_> {
    pub fn to(
        &mut self,
        state_name: Name,
        trigger: AnimationFSMTrigger,
        blend_time: f32,
    ) -> &mut Self {
        self.builder.transitions.push(FSMTransitionEntry {
            from: self.from,
            to: state_name,
            trigger,
            blend_time,
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{
        AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
    };
    use concerto_name::name;
    use concerto_reflect::{Reflect, TypeRegistry};
    use uuid::Uuid;

    use super::{
        AnimationFSMTrigger, AnimationStateMachine, AnimationStateMachineState,
        StateMachineEditError,
    };
    use crate::{
        blackboard::AnimationBlackboard,
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::AnimationGraphContext,
        graph::{AnimationGraph, GraphId},
        node::{
            AnimationClipNode, AnimationNode, ErasedAnimationNode, InputSlots,
            ReflectAnimationNode, register_node_inputs,
        },
        player::AnimationPlayer,
        plugin::register_animation_types,
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        testing::{Walks, made_live},
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Counter {}

    impl AnimationNode for Counter {
        type State = u32;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn input_slots(&self) -> InputSlots {
            InputSlots::None
        }

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> u32 {
            0
        }

        fn update(&self, updates: &mut u32, _: f32, _: &(), _: &AnimationGraphContext<'_>) {
            *updates += 1;
        }

        fn evaluate(
            &self,
            _: &mut u32,
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            _: &mut AnimationFrame,
        ) {
        }

        fn reset(&self, _: &mut u32) {}
    }

    fn counting_graph() -> AnimationGraph {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<Counter>();
        let mut graph = AnimationGraph::from_node(Counter {});
        graph.make_live(&registry, None).unwrap();
        graph
    }

    fn updates_in_state(
        machine: &mut AnimationStateMachineState,
        state: usize,
        walks: &Walks,
    ) -> Option<u32> {
        let mut updates = None;
        machine
            .state_graphs
            .get_mut(GraphId::from(state))?
            .for_each_state_mut::<Counter>(
                walks.graphs(),
                &AnimationBlackboard::default(),
                |_, state, _| updates = Some(*state),
            );
        updates
    }

    #[test]
    fn only_the_current_states_graph_is_updated() {
        let mut server = AssetServer::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&graphs);
        let mut add = |graph: AnimationGraph| {
            let handle = server.add(AnimationGraph::default());
            graphs.insert(handle.id(), graph);
            handle
        };
        let first = add(counting_graph());
        let second = add(counting_graph());
        let machine =
            AnimationStateMachine::from_initial_state(name!("first"), first, |transition| {
                transition.to(
                    name!("second"),
                    AnimationFSMTrigger::on_bool(name!("go"), true),
                    0.1,
                );
            })
            .state(name!("second"), second, |_| {})
            .build();
        let machine: Box<dyn ErasedAnimationNode> = Box::new(machine);
        let walks = Walks::with(AssetStore::new(), graphs, register_node_inputs::<Counter>);
        let blackboard = AnimationBlackboard::default();
        let mut state = walks
            .create(&blackboard, |context| machine.create_state(context))
            .unwrap();

        for _ in 0..3 {
            walks.update(&blackboard, |context| {
                machine.update(&mut state, 0.1, context)
            });
        }

        let machine = state.downcast_mut::<AnimationStateMachineState>().unwrap();
        assert_eq!(updates_in_state(machine, 0, &walks), Some(3));
        assert_eq!(updates_in_state(machine, 1, &walks), Some(0));
    }

    enum Graph {
        A,
        B,
    }

    enum Edit {
        AddState(&'static str, Graph),
        RemoveState(&'static str),
        RenameState(&'static str, &'static str),
        SetStateGraph(&'static str, Graph),
        AddTransition(&'static str, &'static str, AnimationFSMTrigger, f32),
        RemoveTransition(&'static str, usize),
        SetTransition(&'static str, usize, AnimationFSMTrigger, f32),
        SetInitial(&'static str),
        LoseLastTransitionList,
    }

    fn without_last_transition_list(machine: &AnimationStateMachine) -> AnimationStateMachine {
        let mut stored = serde_json::to_value(machine).unwrap();
        stored["transitions"].as_array_mut().unwrap().pop();
        serde_json::from_value(stored).unwrap()
    }

    struct EditCase {
        edit: Edit,
        refused: Option<StateMachineEditError>,
        machine: &'static [&'static str],
    }

    fn described(machine: &AnimationStateMachine, a: &AssetHandle<AnimationGraph>) -> Vec<String> {
        machine
            .states()
            .map(|state| {
                let initial = if machine.initial_state() == Some(state.name) {
                    "*"
                } else {
                    ""
                };
                let graph = if state.graph.id() == a.id() { "a" } else { "b" };
                let transitions: Vec<String> = machine
                    .transitions(state.name)
                    .unwrap()
                    .map(|transition| {
                        let trigger = match transition.trigger {
                            AnimationFSMTrigger::Instant => "instant",
                            AnimationFSMTrigger::OnAnimationEnd => "end",
                            AnimationFSMTrigger::BoolEquals { .. } => "bool",
                            AnimationFSMTrigger::Vec2NonZero { .. } => "vec2",
                        };
                        format!(
                            "{}:{} {trigger} {}",
                            transition.index, transition.target, transition.blend_time
                        )
                    })
                    .collect();
                format!(
                    "{}{initial} {graph} -> {}",
                    state.name,
                    transitions.join(", ")
                )
            })
            .collect()
    }

    #[test]
    fn edits_keep_states_and_transitions_consistent() {
        use AnimationFSMTrigger::{Instant, OnAnimationEnd};
        use Edit::*;

        let mut server = AssetServer::new();
        let graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&graphs);
        let a = server.add(AnimationGraph::default());
        let b = server.add(AnimationGraph::default());
        let unknown = |name: &str| Some(StateMachineEditError::UnknownState { name: name.into() });
        let duplicate =
            |name: &str| Some(StateMachineEditError::DuplicateState { name: name.into() });
        let out_of_range = |from: &str, index: usize| {
            Some(StateMachineEditError::TransitionOutOfRange {
                from: from.into(),
                index,
            })
        };
        let cases = [
            EditCase {
                edit: AddState("idle", Graph::A),
                refused: None,
                machine: &["idle* a -> "],
            },
            EditCase {
                edit: AddState("walk", Graph::B),
                refused: None,
                machine: &["idle* a -> ", "walk b -> "],
            },
            EditCase {
                edit: AddState("run", Graph::A),
                refused: None,
                machine: &["idle* a -> ", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddState("walk", Graph::A),
                refused: duplicate("walk"),
                machine: &["idle* a -> ", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddTransition("idle", "walk", Instant, 0.2),
                refused: None,
                machine: &["idle* a -> 0:walk instant 0.2", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddTransition("idle", "run", OnAnimationEnd, 0.3),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> ",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition("idle", "gone", Instant, 0.1),
                refused: unknown("gone"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> ",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition("gone", "idle", Instant, 0.1),
                refused: unknown("gone"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> ",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition("walk", "run", Instant, 0.4),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition("run", "idle", Instant, 0.1),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1",
                ],
            },
            EditCase {
                edit: AddTransition("run", "walk", OnAnimationEnd, 0.6),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk end 0.6",
                ],
            },
            EditCase {
                edit: AddTransition("run", "idle", OnAnimationEnd, 0.7),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run end 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetTransition("idle", 1, Instant, 0.5),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run instant 0.5",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetTransition("idle", 2, Instant, 0.5),
                refused: out_of_range("idle", 2),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run instant 0.5",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetTransition("gone", 0, Instant, 0.5),
                refused: unknown("gone"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run instant 0.5",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: RenameState("run", "sprint"),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: RenameState("walk", "idle"),
                refused: duplicate("idle"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: RenameState("run", "jog"),
                refused: unknown("run"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint a -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetStateGraph("sprint", Graph::B),
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetStateGraph("run", Graph::B),
                refused: unknown("run"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetInitial("walk"),
                refused: None,
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: SetInitial("run"),
                refused: unknown("run"),
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk end 0.6, 2:idle end 0.7",
                ],
            },
            EditCase {
                edit: RemoveTransition("sprint", 0),
                refused: None,
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:walk end 0.6, 1:idle end 0.7",
                ],
            },
            EditCase {
                edit: RemoveTransition("sprint", 2),
                refused: out_of_range("sprint", 2),
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:walk end 0.6, 1:idle end 0.7",
                ],
            },
            EditCase {
                edit: RemoveState("run"),
                refused: unknown("run"),
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:walk end 0.6, 1:idle end 0.7",
                ],
            },
            EditCase {
                edit: RemoveState("walk"),
                refused: None,
                machine: &[
                    "idle* a -> 0:sprint instant 0.5",
                    "sprint b -> 0:idle end 0.7",
                ],
            },
            EditCase {
                edit: LoseLastTransitionList,
                refused: None,
                machine: &["idle* a -> 0:sprint instant 0.5", "sprint b -> "],
            },
            EditCase {
                edit: SetTransition("sprint", 0, Instant, 0.5),
                refused: out_of_range("sprint", 0),
                machine: &["idle* a -> 0:sprint instant 0.5", "sprint b -> "],
            },
            EditCase {
                edit: RemoveTransition("sprint", 0),
                refused: out_of_range("sprint", 0),
                machine: &["idle* a -> 0:sprint instant 0.5", "sprint b -> "],
            },
            EditCase {
                edit: AddTransition("sprint", "idle", Instant, 0.1),
                refused: None,
                machine: &[
                    "idle* a -> 0:sprint instant 0.5",
                    "sprint b -> 0:idle instant 0.1",
                ],
            },
            EditCase {
                edit: LoseLastTransitionList,
                refused: None,
                machine: &["idle* a -> 0:sprint instant 0.5", "sprint b -> "],
            },
            EditCase {
                edit: RemoveState("sprint"),
                refused: None,
                machine: &["idle* a -> "],
            },
        ];

        let mut machine = AnimationStateMachine::default();
        for (row, case) in cases.into_iter().enumerate() {
            let graph = |graph: Graph| match graph {
                Graph::A => a.clone(),
                Graph::B => b.clone(),
            };
            let result = match case.edit {
                AddState(name, state_graph) => machine.add_state(name.into(), graph(state_graph)),
                RemoveState(name) => machine.remove_state(name.into()),
                RenameState(name, to) => machine.rename_state(name.into(), to.into()),
                SetStateGraph(name, state_graph) => {
                    machine.set_state_graph(name.into(), graph(state_graph))
                }
                AddTransition(from, to, trigger, blend_time) => machine
                    .add_transition(from.into(), to.into(), trigger, blend_time)
                    .map(|_| ()),
                RemoveTransition(from, index) => machine.remove_transition(from.into(), index),
                SetTransition(from, index, trigger, blend_time) => {
                    machine.set_transition(from.into(), index, trigger, blend_time)
                }
                SetInitial(name) => machine.set_initial(name.into()),
                LoseLastTransitionList => {
                    machine = without_last_transition_list(&machine);
                    Ok(())
                }
            };

            assert_eq!(result.err(), case.refused, "row {row}");
            assert_eq!(described(&machine, &a), case.machine, "row {row}");
        }
    }

    fn visited(player: &AnimationPlayer, walks: &Walks) -> Vec<String> {
        let mut visits = Vec::new();
        player.for_each_node(walks.graphs(), |visit| {
            let path: Vec<String> = visit
                .path
                .iter()
                .map(|step| match step.state {
                    Some(state) => format!("{}:{state}", step.node.index()),
                    None => step.node.index().to_string(),
                })
                .collect();
            let mut line = format!("{} w{}", path.join("/"), visit.weight);
            if let Some(time) = visit.report.clip_time {
                line.push_str(&format!(" t{time:.2}"));
            }
            if let Some(machine) = &visit.report.state_machine {
                let current = machine.current.map_or("none", |current| current.as_str());
                line.push_str(&format!(" in {current}"));
                if let Some(transition) = machine.transition {
                    line.push_str(&format!(
                        " {}>{} {:.2}",
                        transition.from, transition.to, transition.progress
                    ));
                }
            }
            visits.push(line);
        });
        visits
    }

    #[test]
    fn for_each_node_names_each_node_by_its_path_and_follows_the_machine_through_a_transition() {
        let idle_clip = AssetId::from_path("for_each_node_idle_clip");
        let mut held = AnimationClip::default();
        held.add_channel(
            Uuid::from_u128(1),
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_translation([[0.0; 3]; 2].into_iter()),
            ),
        );
        let mut clips = AssetStore::<AnimationClip>::new();
        clips.insert(idle_clip, held);
        let idle = AssetId::from_path("for_each_node_idle");
        let walk = AssetId::from_path("for_each_node_walk");
        let top = AssetId::from_path("for_each_node_top");
        let machine = AnimationStateMachine::from_initial_state(
            name!("idle"),
            AssetHandle::weak(idle),
            |transition| {
                transition.to(
                    name!("walk"),
                    AnimationFSMTrigger::on_bool(name!("go"), true),
                    0.2,
                );
            },
        )
        .state(name!("walk"), AssetHandle::weak(walk), |_| {})
        .build();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        graphs.insert(
            idle,
            made_live(AnimationGraph::from_node(AnimationClipNode::new(
                AssetHandle::weak(idle_clip),
            ))),
        );
        graphs.insert(walk, counting_graph());
        graphs.insert(top, made_live(AnimationGraph::from_node(machine)));
        let walks = Walks::with(clips, graphs, register_node_inputs::<Counter>);
        let mut player = AnimationPlayer::new(1);
        walks.initialize(&mut player, AssetHandle::weak(top));

        walks.advance(&mut player, 0.05);
        assert_eq!(
            visited(&player, &walks),
            [
                "0 w1",
                "1 w1 in idle",
                "1:idle/0 w1",
                "1:idle/1 w1 t0.05",
                "1:walk/0 w1",
                "1:walk/1 w1",
            ]
        );

        player.set_bool_param(name!("go"), true);
        walks.advance(&mut player, 0.05);
        assert_eq!(visited(&player, &walks)[1], "1 w1 in walk idle>walk 0.25");

        for _ in 0..4 {
            walks.advance(&mut player, 0.05);
        }
        assert_eq!(visited(&player, &walks)[1], "1 w1 in walk");
    }

    #[test]
    fn for_each_state_and_for_each_state_mut_reach_a_node_inside_a_state_graph() {
        let mut server = AssetServer::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&graphs);
        let state_graph = server.add(AnimationGraph::default());
        graphs.insert(state_graph.id(), counting_graph());
        let machine =
            AnimationStateMachine::from_initial_state(name!("only"), state_graph, |_| {}).build();
        let top = server.add(AnimationGraph::default());
        graphs.insert(top.id(), made_live(AnimationGraph::from_node(machine)));
        let walks = Walks::with(AssetStore::new(), graphs, register_node_inputs::<Counter>);
        let mut player = AnimationPlayer::new(1);
        walks.initialize(&mut player, top);

        let mut visits = 0;
        player.for_each_state_mut::<Counter>(walks.graphs(), |_, _, _| visits += 1);
        let mut shared_visits = 0;
        player.for_each_state::<Counter>(walks.graphs(), |_, _, _| shared_visits += 1);

        assert_eq!(visits, 1);
        assert_eq!(shared_visits, 1);
    }
}
