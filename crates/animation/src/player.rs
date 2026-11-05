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
use glam::{Vec2, Vec3};

use crate::{
    blackboard::AnimationBlackboard,
    evaluation::{AnimationGraphContext, RootBone},
    graph::{AnimationGraph, AnimationGraphState, AnimationNodeIndex},
    node::{
        AnimationClipNodeState, AnimationNode, ErasedAnimationNode, NodeState,
        state_machine::AnimationStateMachineState,
    },
    pose::PosePool,
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
    pose_pool: PosePool,
    paused: bool,
}

impl AnimationPlayer {
    pub fn new(bone_count: usize) -> Self {
        Self {
            graph_state: AnimationGraphState::default(),
            blackboard: AnimationBlackboard::default(),
            pose_pool: PosePool::new(bone_count),
            paused: false,
        }
    }

    /// Freezes the player: its graph stops advancing and its bones are no
    /// longer posed, so they hold (and can be edited from) their last pose.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn play(&mut self, node_index: &AnimationNodeIndex) {
        if let Some(clip_state) = self
            .graph_state
            .get_state_mut::<AnimationClipNodeState>(node_index)
        {
            clip_state.play();
        }
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

    pub fn current_fsm_state(&self, node_index: &AnimationNodeIndex) -> Option<Name> {
        self.graph_state
            .get_state::<AnimationStateMachineState>(node_index)
            .and_then(AnimationStateMachineState::current_state_name)
    }

    /// Calls `f` with each live node of type `N` in the player's graph, its state and the blackboard.
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
            inputs,
        };

        self.pose_pool.set_rest(rest_pose);
        let mut output_pose = self.pose_pool.acquire();
        if !self.graph_state.evaluate(
            graphs,
            &context,
            bone_ids,
            &mut self.pose_pool,
            &mut output_pose,
        ) {
            // No graph evaluated — keep the bones' current (default) pose.
            self.pose_pool.release(output_pose);
            return false;
        }

        let root_motion = output_pose.root_motion();
        for (bone_index, bone) in bones.iter().enumerate() {
            let Some(bone_entity) = bone.entity() else {
                continue;
            };
            let Some(joint_pose) = output_pose.get_joint_pose(bone_index) else {
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

        self.pose_pool.release(output_pose);
        true
    }
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

    use super::AnimationPlayer;
    use crate::{
        clip::AnimationClip,
        graph::{AnimationGraph, made_live},
        node::{AnimationClipNode, BlendNode, ResultNode},
        test_walks::Walks,
    };

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
    fn for_each_state_mut_visits_exactly_the_live_nodes_of_the_type() {
        let mut fixture = fixture();

        let mut clip_nodes = 0;
        fixture.player.for_each_state_mut::<AnimationClipNode>(
            fixture.walks.graphs(),
            |_, _, _| {
                clip_nodes += 1;
            },
        );
        let mut result_nodes = 0;
        fixture
            .player
            .for_each_state_mut::<ResultNode>(fixture.walks.graphs(), |_, _, _| {
                result_nodes += 1;
            });
        let mut blend_nodes = 0;
        fixture
            .player
            .for_each_state_mut::<BlendNode>(fixture.walks.graphs(), |_, _, _| {
                blend_nodes += 1;
            });

        assert_eq!(clip_nodes, 2);
        assert_eq!(result_nodes, 1);
        assert_eq!(blend_nodes, 0);
    }

    #[test]
    fn for_each_state_mut_hands_each_node_its_state_and_the_blackboard() {
        let mut fixture = fixture();
        fixture.player.set_bool_param(name!("flag"), true);
        fixture.player.set_float_param(name!("speed"), 3.0);

        let mut seen = Vec::new();
        fixture.player.for_each_state_mut::<AnimationClipNode>(
            fixture.walks.graphs(),
            |_, state, board| {
                state.play();
                seen.push((
                    board.get_bool(name!("flag")),
                    board.get_float(name!("speed")),
                ));
            },
        );

        assert_eq!(seen, vec![(Some(true), Some(3.0)); 2]);
    }

    #[test]
    fn for_each_state_mut_visits_nothing_before_the_graph_is_initialized() {
        let fixture = fixture();
        let mut player = AnimationPlayer::new(1);

        let mut visits = 0;
        player.for_each_state_mut::<AnimationClipNode>(fixture.walks.graphs(), |_, _, _| {
            visits += 1;
        });

        assert_eq!(visits, 0);
    }
}
