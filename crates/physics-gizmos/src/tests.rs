use std::f32::consts::FRAC_PI_3;

use concerto_app::{
    App,
    plugins::{AssetManagerPlugin, TransformPlugin},
    schedule_groups::{First, LateFixedUpdate, LateUpdate, Update},
};
use concerto_debug_gizmos::{DebugGizmosPlugin, GizmoStorage};
use concerto_ecs::{
    World,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
};
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_physics::{
    collider::Collider,
    contact::Contacts,
    physics_state::PhysicsState,
    plugin::PhysicsPlugin,
    ragdoll::{BodyPose, JointSettings, PartSettings, PartShape, RagdollSettings},
    rigid_body::RigidBody,
};
use glam::{Mat4, Quat, Vec3};

use crate::{PhysicsGizmos, PhysicsGizmosPlugin};

fn chain() -> RagdollSettings {
    let link = |index: usize| {
        PartSettings::new(
            BodyPose::from_translation(Vec3::new(0.0, -0.5 * index as f32, 0.0)),
            PartShape::Capsule {
                half_height: 0.15,
                radius: 0.08,
            },
        )
        .with_shape_offset(BodyPose::from_translation(Vec3::new(0.0, -0.25, 0.0)))
    };
    let joint = JointSettings::new(Vec3::NEG_Y, Vec3::X)
        .with_swing(0.6, 0.6)
        .with_twist(-0.3, 0.3);
    RagdollSettings::new()
        .with_part(link(0))
        .with_part(link(1).with_parent(0, joint))
}

struct Stage {
    app: App,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(TransformPlugin);
        app.register_plugin(PhysicsPlugin);
        app.register_plugin(DebugGizmosPlugin);
        app.register_plugin(PhysicsGizmosPlugin);

        let world = app.main_mut().world_mut();
        world.spawn((
            Collider::cuboid(10.0, 0.5, 10.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0))),
        ));
        world.spawn((
            Collider::cuboid(0.5, 0.5, 0.5),
            RigidBody::default(),
            Contacts::default(),
            Transform::from_translation(Vec3::new(0.0, 0.52, 0.0)),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(0.0, 0.52, 0.0))),
        ));
        let owner = world.spawn(Transform::default());

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);

        let settings = chain();
        let built = settings.build().expect("the chain should build");
        let root = BodyPose::new(Vec3::new(3.0, 2.0, 0.0), Quat::from_rotation_z(FRAC_PI_3));
        let start: Vec<BodyPose> = settings
            .parts
            .iter()
            .map(|part| root.then(part.pose))
            .collect();
        world
            .get_resource_mut::<PhysicsState>()
            .unwrap()
            .create_ragdoll(&built, owner, &start)
            .expect("the chain should be created");

        let mut stage = Self { app };
        for _ in 0..30 {
            stage.world().run_schedule(LateFixedUpdate);
            stage.tick();
        }
        stage
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn tick(&mut self) {
        let world = self.world();
        world.run_schedule(First);
        world.run_schedule(Update);
        world.run_schedule(LateUpdate);
    }

    fn segments(&mut self, shown: PhysicsGizmos) -> usize {
        self.world().insert_resource(shown);
        self.tick();
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }
}

#[test]
fn each_layer_draws_its_physics_while_its_flag_is_on_and_nothing_when_all_are_off() {
    let mut stage = Stage::new();
    let nothing = PhysicsGizmos::default();

    assert_eq!(stage.segments(nothing.clone()), 0);
    assert!(
        stage.segments(PhysicsGizmos {
            colliders: true,
            ..nothing.clone()
        }) > 0,
        "the colliders"
    );
    assert!(
        stage.segments(PhysicsGizmos {
            ragdolls: true,
            ..nothing.clone()
        }) > 0,
        "the ragdoll's parts and joint limits"
    );
    assert!(
        stage.segments(PhysicsGizmos {
            contacts: true,
            ..nothing.clone()
        }) > 0,
        "the box's contact with the floor"
    );
    assert_eq!(stage.segments(nothing), 0);
}
