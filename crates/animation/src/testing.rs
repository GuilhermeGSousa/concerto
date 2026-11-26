use std::cell::RefCell;

use concerto_ecs::{
    Entity, World,
    system::input::{InputSet, InputState},
};
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_mesh::Skeleton;
use concerto_reflect::TypeRegistry;
use glam::Affine3A;

use crate::{
    blackboard::AnimationBlackboard,
    clip::AnimationClip,
    evaluation::{AnimationGraphContext, Outbox, RootBone},
    graph::AnimationGraph,
    mask::BoneMask,
    node::{CreateNodeInputs, EvaluateNodeInputs, UpdateNodeInputs},
    player::AnimationPlayer,
    plugin::register_animation_node_inputs,
    pose::RootMotion,
    retarget::RetargetRig,
};

type Walk<K> = RefCell<InputState<InputSet<'static, 'static, K>>>;

pub(crate) struct Walks {
    world: World,
    entity: Entity,
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
        world.insert_resource(AssetStore::<BoneMask>::new());
        world.insert_resource(AssetStore::<Skeleton>::new());
        world.insert_resource(AssetStore::<RetargetRig>::new());
        register_animation_node_inputs(&mut world);
        register(&mut world);
        let entity = world.spawn(());
        Self {
            entity,
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

    pub(crate) fn masks_mut(&mut self) -> &mut AssetStore<BoneMask> {
        self.world
            .get_resource_mut()
            .expect("a test walk holds the masks")
    }

    pub(crate) fn skeletons_mut(&mut self) -> &mut AssetStore<Skeleton> {
        self.world
            .get_resource_mut()
            .expect("a test walk holds the skeletons")
    }

    pub(crate) fn rigs_mut(&mut self) -> &mut AssetStore<RetargetRig> {
        self.world
            .get_resource_mut()
            .expect("a test walk holds the retarget rigs")
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
        player.initialize_graph(self.entity, graph, self.graphs(), &inputs);
    }

    pub(crate) fn advance(&self, player: &mut AnimationPlayer, delta_time: f32) {
        let mut state = self.update.borrow_mut();
        let inputs = state
            .get(&self.world)
            .expect("a test walk fetches its inputs");
        player.update(self.entity, delta_time, self.graphs(), None, None, &inputs);
    }

    pub(crate) fn advance_with(
        &self,
        player: &mut AnimationPlayer,
        delta_time: f32,
        skeleton: &Skeleton,
        root: Option<RootBone>,
    ) {
        let mut state = self.update.borrow_mut();
        let inputs = state
            .get(&self.world)
            .expect("a test walk fetches its inputs");
        player.update(
            self.entity,
            delta_time,
            self.graphs(),
            Some(skeleton),
            root,
            &inputs,
        );
    }

    pub(crate) fn root_motion(
        &self,
        player: &mut AnimationPlayer,
        skeleton: &Skeleton,
    ) -> RootMotion {
        let mut state = self.update.borrow_mut();
        let inputs = state
            .get(&self.world)
            .expect("a test walk fetches its inputs");
        player.root_motion(self.entity, self.graphs(), skeleton, &inputs)
    }

    pub(crate) fn create<R>(
        &self,
        blackboard: &AnimationBlackboard,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
            &self.create,
            blackboard,
            None,
            None,
            None,
            f,
        )
    }

    pub(crate) fn update<R>(
        &self,
        blackboard: &AnimationBlackboard,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
            &self.update,
            blackboard,
            None,
            None,
            None,
            f,
        )
    }

    pub(crate) fn pass<R>(
        &self,
        blackboard: &AnimationBlackboard,
        root: Option<RootBone>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
            &self.update,
            blackboard,
            root,
            None,
            None,
            f,
        )
    }

    pub(crate) fn evaluate<R>(
        &self,
        blackboard: &AnimationBlackboard,
        root: Option<RootBone>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
            &self.evaluate,
            blackboard,
            root,
            None,
            None,
            f,
        )
    }

    pub(crate) fn evaluate_with<R>(
        &self,
        blackboard: &AnimationBlackboard,
        root: Option<RootBone>,
        skeleton: Option<&Skeleton>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
            &self.evaluate,
            blackboard,
            root,
            skeleton,
            None,
            f,
        )
    }

    pub(crate) fn evaluate_posed<R>(
        &self,
        blackboard: &AnimationBlackboard,
        skeleton: &Skeleton,
        model_to_world: Option<Affine3A>,
        f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
    ) -> R {
        walk(
            &self.world,
            self.entity,
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
    entity: Entity,
    state: &Walk<K>,
    blackboard: &AnimationBlackboard,
    root: Option<RootBone>,
    skeleton: Option<&Skeleton>,
    model_to_world: Option<Affine3A>,
    f: impl FnOnce(&AnimationGraphContext<'_>) -> R,
) -> R {
    let mut state = state.borrow_mut();
    let inputs = state.get(world).expect("a test walk fetches its inputs");
    let outbox = Outbox::default();
    let context = AnimationGraphContext {
        blackboard,
        root,
        skeleton,
        model_to_world,
        inputs: &inputs,
        entity,
        outbox: &outbox,
        messages: &[],
        raised: None,
    };
    f(&context)
}

pub(crate) fn made_live(mut graph: AnimationGraph) -> AnimationGraph {
    let mut registry = TypeRegistry::default();
    crate::plugin::register_animation_types(&mut registry);
    graph.make_live(&registry, None).unwrap();
    graph
}
