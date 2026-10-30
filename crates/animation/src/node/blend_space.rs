use concerto_foundation::{
    assets::handle::AssetHandle,
    geometry::delauney::{TriangulatedPoint2D, Triangulation2D},
};
use concerto_reflect::Reflect;
use glam::Vec2;
use log::warn;

use crate::{
    clip::AnimationClip,
    evaluation::AnimationGraphContext,
    graph::{AnimationGraph, AnimationNodeContext, AnimationNodeIndex, SerializedAnimationNode},
    node::{AnimationClipNode, AnimationNode, ReflectAnimationNode},
    pose::{EvaluatedPose, Pose, PosePool},
};

/// Which blackboard `Vec2` param drives a blend space. Read with
/// `blackboard.get_vec2(&param).unwrap_or(Vec2::ZERO)`.
#[derive(Debug, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub struct BlendInput {
    pub param: String,
}

/// A 2D blend space definition: the sample points and which blackboard `Vec2`
/// param drives sampling. Triangulation is built on the instance, not here.
#[derive(Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(AnimationNode)]
pub struct BlendSpace2DNode {
    points: Vec<Vec2>,
    input: BlendInput,
}

impl BlendSpace2DNode {
    pub(crate) fn new(points: Vec<Vec2>, input: BlendInput) -> Self {
        Self { points, input }
    }

    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    pub fn input(&self) -> &BlendInput {
        &self.input
    }
}

pub struct BlendSpace2DInstanceNode {
    triangulation: Triangulation2D,
    current_triangulated_point: Option<TriangulatedPoint2D>,
}

impl BlendSpace2DInstanceNode {
    pub(crate) fn new(points: &[Vec2]) -> Self {
        Self {
            triangulation: Triangulation2D::build(points.to_vec()),
            current_triangulated_point: None,
        }
    }
}

impl AnimationNode for BlendSpace2DNode {
    type Instance = BlendSpace2DInstanceNode;

    fn create_instance(&self, _context: &AnimationGraphContext<'_>) -> BlendSpace2DInstanceNode {
        BlendSpace2DInstanceNode::new(self.points())
    }

    fn update(
        &self,
        instance: &mut BlendSpace2DInstanceNode,
        _delta_time: f32,
        context: &AnimationGraphContext<'_>,
    ) {
        let sample = context
            .blackboard()
            .get_vec2(&self.input.param)
            .unwrap_or(Vec2::ZERO);
        instance.current_triangulated_point =
            Some(instance.triangulation.locate_or_nearest(sample));
    }

    fn evaluate(
        &self,
        instance: &mut BlendSpace2DInstanceNode,
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[uuid::Uuid],
        evaluated_inputs: &[EvaluatedPose],
        _pool: &mut PosePool,
        output: &mut Pose,
    ) {
        if evaluated_inputs.is_empty() {
            return;
        }

        if evaluated_inputs.len() != self.points().len() {
            warn!(
                "Blend Space inputs and points count are different, this should not happen. Skipping this node"
            );
            return;
        }

        let Some(triangulated_point) = instance.current_triangulated_point else {
            return;
        };

        let triangle = &instance.triangulation.triangles()[triangulated_point.triangle];
        let lambda_a = triangulated_point.lambda_a;
        let lambda_b = triangulated_point.lambda_b;
        let lambda_c = triangulated_point.lambda_c;

        output.copy_from(&evaluated_inputs[triangle.a].pose);

        let ab_sum = lambda_a + lambda_b;
        if ab_sum > 1e-6 {
            output.blend(&evaluated_inputs[triangle.b].pose, lambda_b / ab_sum);
        }

        output.blend(&evaluated_inputs[triangle.c].pose, lambda_c);
    }

    fn reset(&self, instance: &mut BlendSpace2DInstanceNode) {
        instance.current_triangulated_point = None;
    }
}

pub struct BlendSpace2DBuilderContext<'a> {
    pub(crate) graph: &'a mut AnimationGraph,
    pub(crate) output_node_index: AnimationNodeIndex,
    pub(crate) points: Vec<Vec2>,
    pub(crate) nodes: Vec<SerializedAnimationNode>,
    pub(crate) input: BlendInput,
}

impl<'a> BlendSpace2DBuilderContext<'a> {
    pub(crate) fn build(self) -> AnimationNodeContext<'a> {
        let blend_space = BlendSpace2DNode::new(self.points, self.input);

        let blend_space_node = self
            .graph
            .add_node(blend_space, self.output_node_index)
            .index();

        for node in self.nodes.into_iter() {
            self.graph.add_stored_node(node, blend_space_node);
        }

        AnimationNodeContext {
            graph: self.graph,
            node_index: self.output_node_index,
        }
    }

    pub fn input<T: AnimationNode>(&mut self, node: T, point: Vec2) -> &mut Self {
        self.points.push(point);
        self.nodes.push(SerializedAnimationNode::of(&node));
        self
    }

    pub fn animation_clip_input(
        &mut self,
        clip: AssetHandle<AnimationClip>,
        point: Vec2,
    ) -> &mut Self {
        self.points.push(point);
        self.nodes
            .push(SerializedAnimationNode::of(&AnimationClipNode::new(clip)));
        self
    }
}
