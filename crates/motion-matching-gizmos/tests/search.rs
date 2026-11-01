use std::{
    f32::consts::{FRAC_PI_2, PI},
    thread::sleep,
    time::Duration,
};

use concerto_animation::{
    clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
    graph::AnimationGraph,
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::AnimationPlugin,
};
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_debug_gizmos::DebugGizmosPlugin;
use concerto_ecs::{
    Entity, World,
    component::scene::SceneEntityRef,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
};
use concerto_foundation::{
    assets::{asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{JointName, JointPose, Skeleton, SkeletonComponent};
use concerto_motion_matching::{
    BonePositions, BoneVelocities, CharacterLink, CharacterSimulation, DesiredMotion, FeatureShape,
    LastQuery, MotionDatabase, MotionDatabaseDefinition, MotionEntry, MotionMatchingNode,
    MotionMatchingPlugin, SimulationBoneSettings, StoredChannel, TrajectoryDirections,
    TrajectoryPositions, TrajectorySettings, bake,
};
use concerto_motion_matching_gizmos::{
    MotionMatchingGizmos, MotionMatchingGizmosPlugin, matched_shapes, query_shapes,
};
use concerto_physics::{
    collider::{Collider, ColliderOffset},
    plugin::PhysicsPlugin,
    rigid_body::{AllowedDofs, MotionType, RigidBody},
};
use concerto_reflect::{Reflect, TypeRegistry, to_value};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

const NEAR: f32 = 1e-3;
const FAR_FROM_THE_DATABASE: f32 = 5.0;
const START: Vec3 = Vec3::new(6.0, 0.0, 8.0);
const FRAMES: usize = 5;
const FRAME: Duration = Duration::from_millis(34);

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

fn foot() -> Vec<JointName> {
    vec!["Foot".into()]
}

fn samples() -> Vec<usize> {
    (0..TrajectorySettings::default().sample_count).collect()
}

fn stored<T: Reflect>(type_name: &str, channel: &T) -> StoredChannel {
    StoredChannel {
        type_name: type_name.into(),
        value: to_value(channel).unwrap(),
    }
}

fn channels() -> Vec<StoredChannel> {
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
                samples: samples(),
                weight: 1.0,
            },
        ),
        stored(
            "TrajectoryDirections",
            &TrajectoryDirections {
                samples: samples(),
                weight: 1.0,
            },
        ),
    ]
}

fn baked(
    skeleton_handle: AssetHandle<Skeleton>,
    [walk_handle, turn_handle]: [AssetHandle<AnimationClip>; 2],
) -> MotionDatabase {
    let definition = MotionDatabaseDefinition {
        skeleton: skeleton_handle,
        entries: vec![
            MotionEntry {
                clip: walk_handle,
                range: None,
            },
            MotionEntry {
                clip: turn_handle,
                range: None,
            },
        ],
        rate: 30.0,
        speed: 1.0,
        simulation: SimulationBoneSettings {
            facing_axis: Vec3::Z,
            position_window: 5,
            facing_window: 5,
            ..Default::default()
        },
        trajectory: TrajectorySettings::default(),
        channels: channels(),
    };
    let mut registry = TypeRegistry::default();
    registry.register::<BonePositions>();
    registry.register::<BoneVelocities>();
    registry.register::<TrajectoryPositions>();
    registry.register::<TrajectoryDirections>();
    bake(&definition, &skeleton(), &[&walk(), &turn()], &registry).unwrap()
}

fn spawn_capsule(world: &mut World) -> Entity {
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(START.x, -0.5, START.z)),
    ));
    let collider = Collider::capsule(0.5, 0.3);
    let offset = ColliderOffset::bottom_origin(&collider);
    world.spawn((
        RigidBody {
            density: 1000.0,
            allowed_dofs: AllowedDofs::TRANSLATION | AllowedDofs::ROTATION_Y,
            motion_type: MotionType::Dynamic,
        },
        CharacterSimulation::default(),
        DesiredMotion {
            velocity: Vec3::new(1.5, 0.0, 0.0),
            rotation: Quat::from_rotation_y(FRAC_PI_2),
        },
        collider,
        offset,
        Transform::from_translation(START),
    ))
}

struct Drawn {
    last_query: LastQuery,
    frame: usize,
    bone: Transform,
    query: Vec<Vec<FeatureShape>>,
    matched: Vec<Vec<FeatureShape>>,
}

struct Stage {
    app: App,
    capsule: Entity,
    player: Entity,
    database: MotionDatabase,
    skeleton: Skeleton,
    walk: AnimationClip,
    turn: AnimationClip,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(PhysicsPlugin)
            .register_plugin(AnimationPlugin)
            .register_plugin(MotionMatchingPlugin)
            .register_plugin(DebugGizmosPlugin)
            .register_plugin(MotionMatchingGizmosPlugin);

        let world = app.main_mut().world_mut();
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);

        let server = world.get_resource::<AssetServer>().unwrap().clone();
        let skeleton_handle = server.add(skeleton());
        let database = baked(
            skeleton_handle.clone(),
            [server.add(walk()), server.add(turn())],
        );
        let graph = server.add(AnimationGraph::from_node(MotionMatchingNode {
            database: server.add(database.clone()),
            ..Default::default()
        }));

        let bones: Vec<SceneEntityRef> = (0..4)
            .map(|_| SceneEntityRef::Entity(world.spawn(Transform::IDENTITY)))
            .collect();
        let player = world.spawn((
            AnimationPlayer::new(4),
            AnimationHandleComponent::new(graph),
            SkeletonComponent::new(skeleton_handle, bones, None),
        ));
        let capsule = spawn_capsule(world);
        world.spawn((
            MotionMatchingGizmos::default(),
            CharacterLink {
                capsule,
                skeleton: player,
            },
            Transform::from_translation(START),
        ));

        Self {
            app,
            capsule,
            player,
            database,
            skeleton: skeleton(),
            walk: walk(),
            turn: turn(),
        }
    }

    fn frame(&mut self) {
        sleep(FRAME);
        self.app.main_mut().update();
    }

    fn bone(&self) -> Transform {
        let world = self.app.main().world();
        let alpha = world.get_resource::<Time>().unwrap().fixed_alpha();
        let facing = world
            .get_component_for_entity::<CharacterSimulation>(self.capsule)
            .unwrap()
            .interpolated_facing(alpha);
        let position = world
            .get_component_for_entity::<GlobalTransform>(self.capsule)
            .unwrap()
            .translation();
        Transform::from_translation_rotation(position, facing)
    }

    fn drawn(&mut self) -> Option<Drawn> {
        let bone = self.bone();
        let clips = [&self.walk, &self.turn];
        let (database, skeleton, player) = (&self.database, &self.skeleton, self.player);
        let world = self.app.main_mut().world_mut();
        let mut drawn = None;
        world.resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
            world
                .get_component_for_entity_mut::<AnimationPlayer>(player)
                .unwrap()
                .for_each_instance_mut::<MotionMatchingNode>(graphs, |_, instance, _| {
                    assert!(!instance.failed);
                    let Some(last_query) = instance.last_query.clone() else {
                        return;
                    };
                    drawn = Some(Drawn {
                        last_query,
                        frame: instance.frame,
                        query: query_shapes(instance, database, skeleton, &clips, &bone).unwrap(),
                        matched: matched_shapes(instance, database, skeleton, &bone).unwrap(),
                        bone: bone.clone(),
                    });
                });
        });
        drawn
    }
}

fn near(first: Vec3, second: Vec3) -> bool {
    first.distance(second) < NEAR
}

fn ring_centers(shapes: &[FeatureShape]) -> Vec<Vec3> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            FeatureShape::Ring { center, .. } => Some(*center),
            _ => None,
        })
        .collect()
}

fn arrow_starts(shapes: &[FeatureShape]) -> Vec<Vec3> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            FeatureShape::Arrow { start, .. } => Some(*start),
            _ => None,
        })
        .collect()
}

fn assert_same_points(rings: Vec<Vec3>, arrows: Vec<Vec3>) {
    assert!(!rings.is_empty());
    assert_eq!(rings.len(), arrows.len());
    for (ring, arrow) in rings.into_iter().zip(arrows) {
        assert!(near(ring, arrow), "{ring} vs {arrow}");
    }
}

fn assert_lined_up(channels: &[Vec<FeatureShape>], bone: &Transform) {
    let [feet, foot_velocities, trajectory, directions] = channels else {
        panic!("the database has four channels");
    };
    assert_same_points(ring_centers(feet), arrow_starts(foot_velocities));
    assert_same_points(ring_centers(trajectory), arrow_starts(directions));
    let Some(FeatureShape::Line { start, .. }) = trajectory.first() else {
        panic!("a trajectory starts with a line");
    };
    assert!(
        near(*start, bone.translation),
        "{start} vs {}",
        bone.translation
    );
}

fn assert_read_from_the_capsule(last_query: &LastQuery, database: &MotionDatabase) {
    let LastQuery {
        frame,
        simulation,
        trajectory,
        ..
    } = last_query;
    let baked = database.simulation(*frame);
    assert!(simulation.translation.distance(START) < 1.0);
    assert!(simulation.translation.distance(baked.translation) > FAR_FROM_THE_DATABASE);
    assert!(simulation.rotation.angle_between(Quat::IDENTITY) > 0.0);
    assert_eq!(trajectory.len(), samples().len());
    assert!(trajectory[0].position.distance(START) < 2.0);
    assert!(trajectory[1].position.x > trajectory[0].position.x);
}

#[test]
fn a_real_search_on_a_moving_capsule_lines_up_its_shapes() {
    let mut stage = Stage::new();
    let mut transitioned = false;

    for _ in 0..FRAMES {
        stage.frame();
        let Some(drawn) = stage.drawn() else {
            continue;
        };
        assert_read_from_the_capsule(&drawn.last_query, &stage.database);
        assert_lined_up(&drawn.query, &drawn.bone);
        assert_lined_up(&drawn.matched, &drawn.bone);
        let database = &stage.database;
        transitioned |= database.range_of(drawn.frame) != database.range_of(drawn.last_query.frame);
    }

    assert!(transitioned, "no search moved playback to the other clip");
}
