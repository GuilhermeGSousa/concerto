use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_ecs::{Entity, World, component::scene::SceneEntityRef};
use concerto_foundation::transform::Transform;
use concerto_name::Name;
use concerto_navigation::{NavEdge, NavGraph, NavLink, NavLinks, NavPoint, NavigationPlugin};
use concerto_scene::{scene::Scene, spawner::SpawnFilter};
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

fn spawn_point(world: &mut World, x: f32, anchor: &str) -> Entity {
    world.spawn((
        Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        NavPoint {
            anchor: Name::new(anchor),
            enabled: true,
        },
    ))
}

fn links(links: Vec<NavLink>) -> NavLinks {
    NavLinks { links }
}

fn link(to: Entity, one_way: bool) -> NavLink {
    NavLink {
        to: SceneEntityRef::Entity(to),
        one_way,
    }
}

#[test]
fn a_saved_scene_spawns_with_every_link_resolved_to_the_spawned_points() {
    let mut authoring = app();
    let world = authoring.main_mut().world_mut();
    let root = world.spawn(());
    let a = spawn_point(world, 0.0, "a");
    let b = spawn_point(world, 1.0, "b");
    let c = spawn_point(world, 2.0, "c");
    for entity in [a, b, c] {
        world.entity_mut(root).add_child(entity);
    }
    world.insert(links(vec![link(b, false), link(c, true)]), a);
    world.insert(links(vec![link(c, false)]), b);
    let scene = Scene::capture(world, root).unwrap();

    let mut game = app();
    let world = game.main_mut().world_mut();
    let parent = world.spawn(());
    let spawned = scene.spawn_in_world(world, parent, &SpawnFilter::default());
    let [a, b, c] = [
        spawned.node_entities[0],
        spawned.node_entities[1],
        spawned.node_entities[2],
    ];

    assert_eq!(
        world.get_component_for_entity::<NavLinks>(a).unwrap().links,
        vec![link(b, false), link(c, true)]
    );
    assert_eq!(
        world.get_component_for_entity::<NavLinks>(b).unwrap().links,
        vec![link(c, false)]
    );
    assert_eq!(
        world
            .get_component_for_entity::<NavPoint>(c)
            .unwrap()
            .anchor,
        Name::new("c")
    );

    game.update();

    let graph = game.get_resource::<NavGraph>().unwrap();
    assert_eq!(graph.anchor(Name::new("b")), Some(b));
    assert_eq!(
        graph.edges().collect::<Vec<_>>(),
        vec![
            NavEdge {
                from: a,
                to: b,
                one_way: false
            },
            NavEdge {
                from: a,
                to: c,
                one_way: true
            },
            NavEdge {
                from: b,
                to: c,
                one_way: false
            },
        ]
    );
}
