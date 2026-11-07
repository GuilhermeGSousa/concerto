use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{TimePlugin, TransformPlugin},
};
use concerto_ecs::{Entity, World, component::scene::SceneEntityRef};
use concerto_foundation::transform::Transform;
use concerto_name::Name;
use concerto_navigation::{NavEdge, NavGraph, NavLink, NavLinks, NavPoint, NavigationPlugin};
use glam::Vec3;

fn app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(NavigationPlugin);
    app.finish_plugin_build();
    app
}

fn world(app: &mut App) -> &mut World {
    app.main_mut().world_mut()
}

fn graph(app: &App) -> &NavGraph {
    app.get_resource::<NavGraph>().unwrap()
}

fn spawn_point(app: &mut App, position: Vec3) -> Entity {
    world(app).spawn((
        Transform::from_translation(position),
        NavPoint {
            anchor: Name::default(),
            enabled: true,
        },
    ))
}

fn link(to: Entity) -> NavLink {
    NavLink {
        to: SceneEntityRef::Entity(to),
        one_way: false,
    }
}

struct Settled {
    app: App,
    a: Entity,
    b: Entity,
}

fn settled() -> Settled {
    let mut app = app();
    let a = spawn_point(&mut app, Vec3::ZERO);
    let b = spawn_point(&mut app, Vec3::X);
    world(&mut app).insert(
        NavLinks {
            links: vec![link(b)],
        },
        a,
    );
    app.update();
    app.update();
    Settled { app, a, b }
}

fn rebuilt_by_next_update(app: &mut App) -> bool {
    let before = world(app).resource_changed_tick::<NavGraph>();
    app.update();
    world(app).resource_changed_tick::<NavGraph>() != before
}

#[test]
fn the_graph_is_built_from_the_points_and_their_links() {
    let Settled { app, a, b } = settled();

    assert_eq!(graph(&app).point(a).unwrap().position, Vec3::ZERO);
    assert_eq!(graph(&app).point(b).unwrap().position, Vec3::X);
    assert_eq!(
        graph(&app).edges().collect::<Vec<_>>(),
        vec![NavEdge {
            from: a,
            to: b,
            one_way: false
        }]
    );
}

#[test]
fn the_graph_is_not_rebuilt_when_nothing_changed() {
    let Settled { mut app, .. } = settled();

    assert!(!rebuilt_by_next_update(&mut app));
}

#[test]
fn the_graph_is_rebuilt_when_a_point_moves() {
    let Settled { mut app, a, .. } = settled();
    world(&mut app)
        .get_component_for_entity_mut::<Transform>(a)
        .unwrap()
        .translation = Vec3::new(0.0, 0.0, 3.0);

    assert!(rebuilt_by_next_update(&mut app));
    assert_eq!(
        graph(&app).point(a).unwrap().position,
        Vec3::new(0.0, 0.0, 3.0)
    );
}

#[test]
fn the_graph_is_rebuilt_when_a_point_is_added() {
    let Settled { mut app, .. } = settled();
    let c = spawn_point(&mut app, Vec3::Z);

    assert!(rebuilt_by_next_update(&mut app));
    assert_eq!(graph(&app).point(c).unwrap().position, Vec3::Z);
}

#[test]
fn the_graph_is_rebuilt_when_a_point_is_removed() {
    let Settled { mut app, b, .. } = settled();
    world(&mut app).remove_component::<NavPoint>(b);

    assert!(rebuilt_by_next_update(&mut app));
    assert!(graph(&app).point(b).is_none());
    assert_eq!(graph(&app).edges().count(), 0);
}

#[test]
fn the_graph_is_rebuilt_when_a_point_is_despawned() {
    let Settled { mut app, b, .. } = settled();
    world(&mut app).despawn(b);

    assert!(rebuilt_by_next_update(&mut app));
    assert!(graph(&app).point(b).is_none());
}

#[test]
fn the_graph_is_rebuilt_when_links_are_removed() {
    let Settled { mut app, a, .. } = settled();
    world(&mut app).remove_component::<NavLinks>(a);

    assert!(rebuilt_by_next_update(&mut app));
    assert!(graph(&app).point(a).is_some());
    assert_eq!(graph(&app).edges().count(), 0);
}

#[test]
fn the_graph_is_rebuilt_when_links_change() {
    let Settled { mut app, a, b } = settled();
    world(&mut app)
        .get_component_for_entity_mut::<NavLinks>(a)
        .unwrap()
        .links[0]
        .one_way = true;

    assert!(rebuilt_by_next_update(&mut app));
    assert_eq!(
        graph(&app).edges().collect::<Vec<_>>(),
        vec![NavEdge {
            from: a,
            to: b,
            one_way: true
        }]
    );
}

#[test]
fn the_graph_is_rebuilt_when_a_point_is_disabled() {
    let Settled { mut app, b, .. } = settled();
    world(&mut app)
        .get_component_for_entity_mut::<NavPoint>(b)
        .unwrap()
        .enabled = false;

    assert!(rebuilt_by_next_update(&mut app));
    assert!(!graph(&app).point(b).unwrap().enabled);
}

#[test]
fn an_edit_app_builds_the_graph() {
    let mut app = App::for_editing();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(NavigationPlugin);
    app.finish_plugin_build();
    let a = spawn_point(&mut app, Vec3::X);

    app.update();

    assert_eq!(graph(&app).point(a).unwrap().position, Vec3::X);
}
