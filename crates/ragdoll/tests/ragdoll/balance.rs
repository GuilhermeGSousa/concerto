use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{FixedUpdate, LateFixedUpdate, LateUpdate, Update},
};
use concerto_ecs::entity::Entity;
use concerto_ecs::resource::{ResMut, Resource};
use concerto_ecs::signal::On;
use concerto_foundation::assets::asset_server::AssetServer;
use concerto_foundation::transform::Transform;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::plugin::PhysicsPlugin;
use concerto_physics::ragdoll::RagdollId;
use concerto_ragdoll::testing::{
    SpawnedHumanoid, humanoid_skeleton, reference_humanoid, spawn_humanoid,
};
use concerto_ragdoll::{
    Balance, BehaviourEnded, BodyDrive, BodySense, BodySettled, BodyState, CharacterRagdoll,
    Outcome, PhysicalState, RagdollBody, RagdollPlugin,
};
use glam::Vec3;

#[derive(Resource, Default)]
struct Ended(Vec<BehaviourEnded>);

fn record(on: On<BehaviourEnded>, mut ended: ResMut<Ended>) {
    ended.0.push(*on.signal());
}

#[derive(Resource, Default)]
struct Settles(usize);

fn count_settles(_: On<BodySettled>, mut settles: ResMut<Settles>) {
    settles.0 += 1;
}

pub struct Stage {
    pub app: App,
    pub humanoid: SpawnedHumanoid,
    pub character: Entity,
}

impl Stage {
    pub fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(PhysicsPlugin)
            .register_plugin(RagdollPlugin);
        app.add_listener(record).insert_resource(Ended::default());
        app.add_listener(count_settles)
            .insert_resource(Settles::default());
        app.finish_plugin_build();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        let reference = reference_humanoid();
        let skeleton = server.add(reference.skeleton);
        let mut asset = reference.ragdoll;
        asset.skeleton = skeleton.clone();
        let asset = server.add(asset);
        app.update();
        let world = app.main_mut().world_mut();
        world.spawn((
            Collider::cuboid(50.0, 0.5, 50.0),
            Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        ));
        let humanoid = spawn_humanoid(world, skeleton, Transform::default());
        let character = humanoid.skeleton;
        world.insert(
            (
                CharacterRagdoll::new(asset),
                BodyDrive::default(),
                BodySense::default(),
                PhysicalState::default(),
            ),
            character,
        );
        let mut stage = Self {
            app,
            humanoid,
            character,
        };
        stage.run(5);
        stage
    }

    pub fn run(&mut self, frames: usize) {
        let rest = humanoid_skeleton();
        for _ in 0..frames {
            let world = self.app.main_mut().world_mut();
            for (bone, local) in self.humanoid.bones.iter().zip(rest.rest_pose.iter()) {
                let transform = world
                    .get_component_for_entity_mut::<Transform>(*bone)
                    .unwrap();
                transform.translation = local.translation;
                transform.rotation = local.rotation;
            }
            self.app.serve(FixedUpdate);
            self.app.serve(LateFixedUpdate);
            self.app.serve(Update);
            self.app.serve(LateUpdate);
        }
    }

    pub fn push(&mut self, bone: &str, impulse: Vec3) {
        let world = self.app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(self.character)
            .unwrap();
        let part = world
            .get_component_for_entity::<RagdollBody>(self.character)
            .unwrap()
            .resolved()
            .part_of(bone)
            .unwrap();
        let state = self.app.get_resource_mut::<PhysicsState>().unwrap();
        let body = state.ragdoll_bodies(ragdoll)[part];
        state.add_impulse(body, impulse);
    }

    pub fn part(&self, bone: &str) -> Vec3 {
        let world = self.app.main().world();
        let ragdoll = *world
            .get_component_for_entity::<RagdollId>(self.character)
            .unwrap();
        let part = world
            .get_component_for_entity::<RagdollBody>(self.character)
            .unwrap()
            .resolved()
            .part_of(bone)
            .unwrap();
        let mut pose = Vec::new();
        self.app
            .get_resource::<PhysicsState>()
            .unwrap()
            .ragdoll_pose(ragdoll, &mut pose);
        pose[part].translation
    }

    pub fn state(&self) -> BodyState {
        self.app
            .main()
            .world()
            .get_component_for_entity::<PhysicalState>(self.character)
            .unwrap()
            .state
    }

    pub fn settles(&self) -> usize {
        self.app.get_resource::<Settles>().unwrap().0
    }

    pub fn ended(&self) -> Vec<BehaviourEnded> {
        self.app.get_resource::<Ended>().unwrap().0.clone()
    }

    pub fn balance(&self) -> Option<Balance> {
        self.app
            .main()
            .world()
            .get_component_for_entity::<Balance>(self.character)
            .cloned()
    }

    pub fn insert<C: concerto_ecs::Component>(&mut self, component: C) {
        self.app
            .main_mut()
            .world_mut()
            .insert(component, self.character);
    }
}

#[derive(Debug)]
struct Pushed {
    outcome: Option<Outcome>,
    steps: u32,
    travelled: Vec3,
    pelvis_height: f32,
    state: BodyState,
}

fn pushed(balance: Balance, impulse: Vec3, seconds: f32) -> Pushed {
    let mut stage = Stage::new();
    stage.insert(balance);
    stage.run(15);
    let start = stage.part("pelvis");
    stage.push("spine_03", impulse);
    let mut steps = 0;
    let mut stepping = false;
    let mut pelvis = start;
    for _ in 0..(seconds / 0.033) as usize {
        if !stage.ended().is_empty() {
            break;
        }
        pelvis = stage.part("pelvis");
        if let Some(balance) = stage.balance() {
            if balance.stepping() && !stepping {
                steps += 1;
            }
            stepping = balance.stepping();
        }
        stage.run(1);
    }
    Pushed {
        outcome: stage.ended().first().map(|ended| ended.outcome),
        steps,
        travelled: pelvis - start,
        pelvis_height: pelvis.y,
        state: stage.state(),
    }
}

#[test]
fn a_balancing_body_stands_absorbs_a_push_steps_to_catch_itself_or_gives_up() {
    let patient = Balance::default().with_settle_seconds(f32::MAX);
    let standing = pushed(patient, Vec3::ZERO, 10.0);
    assert_eq!(standing.outcome, None, "an unpushed body keeps balancing");
    assert_eq!(standing.steps, 0);
    assert!(
        standing.travelled.length() < 0.05,
        "an unpushed body stands where it is for ten seconds, it drifted {}",
        standing.travelled
    );
    assert_eq!(standing.state, BodyState::Reacting);

    for direction in [Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X] {
        let light = pushed(Balance::default(), direction * 12.0, 10.0);
        assert_eq!(
            (light.outcome, light.steps),
            (Some(Outcome::Succeeded), 0),
            "a light push towards {direction} is absorbed without a step: {light:?}"
        );
        assert_eq!(light.state, BodyState::Animated);

        let medium = pushed(Balance::default(), direction * 70.0, 10.0);
        assert!(
            medium.outcome == Some(Outcome::Succeeded)
                && (1..=2).contains(&medium.steps)
                && medium.pelvis_height > 0.9
                && medium.travelled.dot(direction) > 0.0,
            "a medium push towards {direction} takes a step or two that way and the body ends standing: {medium:?}"
        );

        let hard = pushed(Balance::default(), direction * 300.0, 10.0);
        assert_eq!(
            (hard.outcome, hard.state),
            (Some(Outcome::GaveUp), BodyState::Falling),
            "a hard push towards {direction} ends in giving up: {hard:?}"
        );
    }
}
