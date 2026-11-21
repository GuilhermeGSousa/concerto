use concerto_app::{
    App, main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
};
use concerto_ecs::{Entity, World};
use concerto_gameplay::navigation::follow_nav_steering;
use concerto_motion_matching::DesiredMotion;
use concerto_navigation::NavSteering;
use glam::{Quat, Vec3};

fn app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin);
    app.add_system(Update, follow_nav_steering);
    app.finish_plugin_build();
    app
}

fn world(app: &mut App) -> &mut World {
    app.main_mut().world_mut()
}

fn desired(app: &mut App, entity: Entity) -> DesiredMotion {
    *world(app)
        .get_component_for_entity::<DesiredMotion>(entity)
        .expect("the entity has a DesiredMotion")
}

fn steering() -> NavSteering {
    NavSteering {
        velocity: Vec3::new(1.5, 0.0, -0.5),
        facing: Quat::from_rotation_y(0.7),
    }
}

#[test]
fn desired_motion_takes_the_steering_velocity_and_facing() {
    let mut app = app();
    let entity = world(&mut app).spawn((steering(), DesiredMotion::default()));

    app.update();

    assert_eq!(
        desired(&mut app, entity),
        DesiredMotion {
            velocity: steering().velocity,
            rotation: steering().facing,
        }
    );
}
