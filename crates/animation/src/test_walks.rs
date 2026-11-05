use std::cell::RefCell;

use concerto_ecs::{
    World,
    system::input::{InputSet, InputState},
};
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use glam::Affine3A;

use crate::{
    blackboard::AnimationBlackboard,
    clip::AnimationClip,
    evaluation::{AnimationGraphContext, RootBone},
    graph::AnimationGraph,
    node::{CreateNodeInputs, EvaluateNodeInputs, UpdateNodeInputs},
    player::AnimationPlayer,
    plugin::register_animation_node_inputs,
};

type Walk<K> = RefCell<InputState<InputSet<'static, 'static, K>>>;

pub(crate) struct Walks {
    world: World,
    create: Walk<CreateNodeInputs>,
    update: Walk<UpdateNodeInputs>,
    evaluate: Walk<EvaluateNodeInputs>,
}

impl Walks {
    pub(crate) fn new(
        clips: AssetStore<AnimationClip>,
        graphs: AssetStore<AnimationGraph>,
    ) -> Self {
        Self::with(clips, graphs, |_| {})
    }

    pub(crate) fn with(
        clips: AssetStore<AnimationClip>,
        graphs: AssetStore<AnimationGraph>,
        register: impl FnOnce(&mut World),
    ) -> Self {
        let mut world = World::new();
        world.insert_resource(clips);
        world.insert_resource(graphs);
        register_animation_node_inputs(&mut world);
        register(&mut world);
        Self {
            create: RefCell::new(InputState::new(&mut world)),
            update: RefCell::new(InputState::new(&mut world)),
            evaluate: RefCell::new(InputState::new(&mut world)),
            world,
        }
    }

    pub(crate) fn graphs(&self) -> &AssetStore<AnimationGraph> {
        self.world
            .get_resource()
            .expect("a test walk holds the graphs")
    }

    pub(crate) fn clips_mut(&mut self) -> &mut AssetStore<AnimationClip> {
        self.world
            .get_resource_mut()
            .expect("a test walk holds the clips")
    }

    pub(crate) fn graphs_mut(&mut self) -> &mut AssetStore<AnimationGraph> {
        self.world
            .get_resource_mut()
            .expect("a test walk holds the graphs")
    }

    pub(crate) fn into_world(self) -> World {
        self.world
    }

    pub(crate) fn initialize(
        &self,
        player: &mut AnimationPlayer,
        graph: AssetHandle<AnimationGraph>,
    ) {
        let mut state = self.create.borrow_mut();
        let inputs = state
            .get(&self.world)
            .expect("a test walk fetches its inputs");
        player.initialize_graph(graph, self.graphs(), &inputs);
    }

    pub(crate) fn advance(&self, player: &mut AnimationPlayer, delta_time: f32) {
        let mut state = self.update.borrow_mut();
        let inputs = state
            .get(&self.world)
            .expect("a test walk fetches its inputs");
        player.update(delta_time, self.graphs(), None, &inputs);
    }

    pub(crate) fn create<R>(
        &self,
        blackboard: &AnimationBlackboard,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(&self.world, &self.create, blackboard, None, None, None, f)
    }

    pub(crate) fn update<R>(
        &self,
        blackboard: &AnimationBlackboard,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(&self.world, &self.update, blackboard, None, None, None, f)
    }

    pub(crate) fn evaluate<R>(
        &self,
        blackboard: &AnimationBlackboard,
        root: Option<RootBone>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(&self.world, &self.evaluate, blackboard, root, None, None, f)
    }

    #[expect(dead_code)]
    pub(crate) fn evaluate_posed<R>(
        &self,
        blackboard: &AnimationBlackboard,
        skeleton: &Skeleton,
        model_to_world: Option<Affine3A>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            &self.evaluate,
            blackboard,
            None,
            Some(skeleton),
            model_to_world,
            f,
        )
    }
}

fn walk<K: 'static, R>(
    world: &World,
    state: &Walk<K>,
    blackboard: &AnimationBlackboard,
    root: Option<RootBone>,
    skeleton: Option<&Skeleton>,
    model_to_world: Option<Affine3A>,
    f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
) -> R {
    let mut state = state.borrow_mut();
    let inputs = state.get(world).expect("a test walk fetches its inputs");
    let context = AnimationGraphContext {
        blackboard,
        root,
        skeleton,
        model_to_world,
        inputs: &inputs,
    };
    f(&context)
}
