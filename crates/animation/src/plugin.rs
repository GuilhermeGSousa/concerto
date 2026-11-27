use std::any::TypeId;

use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    plugins::Plugin,
    schedule_groups::{LateFixedUpdate, LateUpdate},
};
use concerto_ecs::{IntoSystemConfig, World, reflect::AppTypeRegistry};
use concerto_foundation::{
    assets::handle::AssetHandle,
    transform::systems::{propagate_global_transforms, update_simple_entities},
};
use concerto_mesh::skeleton::Skeleton;
use concerto_reflect::{GetTypeRegistration, TypeRegistry};
use glam::{Vec2, Vec3};
use log::warn;

use crate::{
    clip::AnimationClip,
    drive::{AnimationDriveSet, drive_animation_players},
    event::{AnimationEvent, ClipEvent},
    graph::AnimationGraph,
    mask::BoneMask,
    message::{
        AnimationMessage, ClipEnded, ReflectAnimationMessage, queue_message,
        queue_reflected_message,
    },
    node::{
        AdditiveNode, AimOffsetNode, AnimationClipNode, BlendNode, EditorAnimationNode, IkTarget,
        LayerNode, PlaceholderNode, ResultNode, TwoBoneIkNode, blend_space::BlendSpace2DNode,
        register_node_inputs, state_machine::AnimationStateMachine,
    },
    param::Param,
    retarget::{RetargetNode, RetargetRig},
    target::{
        animate_targets, initialize_animation_players, initialize_skeletons,
        update_animation_players,
    },
};

/// Registers the built-in animation nodes, the built-in clip event and messages, and the handle types a graph stores.
pub fn register_animation_types(registry: &mut TypeRegistry) {
    registry.register::<ResultNode>();
    registry.register::<BlendNode>();
    registry.register::<TwoBoneIkNode>();
    registry.register::<LayerNode>();
    registry.register::<AdditiveNode>();
    registry.register::<AimOffsetNode>();
    registry.register::<AnimationClipNode>();
    registry.register::<BlendSpace2DNode>();
    registry.register::<AnimationStateMachine>();
    registry.register::<RetargetNode>();
    registry.register::<PlaceholderNode>();
    let mut event = AnimationEvent::get_type_registration();
    event.insert(ReflectAnimationMessage);
    registry.add(event);
    let mut ended = ClipEnded::get_type_registration();
    ended.insert(ReflectAnimationMessage);
    registry.add(ended);
    registry.register::<AssetHandle<AnimationClip>>();
    registry.register::<AssetHandle<AnimationGraph>>();
    registry.register::<AssetHandle<BoneMask>>();
    registry.register::<AssetHandle<RetargetRig>>();
    registry.register::<AssetHandle<Skeleton>>();
    registry.register::<Param<bool>>();
    registry.register::<Param<f32>>();
    registry.register::<Param<Vec2>>();
    registry.register::<Param<Vec3>>();
    registry.register::<Param<IkTarget>>();
    registry.register::<IkTarget>();
}

/// Registers the inputs of the built-in animation nodes with the graph walks of `world`.
pub fn register_animation_node_inputs(world: &mut World) {
    register_node_inputs::<ResultNode>(world);
    register_node_inputs::<BlendNode>(world);
    register_node_inputs::<TwoBoneIkNode>(world);
    register_node_inputs::<LayerNode>(world);
    register_node_inputs::<AdditiveNode>(world);
    register_node_inputs::<AimOffsetNode>(world);
    register_node_inputs::<AnimationClipNode>(world);
    register_node_inputs::<BlendSpace2DNode>(world);
    register_node_inputs::<AnimationStateMachine>(world);
    register_node_inputs::<RetargetNode>(world);
    register_node_inputs::<PlaceholderNode>(world);
}

/// Adds an animation node type to an `App`.
pub trait AnimationAppExt {
    /// Registers `N` with the type registry and its inputs with the graph walks; call it in a plugin's `build`.
    fn register_animation_node<N: EditorAnimationNode + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self;

    /// Registers `S` with the type registry, so clips that store it make it live on load.
    fn register_clip_event<S: ClipEvent + GetTypeRegistration>(&mut self) -> &mut Self;

    /// Registers `M` with the type registry as an animation message, so `OnMessage` triggers
    /// stored with its path fire, and listens for
    /// [`SendAnimationMessage<M>`](crate::message::SendAnimationMessage) to queue it on a
    /// character's player; call it before anything else registers `M`.
    fn register_animation_message<M: AnimationMessage + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self;
}

impl AnimationAppExt for App {
    fn register_animation_node<N: EditorAnimationNode + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self {
        self.register_type::<N>();
        register_node_inputs::<N>(self.main_mut().world_mut());
        self
    }

    fn register_clip_event<S: ClipEvent + GetTypeRegistration>(&mut self) -> &mut Self {
        self.register_type::<S>();
        self
    }

    fn register_animation_message<M: AnimationMessage + GetTypeRegistration>(
        &mut self,
    ) -> &mut Self {
        let registry = self.main_mut().world_mut().type_registry();
        let mut registry = registry.write();
        match registry.get(TypeId::of::<M>()) {
            Some(registration) if registration.data::<ReflectAnimationMessage>().is_none() => {
                warn!(
                    "{} was registered before register_animation_message, so OnMessage triggers never fire on it",
                    std::any::type_name::<M>()
                );
            }
            Some(_) => {}
            None => {
                let mut registration = M::get_type_registration();
                registration.insert(ReflectAnimationMessage);
                registry.add(registration);
                M::register_dependencies(&mut registry);
            }
        }
        drop(registry);
        self.add_listener(queue_message::<M>.at_preview_time());
        self
    }
}

/// Registers the animation assets, node and event types, and the systems that play every
/// [`AnimationPlayer`](crate::player::AnimationPlayer) in `LateUpdate` and drive it in
/// `LateFixedUpdate`, at preview time too.
pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_derived_asset::<AnimationClip>();
        app.register_asset::<AnimationGraph>();
        app.register_asset::<Skeleton>();
        app.register_inspected_asset::<BoneMask>();
        app.register_inspected_asset::<RetargetRig>();

        if let Some(registry) = app.get_resource::<AppTypeRegistry>() {
            register_animation_types(&mut registry.write());
        }
        register_animation_node_inputs(app.main_mut().world_mut());
        app.register_animation_message::<ClipEnded>();
        app.register_animation_message::<AnimationEvent>();
        app.add_listener(queue_reflected_message.at_preview_time());

        app.add_system(
            LateFixedUpdate,
            drive_animation_players
                .in_set(AnimationDriveSet)
                .at_preview_time(),
        );
        app.add_system(LateUpdate, initialize_skeletons.at_preview_time())
            .add_system(
                LateUpdate,
                animate_targets
                    .after(update_animation_players)
                    .before(update_simple_entities)
                    .before(propagate_global_transforms)
                    .at_preview_time(),
            )
            .add_system(
                LateUpdate,
                initialize_animation_players
                    .after(initialize_skeletons)
                    .at_preview_time(),
            )
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
        node::{AnimationNode, CreateNodeInputs, EditorAnimationNode, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        target::update_animation_players,
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct OrderProbe {}

    impl AnimationNode for OrderProbe {
        type State = bool;
        type CreateInput = ();
        type UpdateInput = ();
        type EvaluateInput = ();
        type DriveInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> bool {
            false
        }

        fn update(&self, state: &mut bool, _: f32, _: &(), _: &mut AnimationGraphContext<'_>) {
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

    impl EditorAnimationNode for OrderProbe {}

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

        let stand_in = world.spawn(());
        let mut player = AnimationPlayer::new(1);
        {
            let mut create = InputState::<InputSet<CreateNodeInputs>>::new(world);
            let inputs = create.get(world).unwrap();
            let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
            player.initialize_graph(stand_in, graph_handle, graphs, &inputs);
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
        type DriveInput = ();

        fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) {}

        fn update(&self, _: &mut (), _: f32, _: &Res<Speed>, _: &mut AnimationGraphContext<'_>) {}

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

    impl EditorAnimationNode for SpeedReader {}

    fn access_of<M>(world: &mut World, system: impl IntoSystem<(), M>) -> SystemAccess {
        let mut system = system.into_system();
        system.initialize(world);
        let mut access = SystemAccess::default();
        system.fill_access(&mut SystemMetadata::default(), &mut access);
        access
    }

    #[test]
    fn the_update_walk_declares_the_resource_a_node_reads_in_update() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(AnimationPlugin);
        app.register_animation_node::<SpeedReader>();
        let world = app.main_mut().world_mut();

        let writer = access_of(world, |_: ResMut<Speed>| {});
        let update = access_of(world, update_animation_players);

        assert!(!SystemAccess::are_disjoint(&update, &writer));
    }
}
