use concerto_app::{
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    App, PlaceableAssets, PlaceableKind,
};
use concerto_ecs::{
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    Entity,
};
use concerto_foundation::assets::{
    asset_server::AssetServer, content::save_content_asset_into, registry::AssetRegistry, AssetId,
};
use concerto_foundation::transform::Transform;
use concerto_scene::{
    plugin::ScenePlugin,
    scene::{Scene, SceneNode},
};
use glam::Vec3;

fn node(name: &str, children: Vec<usize>, x: f32) -> SceneNode {
    let mut node = SceneNode {
        name: name.to_string(),
        children,
        components: vec![],
    };
    node.push_component(&Transform::from_translation(Vec3::new(x, 0.0, 0.0)))
        .unwrap();
    node
}

fn place(app: &mut App, kind: &str, asset: AssetId) -> Entity {
    let kind: PlaceableKind = *app
        .get_resource::<PlaceableAssets>()
        .unwrap()
        .get(kind)
        .unwrap_or_else(|| panic!("{kind} is not placeable"));
    (kind.request_load)(app.main_mut().world_mut(), asset);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(kind.is_loaded)(app.main().world(), asset) {
        assert!(
            std::time::Instant::now() < deadline,
            "{} never loaded",
            kind.kind
        );
        app.update();
    }
    let entity = app.main_mut().world_mut().spawn(());
    (kind.fill)(app.main_mut().world_mut(), asset, entity);
    entity
}

struct Placed {
    name: String,
    x: f32,
}

fn children_of(app: &App, parent: Entity) -> Vec<Placed> {
    let world = app.main().world();
    world
        .get_component_for_entity::<Children>(parent)
        .map(|children| {
            children
                .iter()
                .map(|&child| Placed {
                    name: world
                        .get_component_for_entity::<Name>(child)
                        .unwrap()
                        .as_str()
                        .to_string(),
                    x: world
                        .get_component_for_entity::<Transform>(child)
                        .unwrap()
                        .translation
                        .x,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn placing_a_scene_adds_its_roots_as_children() {
    let root = std::env::temp_dir().join(format!("place-assets-{}", AssetId::new().simple_hex()));
    let mut registry = AssetRegistry::new();
    let scene = save_content_asset_into(
        &Scene {
            nodes: vec![
                node("left", vec![2], 1.0),
                node("right", vec![], 2.0),
                node("leaf", vec![], 3.0),
            ],
            referenced_assets: vec![],
        },
        &root,
        "content/props/pair.gasset",
        &mut registry,
    )
    .unwrap();

    let mut app = App::for_editing();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(ScenePlugin);
    app.get_resource::<AssetServer>()
        .unwrap()
        .publish_project_content(&root, registry)
        .unwrap();
    app.finish_plugin_build();

    let placed_scene = place(&mut app, "Scene", scene);
    let mut roots = children_of(&app, placed_scene);
    roots.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        roots
            .iter()
            .map(|root| format!("{} at {}", root.name, root.x))
            .collect::<Vec<_>>(),
        vec!["left at 1", "right at 2"]
    );
    let world = app.main().world();
    let left = *world
        .get_component_for_entity::<Children>(placed_scene)
        .unwrap()
        .iter()
        .find(|&&child| {
            world
                .get_component_for_entity::<Name>(child)
                .is_some_and(|name| name.as_str() == "left")
        })
        .unwrap();
    assert_eq!(
        children_of(&app, left)
            .iter()
            .map(|leaf| format!("{} at {}", leaf.name, leaf.x))
            .collect::<Vec<_>>(),
        vec!["leaf at 3"]
    );
    assert_eq!(
        world
            .get_component_for_entity::<ChildOf>(left)
            .map(ChildOf::parent),
        Some(placed_scene)
    );
    std::fs::remove_dir_all(root).unwrap();
}
