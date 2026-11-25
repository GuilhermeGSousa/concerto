use std::ops::Deref;

use concerto_ecs::{
    component::{Component, scene::SceneEntityRef},
    entity::hierarchy::ChildOf,
    query::Query,
    system::input::Inputs,
};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    transform::{GlobalTransform, Transform, parent_world_of},
};
use concerto_mesh::skeleton::Skeleton;
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Affine3A, Vec2, Vec3};

use crate::{
    attribute::PoseAttributes,
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, DriveContext, EmittedSignal, Extras, Outbox, RootBone},
    graph::{AnimationGraph, AnimationGraphState, AnimationNodeIndex, NodeVisit},
    message::AnimationMessage,
    node::{AnimationNode, ErasedAnimationNode, NodeState},
    pose::{FiredEvent, FramePool, RootMotion},
    root::AnimationRootBone,
};

pub struct ActiveNodeState {
    pub weight: f32,
    pub(crate) node_state: NodeState,
}

impl ActiveNodeState {
    pub(crate) fn update(
        &mut self,
        node: &dyn ErasedAnimationNode,
        delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        node.update(&mut self.node_state, delta_time, context);
    }
}

#[derive(Component)]
pub struct AnimationPlayer {
    graph_state: AnimationGraphState,
    blackboard: AnimationBlackboard,
    frame_pool: FramePool,
    root: Option<RootBone>,
    paused: bool,
    event_threshold: f32,
    pub(crate) fired: Vec<FiredEvent>,
    pub(crate) emitted: Vec<Box<dyn EmittedSignal>>,
    attributes: PoseAttributes,
    drive_attributes: PoseAttributes,
    extras: Extras,
    messages: Vec<Box<dyn Reflect>>,
}

impl AnimationPlayer {
    pub fn new(bone_count: usize) -> Self {
        Self {
            graph_state: AnimationGraphState::default(),
            blackboard: AnimationBlackboard::default(),
            frame_pool: FramePool::new(bone_count),
            root: None,
            paused: false,
            event_threshold: 0.0,
            fired: Vec::new(),
            emitted: Vec::new(),
            attributes: PoseAttributes::default(),
            drive_attributes: PoseAttributes::default(),
            extras: Extras::default(),
            messages: Vec::new(),
        }
    }

    /// Hands every node of the graph `value`, read with `AnimationGraphContext::extra`; replaces the value of the same type.
    pub fn set_extra<T: Send + Sync + 'static>(&mut self, value: T) {
        self.extras.set(value);
    }

    /// The value of type `T` handed to the player with `set_extra`.
    pub fn extra<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extras.get::<T>()
    }

    /// Queues `message` for the next update, where every state machine checks its transitions against the whole queue before every playing node receives each message in order; a paused player keeps its queue until it resumes, and one whose graph is not initialized yet drops it.
    pub fn send(&mut self, message: impl AnimationMessage) {
        self.messages.push(Box::new(message));
    }

    /// Whether a state machine playing in the player's graph is in the state named `name`.
    pub fn in_state(&self, graphs: &AssetStore<AnimationGraph>, name: &Name) -> bool {
        self.graph_state.in_state(graphs, name)
    }

    /// The per-bone attributes of the frame the player last evaluated.
    pub fn attributes(&self) -> &PoseAttributes {
        &self.attributes
    }

    /// Walks the graph once for a physics step of `delta_time` seconds, each node saying through `AnimationNode::drive` what it asks of physics, and keeps the attributes it gives; the graph does not advance.
    pub fn drive(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        inputs: &Inputs<'_, '_>,
        delta_time: f32,
    ) -> &PoseAttributes {
        let outbox = Outbox::default();
        let graph = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: self.root.clone(),
            skeleton: None,
            model_to_world: None,
            inputs,
            extras: &self.extras,
            outbox: &outbox,
        };
        let context = DriveContext {
            delta_time,
            graph: &graph,
        };
        let mut output = self.frame_pool.acquire();
        if self
            .graph_state
            .drive(graphs, &context, &[], &mut self.frame_pool, &mut output)
        {
            std::mem::swap(&mut self.drive_attributes, output.attributes_mut());
        }
        self.frame_pool.release(output);
        self.emitted.extend(outbox.into_signals());
        &self.drive_attributes
    }

    /// The per-bone attributes of the last physics step's drive.
    pub fn drive_attributes(&self) -> &PoseAttributes {
        &self.drive_attributes
    }

    /// Drops clip events whose source weighs no more than `threshold` in the final frame; 0 by default.
    pub fn set_event_threshold(&mut self, threshold: f32) {
        self.event_threshold = threshold;
    }

    /// Freezes the player: its graph stops advancing and its bones are no
    /// longer posed, so they hold (and can be edited from) their last pose.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Restarts the node at `node_index` through its `AnimationNode::reset`; does nothing until the graph is loaded.
    pub fn play(&mut self, node_index: &AnimationNodeIndex, graphs: &AssetStore<AnimationGraph>) {
        self.graph_state.reset_node(node_index, graphs);
    }

    /// Sets the weight a `BlendNode` gives this node's pose among its inputs.
    pub fn set_node_weight(&mut self, node_index: &AnimationNodeIndex, weight: f32) {
        self.graph_state.set_node_weight(node_index, weight);
    }

    pub fn set_param<T: Reflect>(&mut self, key: Name, value: T) {
        self.blackboard.set(key, value);
    }

    /// Sets a parameter to a value whose type is known only at run time, as an editor's preview does.
    pub fn set_boxed_param(&mut self, key: Name, value: Box<dyn Reflect>) {
        self.blackboard.set_boxed(key, value);
    }

    pub fn set_bool_param(&mut self, key: Name, value: bool) {
        self.set_param(key, value);
    }

    pub fn set_int_param(&mut self, key: Name, value: u32) {
        self.set_param(key, value);
    }

    pub fn set_float_param(&mut self, key: Name, value: f32) {
        self.set_param(key, value);
    }

    pub fn set_vec2_param(&mut self, key: Name, value: Vec2) {
        self.set_param(key, value);
    }

    pub fn get_param<T: Reflect>(&self, key: Name) -> Option<&T> {
        self.blackboard.get::<T>(key)
    }

    pub fn get_bool_param(&self, key: Name) -> Option<bool> {
        self.blackboard.get_bool(key)
    }

    pub fn get_int_param(&self, key: Name) -> Option<u32> {
        self.blackboard.get_int(key)
    }

    pub fn get_float_param(&self, key: Name) -> Option<f32> {
        self.blackboard.get_float(key)
    }

    pub fn get_vec2_param(&self, key: Name) -> Option<Vec2> {
        self.blackboard.get_vec2(key)
    }

    /// Calls `f` with each live node of type `N` in the player's graph, inner graphs included, its state and the blackboard.
    pub fn for_each_state<N: AnimationNode>(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        f: impl FnMut(&N, &N::State, &AnimationBlackboard),
    ) {
        self.graph_state.for_each_state(graphs, &self.blackboard, f);
    }

    /// Calls `f` with each playing node of the player's graph, inner graphs included: its path from the root graph, its weight and what it reports.
    pub fn for_each_node(
        &self,
        graphs: &AssetStore<AnimationGraph>,
        f: impl FnMut(&NodeVisit<'_>),
    ) {
        self.graph_state.for_each_node(graphs, f);
    }

    /// Calls `f` with each live node of type `N` in the player's graph, inner graphs included, its mutable state and the blackboard.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn for_each_state_mut<N: AnimationNode>(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        f: impl FnMut(&N, &mut N::State, &AnimationBlackboard),
    ) {
        self.graph_state
            .for_each_state_mut(graphs, &self.blackboard, f);
    }

    pub(crate) fn initialize_graph(
        &mut self,
        animation_graph: AssetHandle<AnimationGraph>,
        graphs: &AssetStore<AnimationGraph>,
        inputs: &Inputs<'_, '_>,
    ) {
        let outbox = Outbox::default();
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: None,
            skeleton: None,
            model_to_world: None,
            inputs,
            extras: &self.extras,
            outbox: &outbox,
        };
        self.graph_state
            .initialize(animation_graph, graphs, &context);
        self.emitted.extend(outbox.into_signals());
    }

    pub(crate) fn update(
        &mut self,
        delta_time: f32,
        graphs: &AssetStore<AnimationGraph>,
        skeleton: Option<&Skeleton>,
        root: Option<RootBone>,
        inputs: &Inputs<'_, '_>,
    ) {
        self.root = root;
        let outbox = Outbox::default();
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: self.root.clone(),
            skeleton,
            model_to_world: None,
            inputs,
            extras: &self.extras,
            outbox: &outbox,
        };
        self.graph_state.route(graphs, &self.messages, &context);
        for message in &self.messages {
            self.graph_state.receive(graphs, message.as_ref(), &context);
        }
        self.messages.clear();
        self.graph_state.update(delta_time, graphs, &context);
        self.emitted.extend(outbox.into_signals());
    }

    pub(crate) fn root_motion(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        skeleton: &Skeleton,
        inputs: &Inputs<'_, '_>,
    ) -> RootMotion {
        let outbox = Outbox::default();
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: self.root.clone(),
            skeleton: Some(skeleton),
            model_to_world: None,
            inputs,
            extras: &self.extras,
            outbox: &outbox,
        };
        let motion = self
            .graph_state
            .root_motion(graphs, &context, &skeleton.bone_ids);
        self.emitted.extend(outbox.into_signals());
        motion
    }

    pub(crate) fn evaluate(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        inputs: &Inputs<'_, '_>,
        skeleton: &Skeleton,
        bones: &[SceneEntityRef],
        transforms: &mut Query<&mut Transform>,
        parents: &Query<&ChildOf>,
        globals: &Query<&GlobalTransform>,
    ) -> bool {
        self.fired.clear();
        let bone_ids = &skeleton.bone_ids;
        let rest_pose = &skeleton.rest_pose;
        let model_to_world =
            bones
                .first()
                .and_then(|bone| bone.entity())
                .map_or(Affine3A::IDENTITY, |bone| {
                    parent_world_of(bone, parents, globals, |entity| {
                        transforms
                            .get_entity_mut(entity)
                            .map(|transform| local_of(&transform))
                    })
                });
        let outbox = Outbox::default();
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: self.root.clone(),
            skeleton: Some(skeleton),
            model_to_world: Some(model_to_world),
            inputs,
            extras: &self.extras,
            outbox: &outbox,
        };

        self.frame_pool.set_rest(rest_pose);
        let mut output_frame = self.frame_pool.acquire();
        let evaluated = self.graph_state.evaluate(
            graphs,
            &context,
            bone_ids,
            &mut self.frame_pool,
            &mut output_frame,
        );
        self.emitted.extend(outbox.into_signals());
        if !evaluated {
            // No graph evaluated — keep the bones' current (default) pose.
            self.frame_pool.release(output_frame);
            return false;
        }

        let threshold = self.event_threshold;
        self.fired.extend(
            output_frame
                .events
                .drain(..)
                .filter(|event| event.weight > threshold),
        );

        for (bone_index, bone) in bones.iter().enumerate() {
            let Some(bone_entity) = bone.entity() else {
                continue;
            };
            let Some(joint_pose) = output_frame.joint(bone_index) else {
                continue;
            };

            if let Some(mut transform) = transforms.get_entity_mut(bone_entity) {
                transform.translation = joint_pose.translation;
                transform.rotation = joint_pose.rotation;
                transform.scale = joint_pose.scale;
            }
        }

        std::mem::swap(&mut self.attributes, output_frame.attributes_mut());
        self.frame_pool.release(output_frame);
        true
    }
}

pub(crate) fn root_bone_of(
    skeleton: &Skeleton,
    bones: &[SceneEntityRef],
    root_bones: &Query<&mut AnimationRootBone>,
    parents: &Query<&ChildOf>,
    transforms: &Query<&Transform>,
    globals: &Query<&GlobalTransform>,
) -> Option<RootBone> {
    let (index, entity) = bones.iter().enumerate().find_map(|(index, bone)| {
        let entity = bone.entity()?;
        root_bones
            .contains_entity(entity)
            .then_some((index, entity))
    })?;
    let parent = parent_world_of(entity, parents, globals, |entity| {
        transforms.get_entity(entity).map(local_of)
    });
    let up = Some(parent.to_scale_rotation_translation().1.inverse() * Vec3::Y)
        .filter(|up| up.is_finite())
        .unwrap_or(Vec3::Y);
    Some(RootBone {
        index,
        rest: skeleton.rest_pose.get(index)?.clone(),
        up,
    })
}

fn local_of(transform: &Transform) -> Affine3A {
    Affine3A::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
}

#[derive(Component)]
pub struct AnimationHandleComponent {
    pub handle: AssetHandle<AnimationGraph>,
}

impl AnimationHandleComponent {
    pub fn new(handle: AssetHandle<AnimationGraph>) -> Self {
        Self { handle }
    }
}

impl Deref for AnimationHandleComponent {
    type Target = AssetHandle<AnimationGraph>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

#[cfg(test)]
mod tests {
    use concerto_foundation::assets::{asset_server::AssetServer, asset_store::AssetStore};

    use concerto_reflect::{Reflect, TypeRegistry};
    use uuid::Uuid;

    use super::AnimationPlayer;
    use crate::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{AnimationNode, InputSlots, ReflectAnimationNode, register_node_inputs},
        plugin::register_animation_types,
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        testing::Walks,
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Replayable {}

    impl AnimationNode for Replayable {
        type State = u32;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();
        type DriveInput = ();

        fn input_slots(&self) -> InputSlots {
            InputSlots::None
        }

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> u32 {
            0
        }

        fn update(&self, _: &mut u32, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

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

        fn reset(&self, resets: &mut u32) {
            *resets += 1;
        }
    }

    fn replayable_resets(player: &mut AnimationPlayer, walks: &Walks) -> Vec<u32> {
        let mut resets = Vec::new();
        player.for_each_state_mut::<Replayable>(walks.graphs(), |_, state, _| {
            resets.push(*state);
        });
        resets
    }

    #[test]
    fn play_restarts_any_node_type_through_its_reset() {
        let mut graph = AnimationGraph::new();
        let result = graph.result_node().index();
        let index = graph.add_node(Replayable {}, result).index();
        let mut server = AssetServer::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&graphs);
        let handle = server.add(AnimationGraph::default());
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<Replayable>();
        graph.make_live(&registry, None).unwrap();
        graphs.insert(handle.id(), graph);
        let walks = Walks::with(
            AssetStore::new(),
            graphs,
            register_node_inputs::<Replayable>,
        );
        let mut player = AnimationPlayer::new(1);
        walks.initialize(&mut player, handle);
        assert_eq!(replayable_resets(&mut player, &walks), [0]);

        player.play(&index, walks.graphs());

        assert_eq!(replayable_resets(&mut player, &walks), [1]);
    }
}
