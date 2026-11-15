use concerto_app::{App, edit_time::AtPreviewTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{IntoSystemConfig, World, reflect::AppTypeRegistry};
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::skeleton::Skeleton;
use concerto_reflect::{GetTypeRegistration, TypeRegistry};
use glam::{Vec2, Vec3};

use crate::{
    clip::AnimationClip,
    event::{AnimationEvent, ClipEvent},
    graph::AnimationGraph,
    mask::BoneMask,
    node::{
        AdditiveNode, AimOffsetNode, AnimationClipNode, AnimationNode, BlendNode, IkTarget,
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

/// Registers the built-in animation nodes, the built-in clip event, and the handle types a graph stores.
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
    registry.register::<AnimationEvent>();
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
        app.register_derived_asset::<AnimationClip>();
        app.register_asset::<AnimationGraph>();
        app.register_asset::<Skeleton>();
        app.register_inspected_asset::<BoneMask>();
        app.register_inspected_asset::<RetargetRig>();

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
    use concerto_app::{App, plugins::AssetManagerPlugin};
    use concerto_ecs::{
        IntoSystem, Res, ResMut, System, World,
        resource::Resource,
        system::{access::SystemAccess, meta::SystemMetadata},
    };

    use concerto_reflect::Reflect;

    use uuid::Uuid;

    use super::{AnimationAppExt, AnimationPlugin};
    use crate::{
        evaluation::AnimationGraphContext,
        node::{AnimationNode, InputSlots, ReflectAnimationNode},
        pose::{AnimationFrame, EvaluatedFrame, FramePool},
        target::update_animation_players,
    };

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

        fn input_slots(&self) -> InputSlots {
            InputSlots::None
        }

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
