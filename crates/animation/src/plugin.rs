use concerto_app::{App, edit_time::AtPreviewTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{IntoSystemConfig, World, reflect::AppTypeRegistry};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::skeleton::Skeleton;
use concerto_reflect::{GetTypeRegistration, TypeRegistry};

use crate::{
    clip::AnimationClip,
    event::{AnimationEvent, ClipEvent},
    graph::AnimationGraph,
    node::{
        AnimationClipNode, AnimationNode, BlendNode, PlaceholderNode, ResultNode,
        blend_space::BlendSpace2DNode, register_node_inputs, state_machine::AnimationStateMachine,
    },
    target::{
        animate_targets, initialize_animation_players, initialize_skeletons,
        update_animation_players,
    },
};

/// Registers the built-in animation nodes, the built-in clip event, and the handle types a graph stores.
pub fn register_animation_types(registry: &mut TypeRegistry) {
    registry.register::<ResultNode>();
    registry.register::<BlendNode>();
    registry.register::<AnimationClipNode>();
    registry.register::<BlendSpace2DNode>();
    registry.register::<AnimationStateMachine>();
    registry.register::<PlaceholderNode>();
    registry.register::<AnimationEvent>();
    registry.register::<AssetHandle<AnimationClip>>();
    registry.register::<AssetHandle<AnimationGraph>>();
}

/// Registers the inputs of the built-in animation nodes with the graph walks of `world`.
pub fn register_animation_node_inputs(world: &mut World) {
    register_node_inputs::<ResultNode>(world);
    register_node_inputs::<BlendNode>(world);
    register_node_inputs::<AnimationClipNode>(world);
    register_node_inputs::<BlendSpace2DNode>(world);
    register_node_inputs::<AnimationStateMachine>(world);
    register_node_inputs::<PlaceholderNode>(world);
}

/// Adds an animation node type to an `App`.
pub trait AnimationAppExt {
    /// Registers `N` with the type registry and its inputs with the graph walks; call it in a plugin's `build`.
    fn register_animation_node<N: AnimationNode + GetTypeRegistration>(&mut self) -> &mut Self;

    /// Registers `S` with the type registry, so clips that store it make it live on load.
    fn register_clip_event<S: ClipEvent + GetTypeRegistration>(&mut self) -> &mut Self;
}

impl AnimationAppExt for App {
    fn register_animation_node<N: AnimationNode + GetTypeRegistration>(&mut self) -> &mut Self {
        self.register_type::<N>();
        register_node_inputs::<N>(self.main_mut().world_mut());
        self
    }

    fn register_clip_event<S: ClipEvent + GetTypeRegistration>(&mut self) -> &mut Self {
        self.register_type::<S>();
        self
    }
}

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_asset::<AnimationClip>();
        app.register_asset::<AnimationGraph>();
        app.register_asset::<Skeleton>();

        if let Some(registry) = app.get_resource::<AppTypeRegistry>() {
            register_animation_types(&mut registry.write());
        }
        register_animation_node_inputs(app.main_mut().world_mut());

        app.add_system(LateUpdate, initialize_skeletons.at_preview_time())
            .add_system(
                LateUpdate,
                animate_targets
                    .after(update_animation_players)
                    .at_preview_time(),
            )
            .add_system(LateUpdate, initialize_animation_players.at_preview_time())
            .add_system(LateUpdate, update_animation_players.at_preview_time());
    }
}

#[cfg(test)]
mod tests {
    use concerto_app::{App, plugins::AssetManagerPlugin, schedule_groups::LateUpdate};
    use concerto_ecs::{
        IntoSystem, Res, ResMut, System, World,
        component::scene::SceneEntityRef,
        resource::Resource,
        system::{
            access::SystemAccess,
            executor::single_thread::SingleThreadedExecutor,
            input::{InputSet, InputState},
            meta::SystemMetadata,
            schedule::Schedules,
        },
    };
    use concerto_foundation::{
        assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
    use concerto_reflect::Reflect;
    use glam::Mat4;
    use uuid::Uuid;

    use super::{AnimationAppExt, AnimationPlugin, register_animation_types};
    use crate::{
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{AnimationNode, CreateNodeInputs, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        target::{animate_targets, initialize_animation_players, update_animation_players},
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct OrderProbe {}

    impl AnimationNode for OrderProbe {
        type State = bool;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> bool {
            false
        }

        fn update(&self, state: &mut bool, _: f32, _: &(), _: &AnimationGraphContext<'_>) {
            *state = true;
        }

        fn evaluate(
            &self,
            state: &mut bool,
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            output: &mut AnimationFrame,
        ) {
            if let Some(joint) = output.joint_mut(0) {
                joint.translation.x = if *state { 1.0 } else { 0.0 };
            }
        }

        fn reset(&self, state: &mut bool) {
            *state = false;
        }
    }

    #[test]
    fn animate_targets_runs_after_update_animation_players() {
        let bone_id = Uuid::from_u128(11);

        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.register_plugin(AnimationPlugin);

        let world = app.main_mut().world_mut();
        let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
        let skeleton = Skeleton::new(
            vec![Mat4::IDENTITY],
            vec![None],
            vec!["bone".into()],
            vec![JointPose::IDENTITY],
            vec![bone_id],
        )
        .unwrap();
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(skeleton_handle.id(), skeleton);

        let mut registry = concerto_reflect::TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<OrderProbe>();
        let mut graph = AnimationGraph::from_node(OrderProbe {});
        graph.make_live(&registry, None).unwrap();
        let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(graph_handle.id(), graph);

        let mut player = AnimationPlayer::new(1);
        {
            let mut create = InputState::<InputSet<CreateNodeInputs>>::new(world);
            let inputs = create.get(world).unwrap();
            let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
            player.initialize_graph(graph_handle, graphs, &inputs);
        }
        let bone = world.spawn(Transform::IDENTITY);
        world.spawn((
            player,
            SkeletonComponent {
                skeleton: skeleton_handle,
                bones: vec![SceneEntityRef::Entity(bone)],
                root: None,
            },
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);

        let x = world
            .get_component_for_entity::<Transform>(bone)
            .unwrap()
            .translation
            .x;
        assert_eq!(
            x, 1.0,
            "the bone must be posed from the graph state updated in the same frame"
        );
    }

    #[derive(Resource)]
    struct Speed;

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct SpeedReader {}

    impl AnimationNode for SpeedReader {
        type State = ();
        type CreateInput = ();
        type UpdateInput = Res<'static, Speed>;
        type EvaluateInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &Res<Speed>, _: &AnimationGraphContext<'_>) {}

        fn evaluate(
            &self,
            _: &mut (),
            _: &(),
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedFrame],
            _: &mut FramePool,
            _: &mut AnimationFrame,
        ) {
        }

        fn reset(&self, _: &mut ()) {}
    }

    fn access_of<M>(world: &mut World, system: impl IntoSystem<(), M>) -> SystemAccess {
        let mut system = system.into_system();
        system.initialize(world);
        let mut access = SystemAccess::default();
        system.fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    #[test]
    fn only_the_walk_whose_input_reads_a_resource_declares_it() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(AnimationPlugin);
        app.register_animation_node::<SpeedReader>();
        let world = app.main_mut().world_mut();

        let writer = access_of(world, |_: ResMut<Speed>| {});
        let update = access_of(world, update_animation_players);
        let evaluate = access_of(world, animate_targets);
        let create = access_of(world, initialize_animation_players);

        assert!(!SystemAccess::are_disjoint(&update, &writer));
        assert!(SystemAccess::are_disjoint(&evaluate, &writer));
        assert!(SystemAccess::are_disjoint(&create, &writer));
    }
}
