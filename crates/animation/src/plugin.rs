use concerto_app::{edit_time::AtPreviewTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::skeleton::Skeleton;
use concerto_reflect::TypeRegistry;

use crate::{
    clip::AnimationClip,
    graph::AnimationGraph,
    node::{
        AnimationClipNode, BlendNode, PlaceholderNode, ResultNode, blend_space::BlendSpace2DNode,
        state_machine::AnimationStateMachine,
    },
    target::{
        animate_targets, initialize_animation_players, initialize_skeletons,
        update_animation_players,
    },
};

/// Registers the built-in animation nodes and the handle types a graph stores.
pub fn register_animation_types(registry: &mut TypeRegistry) {
    registry.register::<ResultNode>();
    registry.register::<BlendNode>();
    registry.register::<AnimationClipNode>();
    registry.register::<BlendSpace2DNode>();
    registry.register::<AnimationStateMachine>();
    registry.register::<PlaceholderNode>();
    registry.register::<AssetHandle<AnimationClip>>();
    registry.register::<AssetHandle<AnimationGraph>>();
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
        component::scene::SceneEntityRef,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    };
    use concerto_foundation::{
        assets::{asset_server::AssetServer, asset_store::AssetStore},
        time::Time,
        transform::Transform,
    };
    use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
    use concerto_reflect::Reflect;
    use glam::Mat4;
    use uuid::Uuid;

    use super::{AnimationPlugin, register_animation_types};
    use crate::{
        clip::AnimationClip,
        evaluation::AnimationGraphContext,
        graph::AnimationGraph,
        node::{AnimationNode, ReflectAnimationNode},
        player::AnimationPlayer,
        pose::{EvaluatedPose, Pose, PosePool},
    };

    #[derive(Default, Reflect)]
    #[reflect(AnimationNode)]
    struct OrderProbe {}

    impl AnimationNode for OrderProbe {
        type Instance = bool;

        fn create_instance(&self, _context: &AnimationGraphContext<'_>) -> bool {
            false
        }

        fn update(&self, instance: &mut bool, _: f32, _: &AnimationGraphContext<'_>) {
            *instance = true;
        }

        fn evaluate(
            &self,
            instance: &mut bool,
            _: &AnimationGraphContext<'_>,
            _: &[Uuid],
            _: &[EvaluatedPose],
            _: &mut PosePool,
            output: &mut Pose,
        ) {
            if let Some(joint) = output.get_joint_pose_mut(0) {
                joint.translation.x = if *instance { 1.0 } else { 0.0 };
            }
        }

        fn reset(&self, instance: &mut bool) {
            *instance = false;
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
        let server = world.get_resource::<AssetServer>().unwrap().clone();
        let skeleton_handle = server.add(
            Skeleton::new(
                vec![Mat4::IDENTITY],
                vec![None],
                vec!["bone".to_string()],
                vec![JointPose::IDENTITY],
                vec![bone_id],
            )
            .unwrap(),
        );
        let skeleton = Skeleton::new(
            vec![Mat4::IDENTITY],
            vec![None],
            vec!["bone".to_string()],
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
        let graph_handle = server.add(AnimationGraph::default());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(graph_handle.id(), graph);

        let mut player = AnimationPlayer::new(1);
        {
            let clips = world.get_resource::<AssetStore<AnimationClip>>().unwrap();
            let graphs = world.get_resource::<AssetStore<AnimationGraph>>().unwrap();
            player.initialize_graph(graph_handle, clips, graphs);
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
}
