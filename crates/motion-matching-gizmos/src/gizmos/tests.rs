use std::{any::TypeId, f32::consts::PI, thread::sleep, time::Duration};

use concerto_animation::{
    animate_targets,
    clip::{AnimationChanelOutput, AnimationChannel},
    graph::AnimationGraph,
    node::BlendNode,
    player::AnimationHandleComponent,
    plugin::{AnimationPlugin, register_animation_types},
    pose::Pose,
    update_animation_players,
};
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    schedule_groups::First,
};
use concerto_debug_gizmos::GizmoStorage;
use concerto_ecs::{
    Entity, World,
    component::scene::SceneEntityRef,
    reflect::AppTypeRegistry,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
};
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_mesh::{JointPose, SkeletonComponent};
use concerto_motion_matching::{
    BonePositions, BoneVelocities, JointAxis, LastQuery, MirrorAxis, MotionDatabaseDefinition,
    MotionEntry, MotionMatchingInstance, SimulationBoneSettings, StoredChannel,
    TrajectoryDirections, TrajectoryPositions, TrajectorySample, TrajectorySettings, bake,
    run_motion_matching,
};
use concerto_reflect::{TypeRegistry, to_value};
use glam::{Mat4, Quat};
use uuid::Uuid;

use super::*;
use crate::placed_shape;

const NEAR: f32 = 1e-3;
const WALK_FRAME: usize = 20;

type Toggle = fn(&mut MotionMatchingGizmos);

const SIMULATION: Toggle = |shown| shown.simulation = true;
const QUERY: Toggle = |shown| shown.query = true;
const MATCHED: Toggle = |shown| shown.matched = true;
const HISTORY: Toggle = |shown| shown.history = true;

fn skeleton() -> Skeleton {
    let joint = |translation| JointPose {
        translation,
        ..JointPose::IDENTITY
    };
    Skeleton::new(
        vec![Mat4::IDENTITY; 4],
        vec![None, Some(0), Some(1), Some(1)],
        vec!["Root".into(), "Hips".into(), "Spine2".into(), "Foot".into()],
        vec![
            JointPose::IDENTITY,
            joint(Vec3::Y),
            joint(Vec3::new(0.0, 0.3, 0.0)),
            joint(Vec3::new(0.2, -1.0, 0.0)),
        ],
        (1..=4).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

fn walk() -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(1),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Translation(vec![Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)]),
        ),
    );
    clip.add_channel(
        Uuid::from_u128(4),
        AnimationChannel::new(
            vec![0.0, 2.0],
            AnimationChanelOutput::Translation(vec![
                Vec3::new(0.2, -1.0, 0.5),
                Vec3::new(0.2, -1.0, -0.5),
            ]),
        ),
    );
    clip
}

fn turn() -> AnimationClip {
    let times: Vec<f32> = (0..5).map(|key| key as f32 * 0.5).collect();
    let rotations = times
        .iter()
        .map(|time| Quat::from_rotation_y(PI + 1.5 * time))
        .collect();
    let mut clip = AnimationClip::default();
    clip.add_channel(
        Uuid::from_u128(2),
        AnimationChannel::new(times, AnimationChanelOutput::Rotation(rotations)),
    );
    clip
}

fn foot() -> Vec<concerto_mesh::JointName> {
    vec!["Foot".into()]
}

fn stored<T: concerto_reflect::Reflect>(type_name: &str, channel: &T) -> StoredChannel {
    StoredChannel {
        type_name: type_name.into(),
        value: to_value(channel).unwrap(),
    }
}

fn every_channel() -> Vec<StoredChannel> {
    vec![
        stored(
            "BonePositions",
            &BonePositions {
                bones: foot(),
                weight: 1.0,
            },
        ),
        stored(
            "BoneVelocities",
            &BoneVelocities {
                bones: foot(),
                weight: 1.0,
            },
        ),
        stored(
            "TrajectoryPositions",
            &TrajectoryPositions {
                samples: vec![0, 1],
                weight: 1.0,
            },
        ),
        stored(
            "TrajectoryDirections",
            &TrajectoryDirections {
                samples: vec![0, 1],
                weight: 1.0,
            },
        ),
    ]
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<BonePositions>();
    registry.register::<BoneVelocities>();
    registry.register::<TrajectoryPositions>();
    registry.register::<TrajectoryDirections>();
    registry
}

fn baked(
    channels: Vec<StoredChannel>,
    skeleton_handle: AssetHandle<Skeleton>,
    [walk_handle, turn_handle]: [AssetHandle<AnimationClip>; 2],
) -> MotionDatabase {
    let definition = MotionDatabaseDefinition {
        skeleton: skeleton_handle,
        entries: vec![
            MotionEntry {
                clip: walk_handle,
                range: None,
                mirrored: false,
            },
            MotionEntry {
                clip: turn_handle,
                range: None,
                mirrored: false,
            },
        ],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            facing_axis: JointAxis::Z,
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        trajectory: TrajectorySettings {
            sample_dt: 0.1,
            sample_count: 2,
        },
        channels,
        mirror_axis: MirrorAxis::X,
    };
    bake(&definition, &skeleton(), &[&walk(), &turn()], &registry()).unwrap()
}

fn read_instance<R>(
    world: &mut World,
    player: Entity,
    read: impl FnOnce(&mut MotionMatchingInstance) -> R,
) -> R {
    let mut read = Some(read);
    let mut result = None;
    world.resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
        world
            .get_component_for_entity_mut::<AnimationPlayer>(player)
            .unwrap()
            .for_each_instance_mut::<MotionMatchingNode>(graphs, |_, instance, _| {
                if let Some(read) = read.take() {
                    result = Some(read(instance));
                }
            });
    });
    result.expect("the graph holds a motion matching node")
}

struct Stage {
    app: App,
    character: Entity,
    capsule: Entity,
    player: Entity,
    database_id: AssetId,
    skeleton_handle: AssetHandle<Skeleton>,
    clip_handles: [AssetHandle<AnimationClip>; 2],
    database: MotionDatabase,
    skeleton: Skeleton,
    walk: AnimationClip,
    turn: AnimationClip,
}

impl Stage {
    fn new() -> Self {
        Self::with_nodes(1)
    }

    fn with_nodes(nodes: usize) -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.insert_resource(Time::new());
        app.register_plugin(AnimationPlugin);
        app.register_asset::<MotionDatabase>();
        app.register_type::<MotionMatchingNode>()
            .register_type::<BonePositions>()
            .register_type::<BoneVelocities>()
            .register_type::<TrajectoryPositions>()
            .register_type::<TrajectoryDirections>();
        app.add_system(
            LateUpdate,
            run_motion_matching
                .after(update_animation_players)
                .before(animate_targets),
        );
        app.register_plugin(DebugGizmosPlugin);
        app.register_plugin(MotionMatchingGizmosPlugin);

        let skeleton_handle = AssetHandle::<Skeleton>::weak(AssetId::new());
        let clip_handles = [
            AssetHandle::<AnimationClip>::weak(AssetId::new()),
            AssetHandle::<AnimationClip>::weak(AssetId::new()),
        ];
        let database = baked(
            every_channel(),
            skeleton_handle.clone(),
            clip_handles.clone(),
        );
        let database_handle = AssetHandle::<MotionDatabase>::weak(AssetId::new());

        let world = app.main_mut().world_mut();
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(skeleton_handle.id(), skeleton());
        {
            let clips = world
                .get_resource_mut::<AssetStore<AnimationClip>>()
                .unwrap();
            clips.insert(clip_handles[0].id(), walk());
            clips.insert(clip_handles[1].id(), turn());
        }
        world
            .get_resource_mut::<AssetStore<MotionDatabase>>()
            .unwrap()
            .insert(database_handle.id(), database.clone());

        let mut graph_registry = TypeRegistry::default();
        register_animation_types(&mut graph_registry);
        graph_registry.register::<MotionMatchingNode>();
        graph_registry.register::<AssetHandle<MotionDatabase>>();
        let mut graph = AnimationGraph::new();
        let blend = graph.result_node().index();
        let blend = graph.add_node(BlendNode {}, blend).index();
        for _ in 0..nodes {
            graph.add_node(
                MotionMatchingNode {
                    database: database_handle.clone(),
                    ..Default::default()
                },
                blend,
            );
        }
        graph.make_live(&graph_registry, None).unwrap();
        let graph_handle = AssetHandle::<AnimationGraph>::weak(AssetId::new());
        world
            .get_resource_mut::<AssetStore<AnimationGraph>>()
            .unwrap()
            .insert(graph_handle.id(), graph);

        let bones: Vec<SceneEntityRef> = (0..4)
            .map(|_| SceneEntityRef::Entity(world.spawn(Transform::IDENTITY)))
            .collect();
        let player = world.spawn((
            AnimationPlayer::new(4),
            AnimationHandleComponent::new(graph_handle),
            SkeletonComponent::new(skeleton_handle.clone(), bones, None),
        ));
        let capsule = world.spawn((
            CharacterSimulation::default(),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(2.0, 0.0, 1.0))),
        ));
        let character = world.spawn((
            MotionMatchingGizmos::default(),
            CharacterLink {
                capsule: SceneEntityRef::Entity(capsule),
                skeleton: SceneEntityRef::Entity(player),
            },
            GlobalTransform::new(Mat4::from_translation(Vec3::new(2.2, 0.0, 1.0))),
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);
        world.run_schedule(LateUpdate);

        Self {
            app,
            character,
            capsule,
            player,
            database_id: database_handle.id(),
            skeleton_handle,
            clip_handles,
            database,
            skeleton: skeleton(),
            walk: walk(),
            turn: turn(),
        }
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn only(&mut self, toggles: &[Toggle]) {
        let character = self.character;
        let shown = self
            .world()
            .get_component_for_entity_mut::<MotionMatchingGizmos>(character)
            .unwrap();
        shown.simulation = false;
        shown.query = false;
        shown.matched = false;
        shown.history = false;
        for toggle in toggles {
            toggle(shown);
        }
    }

    fn time(&mut self) -> &mut Time {
        self.world().get_resource_mut::<Time>().unwrap()
    }

    fn tick(&mut self) {
        sleep(Duration::from_millis(2));
        self.time().update();
        let world = self.world();
        world.run_schedule(First);
        world.run_schedule(LateUpdate);
    }

    fn segments(&mut self) -> usize {
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }

    fn trail_length(&mut self) -> usize {
        let character = self.character;
        self.world()
            .get_component_for_entity::<MotionMatchingGizmos>(character)
            .unwrap()
            .trail
            .len()
    }

    fn seat(&mut self, frame: usize) {
        let clips = [&self.walk, &self.turn];
        let pose = self.database.pose(frame, &self.skeleton, &clips).unwrap();
        read_instance(self.app.main_mut().world_mut(), self.player, |instance| {
            instance.frame = frame;
            instance.pose = Pose::from_joints(pose);
        });
    }

    fn searched(&mut self, frame: usize, simulation: Transform, trajectory: Vec<TrajectorySample>) {
        let (database, skeleton) = (&self.database, &self.skeleton);
        read_instance(self.app.main_mut().world_mut(), self.player, |instance| {
            let features = instance
                .query
                .as_ref()
                .unwrap()
                .raw(database, skeleton, frame, &simulation, &trajectory)
                .unwrap();
            instance.last_query = Some(LastQuery {
                features,
                frame,
                simulation,
                trajectory,
                best_cost: None,
            });
        });
    }

    fn searched_at_its_own_frame(&mut self, frame: usize) {
        let simulation = self.database.simulation(frame);
        let trajectory = self.database.trajectory_samples(frame);
        self.searched(frame, simulation, trajectory);
    }

    fn matched(&mut self, bone: &Transform) -> Option<Vec<Vec<FeatureShape>>> {
        let (database, skeleton) = (&self.database, &self.skeleton);
        read_instance(self.app.main_mut().world_mut(), self.player, |instance| {
            matched_shapes(instance, database, skeleton, bone)
        })
    }

    fn query(&mut self, bone: &Transform) -> Option<Vec<Vec<FeatureShape>>> {
        let (database, skeleton) = (&self.database, &self.skeleton);
        let clips = [&self.walk, &self.turn];
        read_instance(self.app.main_mut().world_mut(), self.player, |instance| {
            query_shapes(instance, database, skeleton, &clips, bone)
        })
    }

    fn replace_database(&mut self, channels: Vec<StoredChannel>) {
        let database = baked(
            channels,
            self.skeleton_handle.clone(),
            self.clip_handles.clone(),
        );
        let id = self.database_id;
        self.world()
            .get_resource_mut::<AssetStore<MotionDatabase>>()
            .unwrap()
            .insert(id, database.clone());
        self.database = database;
    }
}

fn bone() -> Transform {
    Transform::from_translation_rotation(Vec3::new(1.0, 0.0, -3.0), Quat::from_rotation_y(0.7))
}

fn near(first: Vec3, second: Vec3) -> bool {
    first.distance(second) < NEAR
}

fn same_shape(first: &FeatureShape, second: &FeatureShape) -> bool {
    match (first, second) {
        (
            FeatureShape::Ring { center, radius },
            FeatureShape::Ring {
                center: other_center,
                radius: other_radius,
            },
        ) => near(*center, *other_center) && radius == other_radius,
        (
            FeatureShape::Line { start, end },
            FeatureShape::Line {
                start: other_start,
                end: other_end,
            },
        )
        | (
            FeatureShape::Arrow { start, end },
            FeatureShape::Arrow {
                start: other_start,
                end: other_end,
            },
        ) => near(*start, *other_start) && near(*end, *other_end),
        _ => false,
    }
}

fn assert_same(first: &[Vec<FeatureShape>], second: &[Vec<FeatureShape>]) {
    assert_eq!(first.len(), second.len());
    for (first, second) in first.iter().zip(second) {
        assert_eq!(first.len(), second.len(), "{first:?} vs {second:?}");
        for (first, second) in first.iter().zip(second) {
            assert!(same_shape(first, second), "{first:?} vs {second:?}");
        }
    }
}

#[test]
fn an_app_without_the_renderer_finishes_building_and_has_no_gizmos() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(AnimationPlugin);
    app.register_asset::<MotionDatabase>();
    app.register_plugin(MotionMatchingGizmosPlugin);

    app.finish_plugin_build();

    assert!(!app.is_plugin_added::<DebugGizmosPlugin>());
    assert!(app.get_resource::<GizmoStorage>().is_none());
    let registry = app.get_resource::<AppTypeRegistry>().unwrap();
    assert!(
        registry
            .read()
            .get(TypeId::of::<MotionMatchingGizmos>())
            .is_some()
    );
}

#[test]
fn each_toggle_alone_adds_segments() {
    let mut stage = Stage::new();
    stage.searched_at_its_own_frame(0);

    for (name, toggle) in [
        ("simulation", SIMULATION),
        ("query", QUERY),
        ("matched", MATCHED),
        ("history", HISTORY),
    ] {
        stage.only(&[toggle]);
        stage.tick();
        stage.tick();

        assert!(stage.segments() > 0, "{name} drew nothing");
    }
}

#[test]
fn with_every_toggle_off_nothing_is_drawn() {
    let mut stage = Stage::new();
    stage.searched_at_its_own_frame(0);
    stage.only(&[]);

    stage.tick();
    stage.tick();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_character_whose_capsule_is_gone_draws_nothing() {
    let mut stage = Stage::new();
    let capsule = stage.capsule;
    stage.world().despawn(capsule);

    stage.tick();
    stage.tick();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_character_whose_skeleton_is_gone_draws_nothing() {
    let mut stage = Stage::new();
    let player = stage.player;
    stage.world().despawn(player);

    stage.tick();
    stage.tick();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_link_with_an_unresolved_field_draws_nothing() {
    for unresolve in [
        (|link: &mut CharacterLink| link.capsule = SceneEntityRef::default())
            as fn(&mut CharacterLink),
        |link: &mut CharacterLink| link.skeleton = SceneEntityRef::default(),
    ] {
        let mut stage = Stage::new();
        let character = stage.character;
        unresolve(
            stage
                .world()
                .get_component_for_entity_mut::<CharacterLink>(character)
                .unwrap(),
        );

        stage.tick();
        stage.tick();

        assert_eq!(stage.segments(), 0);
    }
}

#[test]
fn a_missing_database_skeleton_or_clip_draws_no_features() {
    let wipes: [fn(&mut World); 3] = [
        |world| world.insert_resource(AssetStore::<MotionDatabase>::new()),
        |world| world.insert_resource(AssetStore::<Skeleton>::new()),
        |world| world.insert_resource(AssetStore::<AnimationClip>::new()),
    ];
    for wipe in wipes {
        let mut stage = Stage::new();
        stage.searched_at_its_own_frame(0);
        stage.only(&[QUERY, MATCHED]);
        wipe(stage.world());

        stage.tick();

        assert_eq!(stage.segments(), 0);
    }
}

#[test]
fn with_no_search_yet_the_query_draws_nothing() {
    let mut stage = Stage::new();
    stage.only(&[QUERY]);

    stage.tick();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_failed_instance_draws_no_features() {
    let mut stage = Stage::new();
    stage.searched_at_its_own_frame(0);
    stage.only(&[QUERY, MATCHED]);
    let player = stage.player;
    read_instance(stage.world(), player, |instance| instance.failed = true);

    stage.tick();

    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_graph_with_several_nodes_draws_one() {
    let mut one = Stage::with_nodes(1);
    let mut two = Stage::with_nodes(2);
    one.only(&[MATCHED]);
    two.only(&[MATCHED]);

    one.tick();
    two.tick();

    assert!(one.segments() > 0);
    assert_eq!(one.segments(), two.segments());
}

#[test]
fn matched_and_query_agree_when_the_search_kept_the_frame() {
    let mut stage = Stage::new();
    stage.seat(WALK_FRAME);
    stage.searched_at_its_own_frame(WALK_FRAME);

    let matched = stage.matched(&bone()).unwrap();
    let query = stage.query(&bone()).unwrap();

    assert!(matched.iter().all(|shapes| !shapes.is_empty()));
    assert_same(&matched, &query);
}

#[test]
fn shapes_are_placed_at_the_bone() {
    let mut stage = Stage::new();
    stage.seat(WALK_FRAME);
    stage.searched_at_its_own_frame(WALK_FRAME);
    let bone = bone();

    let at_origin = stage.matched(&Transform::IDENTITY).unwrap();
    let placed: Vec<Vec<FeatureShape>> = at_origin
        .into_iter()
        .map(|shapes| {
            shapes
                .into_iter()
                .map(|shape| placed_shape(&bone, shape))
                .collect()
        })
        .collect();

    assert_same(&stage.matched(&bone).unwrap(), &placed);
    assert_same(&stage.query(&bone).unwrap(), &placed);
}

#[test]
fn after_a_transition_the_querys_arrow_starts_at_its_own_ring() {
    let mut stage = Stage::new();
    stage.seat(WALK_FRAME);
    let simulation =
        Transform::from_translation_rotation(Vec3::new(3.0, 0.0, -4.0), Quat::from_rotation_y(1.1));
    let trajectory = stage.database.trajectory_samples(WALK_FRAME);
    stage.searched(WALK_FRAME, simulation, trajectory);
    stage.seat(WALK_FRAME + 30);

    let query = stage.query(&bone()).unwrap();

    let FeatureShape::Ring { center, .. } = query[0][0] else {
        panic!("a bone's position is a ring");
    };
    let FeatureShape::Arrow { start, .. } = query[1][0] else {
        panic!("a bone's velocity is an arrow");
    };
    assert!(near(start, center), "{start} vs {center}");
}

#[test]
fn a_frame_the_database_did_not_bake_gives_no_shapes() {
    let mut stage = Stage::new();
    stage.searched_at_its_own_frame(0);
    let past = stage.database.frame_count() + 5;
    let player = stage.player;
    read_instance(stage.world(), player, |instance| {
        instance.frame = past;
        instance.last_query.as_mut().unwrap().frame = past;
    });

    assert_eq!(stage.matched(&bone()), None);
    assert_eq!(stage.query(&bone()), None);
}

#[test]
fn a_replaced_database_of_another_length_draws_no_features() {
    let mut stage = Stage::new();
    stage.searched_at_its_own_frame(0);
    stage.only(&[QUERY, MATCHED]);

    stage.replace_database(vec![every_channel().remove(0)]);

    assert_eq!(stage.matched(&bone()), None);
    assert_eq!(stage.query(&bone()), None);
    stage.tick();
    assert_eq!(stage.segments(), 0);
}

#[test]
fn history_forgets_positions_older_than_a_second() {
    let mut stage = Stage::new();
    stage.only(&[HISTORY]);
    for _ in 0..3 {
        stage.tick();
    }
    assert_eq!(stage.segments(), 2);

    stage.time().set_time_scale(1000.0);
    stage.tick();

    assert_eq!(stage.trail_length(), 1);
    assert_eq!(stage.segments(), 0);
}

#[test]
fn a_trail_that_was_hidden_shows_no_position_older_than_a_second() {
    let mut stage = Stage::new();
    stage.only(&[HISTORY]);
    stage.tick();
    stage.tick();

    stage.only(&[]);
    stage.time().set_time_scale(1000.0);
    stage.tick();
    stage.time().set_time_scale(1.0);
    stage.only(&[HISTORY]);
    stage.tick();

    assert_eq!(stage.trail_length(), 2);
    assert_eq!(stage.segments(), 1);
}

#[test]
fn a_paused_game_adds_nothing_to_the_trail() {
    let mut stage = Stage::new();
    stage.only(&[HISTORY]);
    stage.tick();
    stage.tick();

    stage.time().set_time_scale(0.0);
    stage.tick();
    stage.tick();

    assert_eq!(stage.trail_length(), 2);
    assert_eq!(stage.segments(), 1);
}
