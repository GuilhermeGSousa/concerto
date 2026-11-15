use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
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
        .register_plugin(AssetManagerPlugin)
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

struct Edit {
    name: &'static str,
    apply: fn(&mut World, Entity, Entity),
    check: fn(&NavGraph, Entity, Entity) -> bool,
}

fn edges(graph: &NavGraph) -> Vec<NavEdge> {
    graph.edges().collect()
}

#[test]
fn the_graph_follows_each_edit_to_its_points_and_links() {
    let edits = [
        Edit {
            name: "a point moves",
            apply: |world, a, _| {
                world
                    .get_component_for_entity_mut::<Transform>(a)
                    .unwrap()
                    .translation = Vec3::Z;
            },
            check: |graph, a, _| graph.point(a).unwrap().position == Vec3::Z,
        },
        Edit {
            name: "a point is despawned",
            apply: |world, _, b| world.despawn(b),
            check: |graph, _, b| graph.point(b).is_none() && graph.edges().count() == 0,
        },
        Edit {
            name: "links are removed",
            apply: |world, a, _| {
                world.remove_component::<NavLinks>(a);
            },
            check: |graph, a, _| graph.point(a).is_some() && graph.edges().count() == 0,
        },
        Edit {
            name: "a link changes",
            apply: |world, a, _| {
                world
                    .get_component_for_entity_mut::<NavLinks>(a)
                    .unwrap()
                    .links[0]
                    .one_way = true;
            },
            check: |graph, a, b| {
                edges(graph)
                    == vec![NavEdge {
                        from: a,
                        to: b,
                        one_way: true,
                    }]
            },
        },
    ];
    for edit in edits {
        let Settled { mut app, a, b } = settled();
        (edit.apply)(world(&mut app), a, b);
        app.update();
        assert!((edit.check)(graph(&app), a, b), "{}", edit.name);
    }
}

#[test]
fn an_edit_app_builds_the_graph() {
    let mut app = App::for_editing();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(NavigationPlugin);
    app.finish_plugin_build();
    let a = spawn_point(&mut app, Vec3::X);

    app.update();

    assert_eq!(graph(&app).point(a).unwrap().position, Vec3::X);
}
