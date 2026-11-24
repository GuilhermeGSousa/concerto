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
    graph::{AnimationGraph, AnimationNodeContext, AnimationNodeData, AnimationNodeIndex},
    node::{AnimationClipNode, AnimationNode, InputSlots, ReflectAnimationNode},
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool},
};

/// Which blackboard `Vec2` param drives a blend space. Read with
/// `blackboard.read(&param).copied().unwrap_or(Vec2::ZERO)`.
#[derive(Debug, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub struct BlendInput {
    pub param: Param<Vec2>,
}

/// A 2D blend space definition: the sample points and which blackboard `Vec2`
/// param drives sampling. Triangulation is built on the state, not here.
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
    type State = BlendSpace2DInstanceNode;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::Variable
    }

    fn create_state(
        &self,
        _: &(),
        _context: &AnimationGraphContext<'_>,
    ) -> BlendSpace2DInstanceNode {
        BlendSpace2DInstanceNode::new(self.points())
    }

    fn update(
        &self,
        state: &mut BlendSpace2DInstanceNode,
        _delta_time: f32,
        _: &(),
        context: &AnimationGraphContext<'_>,
    ) {
        let sample = context
            .blackboard()
            .read(&self.input.param)
            .copied()
            .unwrap_or(Vec2::ZERO);
        state.current_triangulated_point = Some(state.triangulation.locate_or_nearest(sample));
    }

    fn evaluate(
        &self,
        state: &mut BlendSpace2DInstanceNode,
        _: &(),
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[uuid::Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _pool: &mut FramePool,
        output: &mut AnimationFrame,
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

        let Some(triangulated_point) = state.current_triangulated_point else {
            return;
        };

        let triangle = &state.triangulation.triangles()[triangulated_point.triangle];
        let lambda_a = triangulated_point.lambda_a;
        let lambda_b = triangulated_point.lambda_b;
        let lambda_c = triangulated_point.lambda_c;

        output.copy_from(&evaluated_inputs[triangle.a].frame);

        let ab_sum = lambda_a + lambda_b;
        if ab_sum > 1e-6 {
            output.blend(&evaluated_inputs[triangle.b].frame, lambda_b / ab_sum);
        }

        output.blend(&evaluated_inputs[triangle.c].frame, lambda_c);
    }

    fn reset(&self, state: &mut BlendSpace2DInstanceNode) {
        state.current_triangulated_point = None;
    }

    fn reads_input(&self, state: &BlendSpace2DInstanceNode, input: usize, _weight: f32) -> bool {
        state
            .current_triangulated_point
            .and_then(|point| state.triangulation.triangles().get(point.triangle))
            .is_none_or(|triangle| [triangle.a, triangle.b, triangle.c].contains(&input))
    }

    fn evaluate_only_writes_output(&self) -> bool {
        true
    }

    fn input_added(&mut self, index: usize) {
        self.points.insert(index.min(self.points.len()), Vec2::ZERO);
    }

    fn input_removed(&mut self, index: usize) {
        if index < self.points.len() {
            self.points.remove(index);
        }
    }
}

pub struct BlendSpace2DBuilderContext<'a> {
    pub(crate) graph: &'a mut AnimationGraph,
    pub(crate) output_node_index: AnimationNodeIndex,
    pub(crate) points: Vec<Vec2>,
    pub(crate) nodes: Vec<AnimationNodeData>,
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
        self.nodes.push(AnimationNodeData::of(&node));
        self
    }

    pub fn animation_clip_input(
        &mut self,
        clip: AssetHandle<AnimationClip>,
        point: Vec2,
    ) -> &mut Self {
        self.points.push(point);
        self.nodes
            .push(AnimationNodeData::of(&AnimationClipNode::new(clip)));
        self
    }
}

#[cfg(test)]
mod tests {
    use concerto_name::name;
    use glam::Vec2;

    use super::{BlendInput, BlendSpace2DNode};
    use crate::node::AnimationNode;

    #[test]
    fn the_blend_space_keeps_a_point_per_input_as_inputs_come_and_go() {
        let mut space = BlendSpace2DNode::new(
            vec![Vec2::X, Vec2::Y],
            BlendInput {
                param: name!("at").into(),
            },
        );

        space.input_added(1);
        space.input_added(3);
        assert_eq!(space.points(), [Vec2::X, Vec2::ZERO, Vec2::Y, Vec2::ZERO]);

        space.input_removed(0);
        space.input_removed(2);
        assert_eq!(space.points(), [Vec2::ZERO, Vec2::Y]);
    }
}
