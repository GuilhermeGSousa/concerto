use concerto_animation::{
    evaluation::AnimationGraphContext,
    node::{AnimationNode, ReflectAnimationNode},
    pose::{EvaluatedPose, Pose, PosePool},
};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};
use uuid::Uuid;

use crate::{
    database::MotionDatabase, instance::MotionMatchingInstance, simulation::TrajectorySample,
};

/// The blackboard key under which the trajectory is published.
pub const TRAJECTORY_KEY: &str = "trajectory";

/// The character's simulation transform and predicted path, published on the animation blackboard for the node to match against.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct MotionMatchingTrajectory {
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

/// Plays a motion database, jumping to the frame whose features best match the character's predicted trajectory.
#[derive(Clone, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct MotionMatchingNode {
    pub database: AssetHandle<MotionDatabase>,
    pub search_period: f32,
    pub velocity_change_threshold: f32,
    pub rotation_change_threshold: f32,
    pub same_frame_tolerance: usize,
    pub blend_halflife: f32,
}

impl Default for MotionMatchingNode {
    fn default() -> Self {
        Self {
            database: AssetHandle::default(),
            search_period: 0.1,
            velocity_change_threshold: 50.0,
            rotation_change_threshold: 50.0,
            same_frame_tolerance: 1,
            blend_halflife: 0.1,
        }
    }
}

impl AnimationNode for MotionMatchingNode {
    type Instance = MotionMatchingInstance;

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) -> MotionMatchingInstance {
        MotionMatchingInstance::new(self.search_period)
    }

    fn update(
        &self,
        instance: &mut MotionMatchingInstance,
        delta_time: f32,
        _context: &AnimationGraphContext<'_>,
    ) {
        instance.activate(delta_time);
    }

    fn evaluate(
        &self,
        instance: &mut MotionMatchingInstance,
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[Uuid],
        _evaluated_inputs: &[EvaluatedPose],
        _pool: &mut PosePool,
        output: &mut Pose,
    ) {
        instance.write_output(output);
    }

    fn reset(&self, instance: &mut MotionMatchingInstance) {
        *instance = MotionMatchingInstance::new(self.search_period);
    }
}

#[cfg(test)]
mod tests {
    use concerto_animation::{
        graph::AnimationGraph, node::ReflectAnimationNode, plugin::register_animation_types,
    };
    use concerto_foundation::assets::handle::AssetHandle;
    use concerto_reflect::TypeRegistry;

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
        assert_eq!(node.blend_halflife, 0.1);
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
            blend_halflife: 0.4,
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
        assert_eq!(restored.blend_halflife, 0.4);
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
