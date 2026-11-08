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
use concerto_procedural_animation::{
    FootLockLeg, FootLockNode, LegLock, LockPhase, ProceduralAnimationPlugin,
};
use concerto_reflect::TypeRegistry;
use glam::{Mat4, Vec3};
use uuid::Uuid;

use crate::{
    FootLockGizmos, ProceduralAnimationGizmosPlugin,
    gizmos::{LockShape, leg_shapes},
};

const EPS: f32 = 1.0e-4;
const KNEE_BEND: f32 = 0.1;
const HALF_LEG: f32 = 0.45;
const CIRCLE_SEGMENTS: usize = 32;
const CROSS_SEGMENTS: usize = 3;

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

fn toe() -> Vec3 {
    Vec3::new(0.1, 2.0 * HALF_LEG * (1.0 - KNEE_BEND.cos()), 0.15)
}

fn shape_segments(shape: &LockShape) -> usize {
    match shape {
        LockShape::Circle { .. } => CIRCLE_SEGMENTS,
        LockShape::Cross { .. } => CROSS_SEGMENTS,
        LockShape::Line { .. } => 1,
    }
}

struct Stage {
    app: App,
    model: Entity,
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
        graph.result_node().with_input(
            FootLockNode::new().with_leg(FootLockLeg::new("foot").with_toe("toe")),
            |_| {},
        );
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

        Self { app, model, player }
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

    fn locked() -> Self {
        let mut stage = Self::new();
        stage.tick();
        assert_eq!(stage.phases(), [LockPhase::Locked]);
        stage
    }

    fn move_model(&mut self, by: Vec3) {
        let model = self.model;
        self.world()
            .get_component_for_entity_mut::<GlobalTransform>(model)
            .unwrap()
            .set_matrix(Mat4::from_translation(by));
    }

    fn show_locks(&mut self, locks: bool) {
        let player = self.player;
        self.world()
            .get_component_for_entity_mut::<FootLockGizmos>(player)
            .unwrap()
            .locks = locks;
    }

    fn legs(&mut self) -> Vec<LegLock> {
        let player = self.player;
        let mut legs = None;
        self.world()
            .resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
                world
                    .get_component_for_entity::<AnimationPlayer>(player)
                    .unwrap()
                    .for_each_state::<FootLockNode>(graphs, |_, state, _| {
                        legs = Some(state.legs().to_vec());
                    });
            });
        legs.expect("the graph holds a foot lock node")
    }

    fn phases(&mut self) -> Vec<LockPhase> {
        self.legs().iter().map(|leg| leg.phase).collect()
    }

    fn segments(&mut self) -> usize {
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }

    fn expected_segments(&mut self) -> usize {
        self.legs()
            .iter()
            .flat_map(leg_shapes)
            .map(|shape| shape_segments(&shape))
            .sum()
    }
}

#[test]
fn a_locked_leg_draws_a_circle_at_its_lock() {
    let mut stage = Stage::locked();

    let legs = stage.legs();
    let lock = legs[0].lock;
    assert!(lock.distance(toe()) < EPS, "{lock:?} vs {:?}", toe());
    let shapes = leg_shapes(&legs[0]);
    assert!(shapes.iter().any(|shape| matches!(
        shape,
        LockShape::Circle { center, .. } if center.distance(lock) < EPS
    )));
    assert_eq!(
        stage.segments(),
        CIRCLE_SEGMENTS + CROSS_SEGMENTS,
        "a circle and a cross"
    );
    assert_eq!(stage.segments(), stage.expected_segments());
}

#[test]
fn a_releasing_leg_draws_a_line_from_where_it_is_shown_to_the_animated_foot() {
    let mut stage = Stage::locked();

    stage.move_model(Vec3::new(0.25, 0.0, 0.0));
    stage.tick();

    assert_eq!(stage.phases(), [LockPhase::Releasing]);
    let leg = stage.legs()[0];
    assert!(leg.shown.distance(leg.animated) > 0.2);
    let shapes = leg_shapes(&leg);
    assert!(shapes.iter().any(|shape| matches!(
        shape,
        LockShape::Line { start, end, .. }
            if start.distance(leg.shown) < EPS && end.distance(leg.animated) < EPS
    )));
    assert_eq!(stage.segments(), CIRCLE_SEGMENTS + 1, "a circle and a line");
    assert_eq!(stage.segments(), stage.expected_segments());
}

#[test]
fn with_locks_off_nothing_is_drawn() {
    let mut stage = Stage::locked();

    stage.show_locks(false);
    stage.tick();

    assert_eq!(stage.phases(), [LockPhase::Locked]);
    assert_eq!(stage.segments(), 0);
}
