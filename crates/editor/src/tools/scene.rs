//! What the scene tools tell whoever hosts them, and the root they edit under.
use std::path::PathBuf;

use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Component, Entity, Resource, World,
    entity::hierarchy::{ChildOf, Children},
    events::{Event, event_channel::EventChannel},
    signal::{On, Signal},
};
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, content::AssetRegistry},
    transform::Transform,
};
use concerto_scene::{capture::capture_scene, scene::Scene, spawner::spawn_scene_in_world};

use crate::tools::selection::Select;

/// Marks the entity every node of the open scene hangs under.
#[derive(Component)]
pub struct SceneRoot;

/// The scene's content changed.
#[derive(Event, Clone)]
pub struct SceneEdited;

/// Whether the open scene has edits its last capture does not hold.
#[derive(Resource, Default)]
pub struct SceneDirty(pub bool);

/// An edit was refused; the text says why.
#[derive(Event, Clone)]
pub struct SceneStatus(pub String);

/// An entity the tree should scroll to and expand.
#[derive(Event, Clone)]
pub struct RevealEntity(pub Entity);

/// Tells the scene's asset server where the project's content is.
pub struct PublishContent {
    pub root: PathBuf,
    pub registry: AssetRegistry,
}
impl Signal for PublishContent {}

/// Replaces whatever scene is open with this one.
pub struct OpenScene(pub Scene);
impl Signal for OpenScene {}

#[derive(Resource, Default)]
struct ReferencedAssets(Vec<AssetId>);

fn publish_content(on: On<PublishContent>, world: &mut World) {
    let Some(server) = world.get_resource::<AssetServer>() else {
        return;
    };
    let PublishContent { root, registry } = on.signal();
    if let Err(error) = server.publish_project_content(root, registry.clone()) {
        write(
            world,
            SceneStatus(format!("Could not activate project assets: {error:#}")),
        );
    }
}

fn open_scene(on: On<OpenScene>, world: &mut World) {
    let stale: Vec<Entity> = world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .map(|(root, _)| root)
        .collect();
    for root in stale {
        world.despawn(root);
    }
    world.register_component::<Transform>();
    let root = world.spawn((Transform::IDENTITY, SceneRoot));
    spawn_scene_in_world(world, &on.signal().0, root);
    world.insert_resource(ReferencedAssets(on.signal().0.referenced_assets.clone()));
    world.insert_resource(SceneDirty(false));
    world.trigger(Select(Some(root)));
}

/// The scene open in `world` as data, with the assets its file listed, or why it could not be captured.
pub(crate) fn capture_open_scene(world: &mut World) -> Result<Scene, String> {
    let root = world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .map(|(root, _)| root)
        .next();
    root.ok_or_else(|| anyhow::anyhow!("No scene is open"))
        .and_then(|root| capture_scene(world, root))
        .map(|mut scene| {
            scene.referenced_assets = world
                .get_resource::<ReferencedAssets>()
                .map(|referenced| referenced.0.clone())
                .unwrap_or_default();
            scene
        })
        .map_err(|error| format!("{error:#}"))
}

pub(crate) fn build(app: &mut App) {
    let world = app.main_mut().world_mut();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    app.register_event::<SceneEdited>()
        .register_event::<SceneStatus>()
        .register_event::<RevealEntity>()
        .insert_resource(SceneDirty::default())
        .add_listener(publish_content.at_edit_time())
        .add_listener(open_scene.at_edit_time());
}

/// Records that the open scene's last capture was written.
pub(crate) fn mark_saved(world: &mut World) {
    world.insert_resource(SceneDirty(false));
}

/// Marks the scene dirty and tells whoever hosts it that it changed.
pub(crate) fn report_edit(world: &mut World) {
    world.insert_resource(SceneDirty(true));
    write(world, SceneEdited);
}

pub(crate) fn write<E: Event + 'static>(world: &mut World, event: E) {
    if let Some(channel) = world.get_resource_mut::<EventChannel<E>>() {
        channel.push_event(event);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use concerto_app::{main_schedule::MainSchedulePlugin, plugins::TimePlugin};
    use concerto_ecs::events::event_cursor::EventCursor;

    pub(crate) fn tools_app() -> App {
        let mut app = App::for_editing();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin);
        crate::tools::build_core(&mut app);
        app.finish_plugin_build();
        app
    }

    pub(crate) fn written<E: Event + Clone + 'static>(app: &App) -> Vec<E> {
        app.get_resource::<EventChannel<E>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{tools_app, written};
    use super::*;
    use crate::tools::selection::Selection;
    use concerto_ecs::component::name::Name;
    use concerto_scene::scene::SceneNode;

    fn node(name: &str, children: Vec<usize>) -> SceneNode {
        SceneNode {
            name: name.into(),
            children,
            components: vec![],
        }
    }

    fn family() -> Scene {
        Scene {
            nodes: vec![node("parent", vec![1]), node("child", vec![])],
            referenced_assets: vec![AssetId::new()],
        }
    }

    fn roots(world: &mut World) -> Vec<Entity> {
        world
            .query::<(Entity, &SceneRoot), ()>()
            .iter(world)
            .map(|(root, _)| root)
            .collect()
    }

    fn names(world: &mut World) -> Vec<String> {
        world
            .query::<&Name, ()>()
            .iter(world)
            .map(|name| name.as_str().to_owned())
            .collect()
    }

    #[test]
    fn an_opened_scene_hangs_under_one_selected_root() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();

        world.trigger(OpenScene(family()));

        let root = roots(world)[0];
        let children = world.get_component_for_entity::<Children>(root).unwrap();
        assert_eq!(children.iter().count(), 1);
        assert_eq!(
            world.get_resource::<Selection>().unwrap().entity(),
            Some(root)
        );
        assert!(written::<SceneEdited>(&app).is_empty());
    }

    #[test]
    fn opening_another_scene_replaces_the_one_that_was_open() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.trigger(OpenScene(family()));

        world.trigger(OpenScene(Scene {
            nodes: vec![node("alone", vec![])],
            referenced_assets: vec![],
        }));

        assert_eq!(roots(world).len(), 1);
        assert_eq!(names(world), ["alone"]);
    }

    #[test]
    fn a_capture_returns_the_open_scene_and_the_assets_its_file_listed() {
        let mut app = tools_app();
        let opened = family();
        let world = app.main_mut().world_mut();
        world.trigger(OpenScene(opened.clone()));

        let scene = capture_open_scene(world).unwrap();
        let names: Vec<_> = scene.nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["parent", "child"]);
        assert_eq!(scene.nodes[0].children, [1]);
        assert_eq!(scene.referenced_assets, opened.referenced_assets);
    }

    fn dirty(world: &World) -> bool {
        world.get_resource::<SceneDirty>().unwrap().0
    }

    #[test]
    fn a_reported_edit_makes_the_scene_dirty_until_it_is_marked_saved() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.trigger(OpenScene(family()));
        assert!(!dirty(world));

        report_edit(world);
        assert!(dirty(world));
        assert_eq!(written::<SceneEdited>(&app).len(), 1);

        let world = app.main_mut().world_mut();
        capture_open_scene(world).unwrap();
        assert!(
            dirty(world),
            "a capture alone does not mean the scene was written"
        );

        mark_saved(world);
        assert!(!dirty(world));
    }

    #[test]
    fn opening_a_scene_makes_it_clean() {
        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.trigger(OpenScene(family()));
        report_edit(world);

        world.trigger(OpenScene(family()));

        assert!(!dirty(world));
    }

    #[test]
    fn a_capture_with_no_scene_open_returns_the_reason() {
        let mut app = tools_app();

        let captured = capture_open_scene(app.main_mut().world_mut());

        assert_eq!(captured.unwrap_err(), "No scene is open");
    }

    #[test]
    fn a_content_root_that_cannot_be_resolved_is_reported_as_a_status() {
        let mut app = tools_app();
        app.insert_resource(AssetServer::new());

        app.main_mut().world_mut().trigger(PublishContent {
            root: PathBuf::from("/nowhere/that/exists"),
            registry: AssetRegistry::default(),
        });

        let statuses = written::<SceneStatus>(&app);
        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0]
                .0
                .starts_with("Could not activate project assets: "),
            "{}",
            statuses[0].0
        );
    }

    #[test]
    fn the_tools_need_no_asset_server_to_be_told_of_content() {
        let mut app = tools_app();

        app.main_mut().world_mut().trigger(PublishContent {
            root: PathBuf::from("/nowhere/that/exists"),
            registry: AssetRegistry::default(),
        });

        assert!(written::<SceneStatus>(&app).is_empty());
    }
}
