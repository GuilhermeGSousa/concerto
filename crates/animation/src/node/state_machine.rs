use std::collections::HashMap;
use std::fmt;

use crate::graph::{AnimationGraph, AnimationGraphState, AnimationGraphStates, GraphId};
use crate::message::{AnimationMessage, MessageType};
use crate::param::Param;
use crate::pose::{AnimationFrame, EvaluatedFrame, FramePool, RootMotion, WeightedMotion};
use crate::transition::{AnimationTransitionBlender, TransitionBlender};
use crate::{
    evaluation::{AnimationGraphContext, DriveContext, RaisedMessage},
    node::{AnimationNode, EditorAnimationNode, ReflectAnimationNode},
};

use concerto_ecs::Res;
use concerto_foundation::assets::handle::LiveLeaves;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::Name;
use concerto_reflect::{
    EncodeError, Reflect, ReflectDefault, ReflectedValue, TypeRegistry, ValueWarning,
};
use derive_more::Deref;
use glam::Vec2;
use uuid::Uuid;

/// A state for [`AnimationStateMachine::new`]: its name and the graph it plays.
pub struct AnimationFSMStateDefinition {
    /// The state's name, unique in the machine.
    pub name: Name,
    /// The graph the state plays.
    pub graph: AssetHandle<AnimationGraph>,
}

#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationFSMState {
    name: Name,
    graph: AssetHandle<AnimationGraph>,
}

/// When a transition fires; the machine tries the current state's transitions in order each
/// update and takes the first that fires.
#[derive(Debug, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub enum AnimationFSMTrigger {
    /// At once, on the first update in the state.
    #[default]
    Instant,
    /// When a `bool` blackboard param holds a value.
    BoolEquals {
        /// The param to read; a missing param never fires.
        param: Param<bool>,
        /// The value that fires the transition.
        value: bool,
    },
    /// When a [`Vec2`] blackboard param is not zero.
    Vec2NonZero {
        /// The param to read; a missing param never fires.
        param: Param<Vec2>,
    },
    /// On a message of the type `message` and, with a `value`, only on one that holds every
    /// field the value names.
    ///
    /// A message the game sent the player this frame is tried before the state's other
    /// transitions. A message the current state raised is tried after that state's update,
    /// and its transition is taken at the start of the next update unless a transition the
    /// game drives fires there first.
    OnMessage {
        /// The type of message the transition waits for.
        message: MessageType,
        /// The fields the message must hold, compared with [`ReflectedValue::contains`]; any
        /// message of the type when `None`.
        value: Option<ReflectedValue>,
    },
}

impl AnimationFSMTrigger {
    /// A trigger that fires when the `bool` param `param` equals `cond`.
    pub fn on_bool(param: impl Into<Param<bool>>, cond: bool) -> Self {
        Self::BoolEquals {
            param: param.into(),
            value: cond,
        }
    }

    /// A trigger that fires when the `Vec2` param `param` is not zero.
    pub fn on_non_zero_vec(param: impl Into<Param<Vec2>>) -> Self {
        Self::Vec2NonZero {
            param: param.into(),
        }
    }

    /// A trigger that fires on a message of type `M`.
    pub fn on_message<M: AnimationMessage>() -> Self {
        Self::OnMessage {
            message: MessageType::of::<M>(),
            value: None,
        }
    }

    /// A trigger that fires on a message of type `M` whose every field equals the one in
    /// `message`, defaults included.
    ///
    /// # Errors
    ///
    /// Returns an [`EncodeError`] when `message` cannot be encoded through reflection.
    pub fn on_message_value<M: AnimationMessage>(message: &M) -> Result<Self, EncodeError> {
        Ok(Self::OnMessage {
            message: MessageType::of::<M>(),
            value: Some(ReflectedValue::encode(message)?),
        })
    }
}

/// A transition for [`AnimationStateMachine::new`], out of the state it is listed under.
pub struct AnimationStateMachineTransitionDefinition {
    /// The state the transition enters; an unknown name enters the first state.
    pub target_state: Name,
    /// When the transition fires.
    pub trigger: AnimationFSMTrigger,
    /// The cross-fade time or inertialization halflife, in seconds; see [`TransitionBlender`].
    pub blend_time: f32,
}

#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationStateMachineTransition {
    next_state: StateId,
    trigger: AnimationFSMTrigger,
    blend_time: f32,
}

/// The index of a state in its [`AnimationStateMachine`].
#[derive(Clone, Copy, Default, Deref, Reflect, serde::Serialize, serde::Deserialize)]
pub struct StateId(usize);

/// One state of an [`AnimationStateMachine`], as its read access shows it.
pub struct AnimationFSMStateView<'a> {
    /// The state's name.
    pub name: Name,
    /// The graph the state plays.
    pub graph: &'a AssetHandle<AnimationGraph>,
}

/// One transition of an [`AnimationStateMachine`]; `index` is its place among its source state's transitions.
pub struct AnimationFSMTransitionView<'a> {
    /// The transition's place among its source state's transitions.
    pub index: usize,
    /// The state the transition enters.
    pub target: Name,
    /// When the transition fires.
    pub trigger: &'a AnimationFSMTrigger,
    /// The cross-fade time or inertialization halflife, in seconds; see [`TransitionBlender`].
    pub blend_time: f32,
}

/// Why an edit of an [`AnimationStateMachine`] was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateMachineEditError {
    /// No state has the name.
    UnknownState {
        /// The name no state has.
        name: Name,
    },
    /// A state already has the name.
    DuplicateState {
        /// The name already taken.
        name: Name,
    },
    /// The state has no transition at the index.
    TransitionOutOfRange {
        /// The state the transition would leave.
        from: Name,
        /// The index past the state's transitions.
        index: usize,
    },
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

/// A node that plays one of its states' graphs at a time and moves between them by triggered
/// transitions, blended by its [`TransitionBlender`].
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
    /// A machine with `states_definition` in order, starting in `initial_state`.
    ///
    /// Transitions listed under an unknown state are dropped. A transition to an unknown state, and
    /// an unknown `initial_state`, fall back to the first state.
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

    /// Starts a builder whose first state, `state_name`, is the initial one and gets its
    /// transitions from `f`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `from`.
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

    /// Adds a state after the others.
    ///
    /// The first state added is the initial one.
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::DuplicateState`] when a state named `name` exists.
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

    /// Removes a state with every transition to or from it.
    ///
    /// When the removed state was the initial one, the first remaining state becomes initial.
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `name`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `name`,
    /// and [`StateMachineEditError::DuplicateState`] when another state is named `to`.
    pub fn rename_state(&mut self, name: Name, to: Name) -> Result<(), StateMachineEditError> {
        let index = self.state_index(name)?;
        if to != name && self.state_index(to).is_ok() {
            return Err(StateMachineEditError::DuplicateState { name: to });
        }
        self.states[index].name = to;
        Ok(())
    }

    /// Replaces the graph a state plays.
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `name`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `from`
    /// or `to`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `from`,
    /// and [`StateMachineEditError::TransitionOutOfRange`] when it has no transition `index`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `from`,
    /// and [`StateMachineEditError::TransitionOutOfRange`] when it has no transition `index`.
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
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineEditError::UnknownState`] when the machine has no state `name`.
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

    /// Every transition trigger, flattened across states.
    ///
    /// Lets callers inspect trigger payloads without reaching the transition table.
    pub fn triggers(&self) -> impl Iterator<Item = &AnimationFSMTrigger> + '_ {
        self.transitions
            .iter()
            .flatten()
            .map(|transition| &transition.trigger)
    }
}

/// A transition in flight; `progress` runs from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionView {
    /// The state the transition leaves.
    pub from: Name,
    /// The state the transition enters.
    pub to: Name,
    /// How far the blend has run, from 0 to 1.
    pub progress: f32,
}

/// The state of an [`AnimationStateMachine`] in one player: the current state, each state's
/// graph state and the blend between them.
pub struct AnimationStateMachineState {
    state_graphs: AnimationGraphStates,
    current_state: StateId,
    state_names: Vec<Name>,
    blender: Box<dyn AnimationTransitionBlender>,
    reset_pending: bool,
    pending: Option<usize>,
}

impl AnimationStateMachineState {
    pub(crate) fn new(
        initial_state: StateId,
        state_graphs: Vec<AnimationGraphState>,
        state_names: Vec<Name>,
        blender: TransitionBlender,
    ) -> Self {
        Self {
            state_graphs: AnimationGraphStates::new(state_graphs),
            current_state: initial_state,
            state_names,
            blender: blender.starting_in(initial_state.as_graph_id()),
            reset_pending: false,
            pending: None,
        }
    }

    fn restart(&mut self, initial_state: StateId, blender: TransitionBlender) {
        self.current_state = initial_state;
        self.blender = blender.starting_in(initial_state.as_graph_id());
        self.reset_pending = true;
        self.pending = None;
    }

    /// The name of the state the machine is in, or `None` for a machine without states.
    pub fn current_state_name(&self) -> Option<Name> {
        self.state_names.get(*self.current_state).copied()
    }

    /// The transition the machine is blending through, if any.
    pub fn transition(&self) -> Option<TransitionView> {
        let named = |graph: GraphId| self.state_names.get(*graph).copied();
        self.blender.in_flight().and_then(|blend| {
            Some(TransitionView {
                from: named(blend.from)?,
                to: named(blend.to)?,
                progress: blend.progress,
            })
        })
    }

    pub(crate) fn current_graph(&self) -> Option<&AnimationGraphState> {
        self.state_graphs.get(self.current_state.as_graph_id())
    }

    fn begin_transition(
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
    type DriveInput = Res<'static, AssetStore<AnimationGraph>>;

    fn make_live(&mut self, registry: &TypeRegistry, leaves: &mut LiveLeaves) -> Vec<ValueWarning> {
        let mut warnings = Vec::new();
        for transition in self.transitions.iter().flatten() {
            let AnimationFSMTrigger::OnMessage {
                message,
                value: Some(wanted),
            } = &transition.trigger
            else {
                continue;
            };
            let Some(mut probe) = registry
                .get_by_name(message.path())
                .filter(|registration| registration.type_path() == message.path())
                .and_then(|registration| {
                    registration
                        .data::<ReflectDefault>()
                        .map(ReflectDefault::default)
                })
            else {
                continue;
            };
            warnings.extend(
                wanted
                    .apply_with(probe.as_mut(), leaves)
                    .into_iter()
                    .map(|warning| ValueWarning {
                        path: message.path().to_owned(),
                        reason: format!("OnMessage value {}: {}", warning.path, warning.reason),
                    }),
            );
        }
        warnings
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
        context: &mut AnimationGraphContext<'_>,
    ) {
        if std::mem::take(&mut state.reset_pending) {
            state
                .state_graphs
                .iter_mut()
                .for_each(|state_graph| state_graph.reset(graphs));
        }
        let transitions = self
            .get_state_transitions(state.current_state)
            .map_or(&[][..], Vec::as_slice);
        let fired =
            |transition: &&AnimationStateMachineTransition| fires(&transition.trigger, context);
        let on_message = |transition: &&AnimationStateMachineTransition| {
            matches!(transition.trigger, AnimationFSMTrigger::OnMessage { .. })
        };
        let pending = state.pending.take();
        let next = transitions
            .iter()
            .filter(on_message)
            .find(fired)
            .or_else(|| {
                transitions
                    .iter()
                    .filter(|transition| !on_message(transition))
                    .find(fired)
            })
            .or_else(|| pending.and_then(|index| transitions.get(index)));
        if let Some(transition) = next {
            state.begin_transition(graphs, transition);
        }

        let raised = state
            .blender
            .update(delta_time, &mut state.state_graphs, graphs, context);
        state.pending = self
            .get_state_transitions(state.current_state)
            .and_then(|transitions| {
                transitions.iter().position(|transition| {
                    heard(
                        &transition.trigger,
                        raised.iter().map(RaisedMessage::message),
                    )
                })
            });
        for message in raised {
            context.pass_up(message);
        }
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

    fn drive(
        &self,
        state: &mut AnimationStateMachineState,
        graphs: &Res<AssetStore<AnimationGraph>>,
        context: &mut DriveContext<'_>,
        bone_ids: &[Uuid],
        _inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        state.blender.drive(
            bone_ids,
            &mut state.state_graphs,
            graphs,
            context,
            pool,
            output,
        );
    }

    fn root_motion(
        &self,
        state: &mut AnimationStateMachineState,
        graphs: &Res<AssetStore<AnimationGraph>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _inputs: &[WeightedMotion],
    ) -> RootMotion {
        state
            .blender
            .root_motion(bone_ids, &mut state.state_graphs, graphs, context)
    }

    fn reset(&self, state: &mut AnimationStateMachineState) {
        state.restart(self.initial_state, self.blender);
    }

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

    #[cfg(any(test, feature = "test-support"))]
    fn inner_states_mut(
        &self,
        state: &mut AnimationStateMachineState,
        visit: &mut dyn FnMut(&mut AnimationGraphState),
    ) {
        state.state_graphs.iter_mut().for_each(visit);
    }
}

impl EditorAnimationNode for AnimationStateMachine {
    fn describe(&self, state: &AnimationStateMachineState) -> Option<String> {
        let mut line = match state.current_state_name() {
            Some(current) => format!("in {current}"),
            None => "in no state".to_owned(),
        };
        if let Some(transition) = state.transition() {
            line.push_str(&format!(
                " · {} → {} {:.2}",
                transition.from, transition.to, transition.progress
            ));
        }
        Some(line)
    }
}

fn fires(trigger: &AnimationFSMTrigger, context: &AnimationGraphContext<'_>) -> bool {
    match trigger {
        AnimationFSMTrigger::Instant => true,
        AnimationFSMTrigger::BoolEquals { param, value } => {
            context.blackboard().read(param).is_some_and(|v| v == value)
        }
        AnimationFSMTrigger::Vec2NonZero { param } => context
            .blackboard()
            .read(param)
            .is_some_and(|v| v.length_squared() > f32::EPSILON),
        AnimationFSMTrigger::OnMessage { .. } => heard(trigger, context.messages()),
    }
}

fn heard<'m>(
    trigger: &AnimationFSMTrigger,
    mut messages: impl Iterator<Item = &'m dyn Reflect>,
) -> bool {
    let AnimationFSMTrigger::OnMessage {
        message: wanted,
        value,
    } = trigger
    else {
        return false;
    };
    messages.any(|message| {
        wanted.matches(message)
            && value.as_ref().is_none_or(|wanted| {
                ReflectedValue::encode(message).is_ok_and(|encoded| encoded.contains(wanted))
            })
    })
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

/// Builds an [`AnimationStateMachine`] state by state; see
/// [`AnimationStateMachine::from_initial_state`].
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

    /// Adds a state after the others, its transitions added through `f`.
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

    /// The machine; transitions to unknown states enter the first state.
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

/// Adds the transitions out of one state of an [`AnimationStateMachineBuilder`].
pub struct TransitionBuilder<'a> {
    builder: &'a mut AnimationStateMachineBuilder,
    from: Name,
}

impl TransitionBuilder<'_> {
    /// Adds a transition to `state_name`, fired by `trigger`, blended over `blend_time` seconds.
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

    use super::{AnimationFSMTrigger, AnimationStateMachine, StateMachineEditError};
    use crate::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        message::ClipEnded,
        node::{
            AnimationClipNode, AnimationNode, EditorAnimationNode, ReflectAnimationNode,
            register_node_inputs,
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
        type DriveInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> u32 {
            0
        }

        fn update(&self, updates: &mut u32, _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {
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

    impl EditorAnimationNode for Counter {}

    fn counting_graph() -> AnimationGraph {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<Counter>();
        let mut graph = AnimationGraph::from_node(Counter {});
        graph.make_live(&registry, None).unwrap();
        graph
    }

    enum Graph {
        A,
        B,
    }

    enum Edit {
        AddState {
            name: &'static str,
            graph: Graph,
        },
        RemoveState {
            name: &'static str,
        },
        RenameState {
            name: &'static str,
            to: &'static str,
        },
        SetStateGraph {
            name: &'static str,
            graph: Graph,
        },
        AddTransition {
            from: &'static str,
            to: &'static str,
            trigger: AnimationFSMTrigger,
            blend_time: f32,
        },
        RemoveTransition {
            from: &'static str,
            index: usize,
        },
        SetTransition {
            from: &'static str,
            index: usize,
            trigger: AnimationFSMTrigger,
            blend_time: f32,
        },
        SetInitial {
            name: &'static str,
        },
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
                            AnimationFSMTrigger::BoolEquals { .. } => "bool",
                            AnimationFSMTrigger::Vec2NonZero { .. } => "vec2",
                            AnimationFSMTrigger::OnMessage { .. } => "message",
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
        use AnimationFSMTrigger::Instant;
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
                edit: AddState {
                    name: "idle",
                    graph: Graph::A,
                },
                refused: None,
                machine: &["idle* a -> "],
            },
            EditCase {
                edit: AddState {
                    name: "walk",
                    graph: Graph::B,
                },
                refused: None,
                machine: &["idle* a -> ", "walk b -> "],
            },
            EditCase {
                edit: AddState {
                    name: "run",
                    graph: Graph::A,
                },
                refused: None,
                machine: &["idle* a -> ", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddState {
                    name: "walk",
                    graph: Graph::A,
                },
                refused: duplicate("walk"),
                machine: &["idle* a -> ", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddTransition {
                    from: "idle",
                    to: "walk",
                    trigger: Instant,
                    blend_time: 0.2,
                },
                refused: None,
                machine: &["idle* a -> 0:walk instant 0.2", "walk b -> ", "run a -> "],
            },
            EditCase {
                edit: AddTransition {
                    from: "idle",
                    to: "run",
                    trigger: AnimationFSMTrigger::on_message::<ClipEnded>(),
                    blend_time: 0.3,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> ",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition {
                    from: "idle",
                    to: "gone",
                    trigger: Instant,
                    blend_time: 0.1,
                },
                refused: unknown("gone"),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> ",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition {
                    from: "walk",
                    to: "run",
                    trigger: Instant,
                    blend_time: 0.4,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> ",
                ],
            },
            EditCase {
                edit: AddTransition {
                    from: "run",
                    to: "idle",
                    trigger: Instant,
                    blend_time: 0.1,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1",
                ],
            },
            EditCase {
                edit: AddTransition {
                    from: "run",
                    to: "walk",
                    trigger: AnimationFSMTrigger::on_message::<ClipEnded>(),
                    blend_time: 0.6,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk message 0.6",
                ],
            },
            EditCase {
                edit: AddTransition {
                    from: "run",
                    to: "idle",
                    trigger: AnimationFSMTrigger::on_message::<ClipEnded>(),
                    blend_time: 0.7,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run message 0.3",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: SetTransition {
                    from: "idle",
                    index: 1,
                    trigger: Instant,
                    blend_time: 0.5,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run instant 0.5",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: SetTransition {
                    from: "idle",
                    index: 2,
                    trigger: Instant,
                    blend_time: 0.5,
                },
                refused: out_of_range("idle", 2),
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:run instant 0.5",
                    "walk b -> 0:run instant 0.4",
                    "run a -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: RenameState {
                    name: "run",
                    to: "sprint",
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint a -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: SetStateGraph {
                    name: "sprint",
                    graph: Graph::B,
                },
                refused: None,
                machine: &[
                    "idle* a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: SetInitial { name: "walk" },
                refused: None,
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:idle instant 0.1, 1:walk message 0.6, 2:idle message 0.7",
                ],
            },
            EditCase {
                edit: RemoveTransition {
                    from: "sprint",
                    index: 0,
                },
                refused: None,
                machine: &[
                    "idle a -> 0:walk instant 0.2, 1:sprint instant 0.5",
                    "walk* b -> 0:sprint instant 0.4",
                    "sprint b -> 0:walk message 0.6, 1:idle message 0.7",
                ],
            },
            EditCase {
                edit: RemoveState { name: "walk" },
                refused: None,
                machine: &[
                    "idle* a -> 0:sprint instant 0.5",
                    "sprint b -> 0:idle message 0.7",
                ],
            },
            EditCase {
                edit: RemoveState { name: "sprint" },
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
                AddState {
                    name,
                    graph: state_graph,
                } => machine.add_state(name.into(), graph(state_graph)),
                RemoveState { name } => machine.remove_state(name.into()),
                RenameState { name, to } => machine.rename_state(name.into(), to.into()),
                SetStateGraph {
                    name,
                    graph: state_graph,
                } => machine.set_state_graph(name.into(), graph(state_graph)),
                AddTransition {
                    from,
                    to,
                    trigger,
                    blend_time,
                } => machine
                    .add_transition(from.into(), to.into(), trigger, blend_time)
                    .map(|_| ()),
                RemoveTransition { from, index } => machine.remove_transition(from.into(), index),
                SetTransition {
                    from,
                    index,
                    trigger,
                    blend_time,
                } => machine.set_transition(from.into(), index, trigger, blend_time),
                SetInitial { name } => machine.set_initial(name.into()),
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
            if let Some(described) = visit.node.describe(visit.state) {
                line.push_str(&format!(" {described}"));
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
                "1:idle/1 w1 time 0.05 s",
                "1:walk/0 w1",
                "1:walk/1 w1",
            ]
        );
        let mut counters = 0;
        player.for_each_state::<Counter>(walks.graphs(), |_, _, _| counters += 1);
        assert_eq!(counters, 1);

        player.set_bool_param(name!("go"), true);
        walks.advance(&mut player, 0.05);
        assert_eq!(
            visited(&player, &walks)[1],
            "1 w1 in walk · idle → walk 0.25"
        );

        for _ in 0..4 {
            walks.advance(&mut player, 0.05);
        }
        assert_eq!(visited(&player, &walks)[1], "1 w1 in walk");
    }
}
