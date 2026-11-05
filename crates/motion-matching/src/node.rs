use concerto_animation::{
    clip::AnimationClip,
    evaluation::AnimationGraphContext,
    node::{effective_play_rate, AnimationNode, ReflectAnimationNode},
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};
use concerto_ecs::Res;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::skeleton::Skeleton;
use concerto_name::{name, Name};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use uuid::Uuid;

use crate::{
    database::MotionDatabase,
    simulation::TrajectorySample,
    state::{MotionMatchingState, Sources},
};

/// The blackboard key under which the trajectory is published.
pub(crate) fn trajectory_key() -> Name {
    name!("trajectory")
}

/// The character's simulation transform and predicted path, published on the animation blackboard for the node to match against.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub(crate) struct MotionMatchingTrajectory {
    pub translation: Vec3,
    pub rotation: Quat,
    pub samples: Vec<TrajectorySample>,
}

impl Default for MotionMatchingTrajectory {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            samples: Vec::new(),
        }
    }
}

fn sources<'a>(
    node: &MotionMatchingNode,
    databases: &'a AssetStore<MotionDatabase>,
    skeletons: &'a AssetStore<Skeleton>,
    clips: &'a AssetStore<AnimationClip>,
    player_skeleton: Option<&'a Skeleton>,
) -> Option<Sources<'a>> {
    let database = databases.get(&node.database)?;
    Some(Sources {
        database,
        skeleton: skeletons.get(database.skeleton())?,
        player_skeleton: player_skeleton?,
        clips: database
            .clips()
            .iter()
            .map(|handle| clips.get(handle))
            .collect::<Option<_>>()?,
    })
}

/// Plays a motion database, jumping to the frame whose features best match the character's predicted trajectory.
#[derive(Clone, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct MotionMatchingNode {
    pub database: AssetHandle<MotionDatabase>,
    pub search_period: f32,
    pub velocity_change_threshold: f32,
    pub rotation_change_threshold: f32,
    pub same_frame_tolerance: usize,
    pub ignore_surrounding: usize,
    pub ignore_range_end: usize,
    pub blend_halflife: f32,
    pub play_rate: f32,
    pub play_rate_param: Option<Name>,
}

impl Default for MotionMatchingNode {
    fn default() -> Self {
        Self {
            database: AssetHandle::default(),
            search_period: 0.1,
            velocity_change_threshold: 50.0,
            rotation_change_threshold: 50.0,
            same_frame_tolerance: 1,
            ignore_surrounding: 20,
            ignore_range_end: 20,
            blend_halflife: 0.1,
            play_rate: 1.0,
            play_rate_param: None,
        }
    }
}

impl AnimationNode for MotionMatchingNode {
    type State = MotionMatchingState;
    type CreateInput = ();
    type UpdateInput = (
        Res<'static, AssetStore<MotionDatabase>>,
        Res<'static, AssetStore<Skeleton>>,
        Res<'static, AssetStore<AnimationClip>>,
    );
    type EvaluateInput = ();

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> MotionMatchingState {
        MotionMatchingState::new(self.search_period)
    }

    fn update(
        &self,
        state: &mut MotionMatchingState,
        delta_time: f32,
        (databases, skeletons, clips): &(
            Res<AssetStore<MotionDatabase>>,
            Res<AssetStore<Skeleton>>,
            Res<AssetStore<AnimationClip>>,
        ),
        context: &AnimationGraphContext<'_>,
    ) {
        state.clear_fired();
        match sources(self, databases, skeletons, clips, context.skeleton()) {
            Some(sources) => {
                let rate =
                    effective_play_rate(self.play_rate, self.play_rate_param, context.blackboard());
                state.step(self, delta_time, rate, context.blackboard(), sources)
            }
            None => state.clear_root_motion(),
        }
    }

    fn evaluate(
        &self,
        state: &mut MotionMatchingState,
        _: &(),
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedFrame],
        _pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        state.write_output(output);
        for event in state.fired() {
            output.push_event(*event);
        }
    }

    fn reset(&self, state: &mut MotionMatchingState) {
        *state = MotionMatchingState::new(self.search_period);
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        graph::AnimationGraph, node::ReflectAnimationNode, plugin::register_animation_types,
    };
    use concerto_foundation::assets::handle::{AssetHandle, LiveLeaves};
    use concerto_name::name;
    use concerto_reflect::{to_value, ReflectedValue, TypeRegistry};

    use super::{MotionMatchingNode, MotionMatchingTrajectory};
    use crate::database::MotionDatabase;

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<MotionMatchingNode>();
        registry.register::<MotionMatchingTrajectory>();
        registry.register::<AssetHandle<MotionDatabase>>();
        registry
    }

    fn live_node(graph: &AnimationGraph) -> &MotionMatchingNode {
        graph
            .live_nodes()
            .find_map(|node| node.as_reflect().downcast_ref::<MotionMatchingNode>())
            .expect("the graph holds a motion matching node")
    }

    #[test]
    fn the_default_node_has_orangeducks_values() {
        let node = MotionMatchingNode::default();

        assert_eq!(node.search_period, 0.1);
        assert_eq!(node.velocity_change_threshold, 50.0);
        assert_eq!(node.rotation_change_threshold, 50.0);
        assert_eq!(node.same_frame_tolerance, 1);
        assert_eq!(node.ignore_surrounding, 20);
        assert_eq!(node.ignore_range_end, 20);
        assert_eq!(node.blend_halflife, 0.1);
        assert_eq!(node.play_rate, 1.0);
        assert_eq!(node.play_rate_param, None);
        assert_eq!(
            node.database.id(),
            AssetHandle::<MotionDatabase>::default().id()
        );
    }

    #[test]
    fn the_node_is_an_animation_node() {
        let registry = registry();
        let data = registry
            .get_type_data::<ReflectAnimationNode>(std::any::TypeId::of::<MotionMatchingNode>())
            .copied();

        assert!(data.is_some());
    }

    #[test]
    fn a_graph_holding_the_node_round_trips_through_serialization() {
        let registry = registry();
        let node = MotionMatchingNode {
            search_period: 0.25,
            velocity_change_threshold: 12.0,
            rotation_change_threshold: 13.0,
            same_frame_tolerance: 3,
            ignore_surrounding: 7,
            ignore_range_end: 9,
            blend_halflife: 0.4,
            play_rate: 1.5,
            play_rate_param: Some(name!("speed")),
            ..Default::default()
        };
        let graph = AnimationGraph::from_node(node);

        let bytes = bincode::serialize(&graph).unwrap();
        let mut restored: AnimationGraph = bincode::deserialize(&bytes).unwrap();
        restored.make_live(&registry, None).unwrap();

        assert!(restored.load_warnings().is_empty());
        let restored = live_node(&restored);
        assert_eq!(restored.search_period, 0.25);
        assert_eq!(restored.velocity_change_threshold, 12.0);
        assert_eq!(restored.rotation_change_threshold, 13.0);
        assert_eq!(restored.same_frame_tolerance, 3);
        assert_eq!(restored.ignore_surrounding, 7);
        assert_eq!(restored.ignore_range_end, 9);
        assert_eq!(restored.blend_halflife, 0.4);
        assert_eq!(restored.play_rate, 1.5);
        assert_eq!(restored.play_rate_param, Some(name!("speed")));
    }

    #[test]
    fn a_stored_node_without_a_play_rate_loads_at_the_defaults_with_no_warning() {
        let registry = registry();
        let node = MotionMatchingNode {
            search_period: 0.25,
            play_rate: 1.5,
            play_rate_param: Some(name!("speed")),
            ..Default::default()
        };
        let ReflectedValue::Struct(mut fields) = to_value(&node).unwrap() else {
            panic!("a motion matching node is stored as a struct");
        };
        fields.retain(|(name, _)| name != "play_rate" && name != "play_rate_param");
        let data = *registry
            .get_type_data::<ReflectAnimationNode>(std::any::TypeId::of::<MotionMatchingNode>())
            .unwrap();

        let mut leaves = LiveLeaves::from_parts(&registry, None);
        let (built, warnings) = (data.from_value)(&ReflectedValue::Struct(fields), &mut leaves);

        assert!(warnings.is_empty(), "{warnings:?}");
        let built = built
            .as_reflect()
            .downcast_ref::<MotionMatchingNode>()
            .unwrap();
        assert_eq!(built.search_period, 0.25);
        assert_eq!(built.play_rate, 1.0);
        assert_eq!(built.play_rate_param, None);
    }

    #[test]
    fn a_graph_with_the_default_node_round_trips_with_its_defaults() {
        let registry = registry();
        let graph = AnimationGraph::from_node(MotionMatchingNode::default());

        let bytes = bincode::serialize(&graph).unwrap();
        let mut restored: AnimationGraph = bincode::deserialize(&bytes).unwrap();
        restored.make_live(&registry, None).unwrap();

        let defaults = MotionMatchingNode::default();
        let restored = live_node(&restored);
        assert!(restored.search_period == defaults.search_period);
        assert!(restored.same_frame_tolerance == defaults.same_frame_tolerance);
    }
}
