use std::{thread::sleep, time::Duration};

use concerto_animation::{
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{AnimationPlugin, register_animation_types},
    pose::JointPose,
};
use concerto_app::{
    App,
    plugins::AssetManagerPlugin,
    schedule_groups::{First, LateUpdate},
};
use concerto_debug_gizmos::{DebugGizmosPlugin, GizmoStorage};
use concerto_ecs::{
    Entity, World,
    component::scene::SceneEntityRef,
    entity::hierarchy::ChildOf,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
};
use concerto_foundation::{
    assets::{AssetId, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_procedural_animation::{FootLockLeg, FootLockNode, ProceduralAnimationPlugin};
use concerto_reflect::TypeRegistry;
use glam::{Mat4, Vec3};
use uuid::Uuid;

use crate::{FootLockGizmos, ProceduralAnimationGizmosPlugin};

const KNEE_BEND: f32 = 0.1;
const HALF_LEG: f32 = 0.45;

fn rig() -> Skeleton {
    let joint = |translation| JointPose {
        translation,
        ..JointPose::IDENTITY
    };
    Skeleton::new(
        vec![Mat4::IDENTITY; 5],
        vec![None, Some(0), Some(1), Some(2), Some(3)],
        vec![
            "pelvis".into(),
            "hip".into(),
            "knee".into(),
            "foot".into(),
            "toe".into(),
        ],
        vec![
            joint(Vec3::new(0.0, 2.0 * HALF_LEG, 0.0)),
            joint(Vec3::new(0.1, 0.0, 0.0)),
            joint(Vec3::new(
                0.0,
                -HALF_LEG * KNEE_BEND.cos(),
                HALF_LEG * KNEE_BEND.sin(),
            )),
            joint(Vec3::new(
                0.0,
                -HALF_LEG * KNEE_BEND.cos(),
                -HALF_LEG * KNEE_BEND.sin(),
            )),
            joint(Vec3::new(0.0, 0.0, 0.15)),
        ],
        (0..5).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

struct Stage {
    app: App,
    player: Entity,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.register_plugin(AnimationPlugin);
        app.register_plugin(ProceduralAnimationPlugin);
        app.register_plugin(DebugGizmosPlugin);
        app.register_plugin(ProceduralAnimationGizmosPlugin);

        let skeleton = rig();
        let bone_count = skeleton.bone_count();
        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(skeleton_handle.id(), skeleton);

        let mut registry = TypeRegistry::default();
        register_animation_types(&mut registry);
        registry.register::<FootLockNode>();
        let mut graph = AnimationGraph::new();
        graph
            .result_node()
            .add_input(FootLockNode::new().with_leg(FootLockLeg::new("foot").with_toe("toe")));
        graph.make_live(&registry, None).unwrap();
        let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(graph_handle.id(), graph);

        let bones: Vec<Entity> = (0..bone_count)
            .map(|_| world.spawn(Transform::IDENTITY))
            .collect();
        let model = world.spawn(GlobalTransform::new(Mat4::IDENTITY));
        world.insert(ChildOf::new(model), bones[0]);
        let player = world.spawn((
            FootLockGizmos::default(),
            AnimationPlayer::new(bone_count),
            AnimationHandleComponent::new(graph_handle),
            SkeletonComponent::new(
                skeleton_handle,
                bones
                    .iter()
                    .map(|bone| SceneEntityRef::Entity(*bone))
                    .collect(),
                None,
            ),
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);
        world.run_schedule(LateUpdate);

        Self { app, player }
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn tick(&mut self) {
        sleep(Duration::from_millis(2));
        let world = self.world();
        world.get_resource_mut::<Time>().unwrap().update();
        world.run_schedule(First);
        world.run_schedule(LateUpdate);
    }

    fn show_locks(&mut self, locks: bool) {
        let player = self.player;
        self.world()
            .get_component_for_entity_mut::<FootLockGizmos>(player)
            .unwrap()
            .locks = locks;
    }

    fn segments(&mut self) -> usize {
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }
}

#[test]
fn a_leg_lock_is_drawn_while_locks_are_on_and_nothing_when_off() {
    let mut stage = Stage::new();
    stage.tick();
    assert!(stage.segments() > 0);

    stage.show_locks(false);
    stage.tick();

    assert_eq!(stage.segments(), 0);
}
