use concerto_animation::{
    clip::AnimationClip,
    evaluation::AnimationGraphContext,
    node::{effective_play_rate, AnimationNode, EditorAnimationNode, ReflectAnimationNode},
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, RootMotion, WeightedMotion},
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
    let database = databases
        .get(&node.database)
        .filter(|database| database.is_baked())?;
    Some(Sources {
        database,
        skeleton: skeletons.get(&database.skeleton)?,
        player_skeleton: player_skeleton?,
        clips: database
            .clips()
            .map(|handle| clips.get(handle))
            .collect::<Option<_>>()?,
    })
}

/// Plays a motion database, jumping to the frame whose features best match the character's predicted trajectory.
#[derive(Clone, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode, Default)]
pub struct MotionMatchingNode {
    /// The database to play; the node holds the rest pose until it is baked and its clips load.
    pub database: AssetHandle<MotionDatabase>,
    /// The time between searches, in seconds; also the least time between two forced searches.
    pub search_period: f32,
    /// The change of the desired velocity, in metres per second squared, whose settling forces a
    /// search: one runs when it falls below this after being at or above it.
    pub velocity_change_threshold: f32,
    /// The angular speed of the desired facing, in radians per second, whose settling forces a
    /// search, as [`Self::velocity_change_threshold`] does.
    pub rotation_change_threshold: f32,
    /// How many frames from the current one a better frame must be for the node to jump to it.
    pub same_frame_tolerance: usize,
    /// How many frames either side of the current one are left out of a search.
    pub ignore_surrounding: usize,
    /// How many frames at the end of each clip range are left out of a search; reaching them
    /// forces a search.
    pub ignore_range_end: usize,
    /// The halflife, in seconds, of the inertialization that hides each jump.
    pub blend_halflife: f32,
    /// How many times faster than baked the database plays.
    pub play_rate: f32,
    /// The `f32` blackboard param the play rate is multiplied by, when it holds one.
    pub play_rate_param: Option<Param<f32>>,
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

impl MotionMatchingNode {
    /// A node that plays `database` with the default search settings.
    pub fn new(database: AssetHandle<MotionDatabase>) -> Self {
        Self {
            database,
            ..Default::default()
        }
    }

    /// Sets the time between searches, in seconds.
    pub fn with_search_period(mut self, search_period: f32) -> Self {
        self.search_period = search_period;
        self
    }

    /// Sets the change of the desired velocity, in metres per second squared, whose settling
    /// forces a search.
    pub fn with_velocity_change_threshold(mut self, velocity_change_threshold: f32) -> Self {
        self.velocity_change_threshold = velocity_change_threshold;
        self
    }

    /// Sets the angular speed of the desired facing, in radians per second, whose settling forces
    /// a search.
    pub fn with_rotation_change_threshold(mut self, rotation_change_threshold: f32) -> Self {
        self.rotation_change_threshold = rotation_change_threshold;
        self
    }

    /// Sets how many frames from the current one a better frame must be to jump to it.
    pub fn with_same_frame_tolerance(mut self, same_frame_tolerance: usize) -> Self {
        self.same_frame_tolerance = same_frame_tolerance;
        self
    }

    /// Sets how many frames either side of the current one a search leaves out.
    pub fn with_ignore_surrounding(mut self, ignore_surrounding: usize) -> Self {
        self.ignore_surrounding = ignore_surrounding;
        self
    }

    /// Sets how many frames at the end of each clip range a search leaves out.
    pub fn with_ignore_range_end(mut self, ignore_range_end: usize) -> Self {
        self.ignore_range_end = ignore_range_end;
        self
    }

    /// Sets the halflife, in seconds, of the inertialization that hides each jump.
    pub fn with_blend_halflife(mut self, blend_halflife: f32) -> Self {
        self.blend_halflife = blend_halflife;
        self
    }

    /// Sets how many times faster than baked the database plays.
    pub fn with_play_rate(mut self, play_rate: f32) -> Self {
        self.play_rate = play_rate;
        self
    }

    /// Multiplies the play rate by the `f32` blackboard param named `play_rate_param`, when it
    /// holds one.
    pub fn with_play_rate_param(mut self, play_rate_param: impl Into<Param<f32>>) -> Self {
        self.play_rate_param = Some(play_rate_param.into());
        self
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
        context: &mut AnimationGraphContext<'_>,
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

    fn root_motion(
        &self,
        state: &mut MotionMatchingState,
        _: &(
            Res<AssetStore<MotionDatabase>>,
            Res<AssetStore<Skeleton>>,
            Res<AssetStore<AnimationClip>>,
        ),
        _context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        _inputs: &[WeightedMotion],
    ) -> RootMotion {
        state.delta(bone_ids.len())
    }

    fn reset(&self, state: &mut MotionMatchingState) {
        *state = MotionMatchingState::new(self.search_period);
    }
}

impl EditorAnimationNode for MotionMatchingNode {}

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
    fn a_graph_holding_the_node_round_trips_through_serialization() {
        let registry = registry();
        let node = MotionMatchingNode::default()
            .with_search_period(0.25)
            .with_velocity_change_threshold(12.0)
            .with_rotation_change_threshold(13.0)
            .with_same_frame_tolerance(3)
            .with_ignore_surrounding(7)
            .with_ignore_range_end(9)
            .with_blend_halflife(0.4)
            .with_play_rate(1.5)
            .with_play_rate_param(name!("speed"));
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
        assert_eq!(
            restored.play_rate_param,
            Some(name!("speed")).map(Into::into)
        );
    }

    #[test]
    fn a_stored_node_without_a_play_rate_loads_at_the_defaults_with_no_warning() {
        let registry = registry();
        let node = MotionMatchingNode::default()
            .with_search_period(0.25)
            .with_play_rate(1.5)
            .with_play_rate_param(name!("speed"));
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
}
