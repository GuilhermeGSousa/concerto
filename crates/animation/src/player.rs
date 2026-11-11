use std::ops::Deref;

use concerto_ecs::{
    Entity,
    component::{Component, scene::SceneEntityRef},
    entity::hierarchy::ChildOf,
    query::Query,
    system::input::Inputs,
};
use concerto_foundation::{
    assets::{asset_store::AssetStore, handle::AssetHandle},
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::skeleton::Skeleton;
use concerto_name::Name;
use concerto_reflect::Reflect;
use glam::{Affine3A, Vec2, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, RootBone},
    graph::{AnimationGraph, AnimationGraphState, AnimationNodeIndex},
    node::{AnimationNode, ErasedAnimationNode, NodeState},
    pose::{FiredEvent, FramePool},
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
    paused: bool,
    event_threshold: f32,
    pub(crate) fired: Vec<FiredEvent>,
}

impl AnimationPlayer {
    pub fn new(bone_count: usize) -> Self {
        Self {
            graph_state: AnimationGraphState::default(),
            blackboard: AnimationBlackboard::default(),
            frame_pool: FramePool::new(bone_count),
            paused: false,
            event_threshold: 0.0,
            fired: Vec::new(),
        }
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
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: None,
            skeleton: None,
            model_to_world: None,
            inputs,
        };
        self.graph_state
            .initialize(animation_graph, graphs, &context);
    }

    pub(crate) fn update(
        &mut self,
        delta_time: f32,
        graphs: &AssetStore<AnimationGraph>,
        skeleton: Option<&Skeleton>,
        inputs: &Inputs<'_, '_>,
    ) {
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root: None,
            skeleton,
            model_to_world: None,
            inputs,
        };
        self.graph_state.update(delta_time, graphs, &context);
    }

    pub(crate) fn evaluate(
        &mut self,
        graphs: &AssetStore<AnimationGraph>,
        inputs: &Inputs<'_, '_>,
        skeleton: &Skeleton,
        bones: &[SceneEntityRef],
        transforms: &Query<&mut Transform>,
        root_bones: &Query<&mut AnimationRootBone>,
        parents: &Query<&ChildOf>,
        globals: &Query<&GlobalTransform>,
    ) -> bool {
        self.fired.clear();
        let bone_ids = &skeleton.bone_ids;
        let rest_pose = &skeleton.rest_pose;
        let root = bones
            .iter()
            .enumerate()
            .find_map(|(index, bone)| {
                let entity = bone.entity()?;
                root_bones
                    .contains_entity(entity)
                    .then_some((index, entity))
            })
            .and_then(|(index, entity)| {
                Some(RootBone {
                    index,
                    rest: rest_pose.get(index)?.clone(),
                    up: up_of(entity, parents, globals),
                })
            });
        let context = AnimationGraphContext {
            blackboard: &self.blackboard,
            root,
            skeleton: Some(skeleton),
            model_to_world: Some(model_to_world_of(bones, parents, globals)),
            inputs,
        };

        self.frame_pool.set_rest(rest_pose);
        let mut output_frame = self.frame_pool.acquire();
        if !self.graph_state.evaluate(
            graphs,
            &context,
            bone_ids,
            &mut self.frame_pool,
            &mut output_frame,
        ) {
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

        let root_motion = output_frame.root_motion();
        for (bone_index, bone) in bones.iter().enumerate() {
            let Some(bone_entity) = bone.entity() else {
                continue;
            };
            let Some(joint_pose) = output_frame.joint(bone_index) else {
                continue;
            };

            if let Some(mut root_bone) = root_bones.get_entity(bone_entity) {
                root_bone.root_motion = root_motion;
            }

            if let Some(mut transform) = transforms.get_entity(bone_entity) {
                transform.translation = joint_pose.translation;
                transform.rotation = joint_pose.rotation;
                transform.scale = joint_pose.scale;
            }
        }

        self.frame_pool.release(output_frame);
        true
    }
}

fn model_to_world_of(
    bones: &[SceneEntityRef],
    parents: &Query<&ChildOf>,
    globals: &Query<&GlobalTransform>,
) -> Affine3A {
    bones
        .first()
        .and_then(|bone| bone.entity())
        .and_then(|bone| parents.get_entity(bone))
        .and_then(|parent| globals.get_entity(parent.parent()))
        .map_or(Affine3A::IDENTITY, |global| {
            Affine3A::from_mat4(global.matrix())
        })
}

fn up_of(root: Entity, parents: &Query<&ChildOf>, globals: &Query<&GlobalTransform>) -> Vec3 {
    parents
        .get_entity(root)
        .and_then(|parent| globals.get_entity(parent.parent()))
        .map(|parent| parent.rotation().inverse() * Vec3::Y)
        .filter(|up| up.is_finite())
        .unwrap_or(Vec3::Y)
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
    use concerto_name::name;
    use concerto_reflect::{Reflect, TypeRegistry};
    use uuid::Uuid;

    use super::AnimationPlayer;
    use crate::{
        blackboard::AnimationBlackboard,
        clip::AnimationClip,
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{
            AnimationClipNode, AnimationNode, BlendNode, ReflectAnimationNode, ResultNode,
            register_node_inputs,
        },
        plugin::register_animation_types,
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        testing::{Walks, made_live},
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct Replayable {}

    impl AnimationNode for Replayable {
        type State = u32;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

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

    struct Fixture {
        _server: AssetServer,
        walks: Walks,
        player: AnimationPlayer,
    }

    fn fixture() -> Fixture {
        let mut server = AssetServer::new();
        let mut clips = AssetStore::<AnimationClip>::new();
        let mut graphs = AssetStore::<AnimationGraph>::new();
        server.register_asset(&clips);
        server.register_asset(&graphs);

        let first = server.add(AnimationClip::default());
        clips.insert(first.id(), AnimationClip::default());
        let second = server.add(AnimationClip::default());
        clips.insert(second.id(), AnimationClip::default());

        let mut graph = AnimationGraph::from_node(AnimationClipNode::new(first));
        graph
            .result_node()
            .with_input(AnimationClipNode::new(second), |_| {});
        let handle = server.add(AnimationGraph::default());
        graphs.insert(handle.id(), made_live(graph));

        let mut player = AnimationPlayer::new(1);
        let walks = Walks::new(clips, graphs);
        walks.initialize(&mut player, handle);
        Fixture {
            _server: server,
            walks,
            player,
        }
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

    #[derive(Debug, PartialEq)]
    struct Board {
        flag: Option<bool>,
        speed: Option<f32>,
    }

    fn board(blackboard: &AnimationBlackboard) -> Board {
        Board {
            flag: blackboard.get_bool(name!("flag")),
            speed: blackboard.get_float(name!("speed")),
        }
    }

    fn set_board() -> Board {
        Board {
            flag: Some(true),
            speed: Some(3.0),
        }
    }

    #[test]
    fn for_each_state_and_for_each_state_mut_visit_only_the_live_nodes_of_the_type_with_the_blackboard()
     {
        let mut fixture = fixture();
        let mut uninitialized = AnimationPlayer::new(1);
        fixture.player.set_bool_param(name!("flag"), true);
        fixture.player.set_float_param(name!("speed"), 3.0);
        let graphs = fixture.walks.graphs();

        let mut mutable = Vec::new();
        fixture
            .player
            .for_each_state_mut::<AnimationClipNode>(graphs, |_, state, blackboard| {
                state.play();
                mutable.push(board(blackboard));
            });
        let mut shared = Vec::new();
        fixture
            .player
            .for_each_state::<AnimationClipNode>(graphs, |_, _, blackboard| {
                shared.push(board(blackboard))
            });
        let mut result_nodes = 0;
        fixture
            .player
            .for_each_state::<ResultNode>(graphs, |_, _, _| result_nodes += 1);
        let mut blend_nodes = 0;
        fixture
            .player
            .for_each_state_mut::<BlendNode>(graphs, |_, _, _| blend_nodes += 1);
        let mut before_initialization = 0;
        uninitialized
            .for_each_state_mut::<AnimationClipNode>(graphs, |_, _, _| before_initialization += 1);

        assert_eq!(mutable, [set_board(), set_board()]);
        assert_eq!(shared, mutable);
        assert_eq!(result_nodes, 1);
        assert_eq!(blend_nodes, 0);
        assert_eq!(before_initialization, 0);
    }
}
