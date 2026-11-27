use concerto_foundation::{
    assets::handle::AssetHandle,
    geometry::delauney::{TriangulatedPoint2D, Triangulation2D},
};
use concerto_reflect::Reflect;
use glam::Vec2;
use log::warn;

use crate::{
    clip::AnimationClip,
    evaluation::{AnimationGraphContext, DriveContext},
    graph::{AnimationGraph, AnimationNodeContext, AnimationNodeData, AnimationNodeIndex},
    node::{
        AnimationClipNode, AnimationNode, EditorAnimationNode, InputSlots, ReflectAnimationNode,
    },
    param::Param,
    pose::{AnimationFrame, EvaluatedFrame, FramePool, RootMotion, WeightedMotion},
};

/// Which blackboard `Vec2` param drives a blend space. Read with
/// `blackboard.read(&param).copied().unwrap_or(Vec2::ZERO)`.
#[derive(Debug, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub struct BlendInput {
    /// The blackboard param holding the point to sample at; zero when it is missing.
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

    /// The sample points, one per input, in input order.
    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    /// The param that drives sampling.
    pub fn input(&self) -> &BlendInput {
        &self.input
    }
}

/// The state of a [`BlendSpace2DNode`]: the triangulation of its points and the triangle the
/// param last fell in.
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

struct Corners {
    a: usize,
    b: usize,
    b_share: Option<f32>,
    c: usize,
    c_share: f32,
}

fn corners(state: &BlendSpace2DInstanceNode) -> Option<Corners> {
    let point = state.current_triangulated_point?;
    let triangle = &state.triangulation.triangles()[point.triangle];
    let ab_sum = point.lambda_a + point.lambda_b;
    Some(Corners {
        a: triangle.a,
        b: triangle.b,
        b_share: (ab_sum > 1e-6).then(|| point.lambda_b / ab_sum),
        c: triangle.c,
        c_share: point.lambda_c,
    })
}

impl AnimationNode for BlendSpace2DNode {
    type State = BlendSpace2DInstanceNode;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();
    type DriveInput = ();

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

        let Some(corners) = corners(state) else {
            return;
        };

        output.copy_from(&evaluated_inputs[corners.a].frame);
        if let Some(share) = corners.b_share {
            output.blend(&evaluated_inputs[corners.b].frame, share);
        }
        output.blend(&evaluated_inputs[corners.c].frame, corners.c_share);
    }

    fn drive(
        &self,
        state: &mut BlendSpace2DInstanceNode,
        _: &(),
        _: &DriveContext<'_>,
        _: &[uuid::Uuid],
        inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        if inputs.is_empty() || inputs.len() != self.points().len() {
            return;
        }
        let Some(corners) = corners(state) else {
            return;
        };
        let attributes = output.attributes_mut();
        attributes.copy_from(inputs[corners.a].frame.attributes());
        if let Some(share) = corners.b_share {
            attributes.blend(inputs[corners.b].frame.attributes(), share);
        }
        attributes.blend(inputs[corners.c].frame.attributes(), corners.c_share);
    }

    fn drive_shares(
        &self,
        state: &BlendSpace2DInstanceNode,
        _: &DriveContext<'_>,
        shares: &mut [f32],
    ) {
        let corners = corners(state).filter(|_| shares.len() == self.points().len());
        shares.fill(0.0);
        let Some(corners) = corners.filter(|_| !shares.is_empty()) else {
            return;
        };
        let b_share = corners.b_share.unwrap_or(0.0);
        shares[corners.a] += (1.0 - b_share) * (1.0 - corners.c_share);
        shares[corners.b] += b_share * (1.0 - corners.c_share);
        shares[corners.c] += corners.c_share;
    }

    fn root_motion(
        &self,
        state: &mut BlendSpace2DInstanceNode,
        _: &(),
        _context: &AnimationGraphContext<'_>,
        _bone_ids: &[uuid::Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        if inputs.is_empty() || inputs.len() != self.points().len() {
            return RootMotion::IDENTITY;
        }

        let Some(triangulated_point) = state.current_triangulated_point else {
            return RootMotion::IDENTITY;
        };

        let triangle = &state.triangulation.triangles()[triangulated_point.triangle];
        let lambda_a = triangulated_point.lambda_a;
        let lambda_b = triangulated_point.lambda_b;
        let lambda_c = triangulated_point.lambda_c;

        let mut motion = inputs[triangle.a].motion;
        let ab_sum = lambda_a + lambda_b;
        if ab_sum > 1e-6 {
            motion = motion.blend(inputs[triangle.b].motion, lambda_b / ab_sum);
        }
        motion.blend(inputs[triangle.c].motion, lambda_c)
    }

    fn reset(&self, state: &mut BlendSpace2DInstanceNode) {
        state.current_triangulated_point = None;
    }
}

impl EditorAnimationNode for BlendSpace2DNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Variable
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

/// Adds the inputs of a blend space being built by
/// [`AnimationNodeContext::add_blend_space_2d_input_with`].
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

    /// Adds `node` as an input sampled at `point`.
    pub fn input<T: AnimationNode>(&mut self, node: T, point: Vec2) -> &mut Self {
        self.points.push(point);
        self.nodes.push(AnimationNodeData::of(&node));
        self
    }

    /// Adds a clip node that loops `clip` as an input sampled at `point`.
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

    use concerto_foundation::assets::asset_store::AssetStore;
    use glam::Vec3;

    use super::{BlendInput, BlendSpace2DInstanceNode, BlendSpace2DNode};
    use crate::{
        blackboard::AnimationBlackboard,
        node::{AnimationNode, EditorAnimationNode},
        pose::{RootMotion, WeightedMotion},
        testing::Walks,
    };

    #[test]
    fn the_blend_space_keeps_a_point_per_input_as_inputs_come_and_go_and_blends_no_motion_when_they_differ()
     {
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

        let triangle = [Vec2::ZERO, Vec2::X, Vec2::Y];
        let space = BlendSpace2DNode::new(
            triangle[..2].to_vec(),
            BlendInput {
                param: name!("at").into(),
            },
        );
        let mut state = BlendSpace2DInstanceNode::new(&triangle);
        let moving = |x| WeightedMotion {
            motion: RootMotion {
                translation: Vec3::new(x, 0.0, 0.0),
                ..RootMotion::IDENTITY
            },
            weight: 1.0,
        };
        let walks = Walks::new(AssetStore::new(), AssetStore::new());

        let motion = walks.evaluate(&AnimationBlackboard::default(), None, |context| {
            space.update(&mut state, 0.0, &(), context);
            space.root_motion(
                &mut state,
                &(),
                context,
                &[],
                &[moving(1.0), moving(2.0), moving(3.0)],
            )
        });

        assert_eq!(motion, RootMotion::IDENTITY);
    }
}
