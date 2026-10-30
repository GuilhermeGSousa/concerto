//! What the scene tools tell whoever hosts them, and the root they edit under.
use concerto_app::{App, edit_time::AtEditTime};
use concerto_ecs::{
    Component, Entity, Query, ResMut, Resource, World,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::{Event, event_channel::EventChannel},
    signal::{On, Signal},
};
use concerto_foundation::transform::Transform;
use concerto_scene::{capture::capture_scene, scene::Scene, spawner::spawn_scene_in_world};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::tools::{
    camera::{FlyCamera, MAX_PITCH},
    selection::{Select, Selection},
};

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

/// Replaces whatever scene is open with this one.
pub struct OpenScene(pub Scene);
impl Signal for OpenScene {}

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
    world.insert_resource(SceneDirty(false));
    world.trigger(Select(Some(root)));
}

/// The scene open in `world` as data, listing the assets it names, or why it could not be captured.
pub(crate) fn capture_open_scene(world: &mut World) -> Result<Scene, String> {
    let root = world
        .query::<(Entity, &SceneRoot), ()>()
        .iter(world)
        .map(|(root, _)| root)
        .next();
    root.ok_or_else(|| anyhow::anyhow!("No scene is open"))
        .and_then(|root| capture_scene(world, root))
        .map_err(|error| format!("{error:#}"))
}

/// Where the editor camera stands, where it looks, and how fast it flies.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraState {
    /// Where the camera stands.
    pub position: [f32; 3],
    /// Radians around world up, as [`FlyCamera`] holds them.
    pub yaw: f32,
    /// Radians up from level.
    pub pitch: f32,
    /// Metres per second.
    pub speed: f32,
}

/// What the open scene shows besides its content: the selection as a path of child indices from the root, and the camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneView {
    /// The child indices from the scene root to the selected entity; `None` when nothing under the root is selected.
    pub selection: Option<Vec<usize>>,
    /// The fly camera, when the world has one.
    pub camera: Option<CameraState>,
}

pub(crate) fn scene_view(world: &World) -> SceneView {
    SceneView {
        selection: world
            .get_resource::<Selection>()
            .and_then(Selection::entity)
            .and_then(|entity| path_from_root(world, entity)),
        camera: world.get_resource::<FlyCamera>().map(|fly| CameraState {
            position: fly.position.to_array(),
            yaw: fly.yaw,
            pitch: fly.pitch,
            speed: fly.speed,
        }),
    }
}

fn path_from_root(world: &World, entity: Entity) -> Option<Vec<usize>> {
    let mut path = Vec::new();
    let mut current = entity;
    while world
        .get_component_for_entity::<SceneRoot>(current)
        .is_none()
    {
        let parent = world.get_component_for_entity::<ChildOf>(current)?.parent();
        let index = world
            .get_component_for_entity::<Children>(parent)?
            .iter()
            .position(|child| *child == current)?;
        path.push(index);
        current = parent;
    }
    path.reverse();
    Some(path)
}

/// Selects the entity at `selection`'s path from the scene root, or the root when the path no longer resolves, and places the fly camera.
pub struct RestoreView {
    /// The child indices from the scene root; `None` selects nothing.
    pub selection: Option<Vec<usize>>,
    /// Where the fly camera goes, if the world has one.
    pub camera: Option<CameraState>,
}
impl Signal for RestoreView {}

fn restore_view(
    on: On<RestoreView>,
    roots: Query<(Entity, &SceneRoot)>,
    children: Query<&Children>,
    fly: Option<ResMut<FlyCamera>>,
    mut cmd: CommandQueue,
) {
    let view = on.signal();
    let root = roots.iter().map(|(root, _)| root).next();
    let selected = view.selection.as_ref().and_then(|path| {
        let root = root?;
        Some(
            path.iter()
                .try_fold(root, |parent, &index| {
                    children.get_entity(parent)?.iter().nth(index).copied()
                })
                .unwrap_or(root),
        )
    });
    cmd.trigger(Select(selected));
    if let (Some(camera), Some(mut fly)) = (view.camera, fly) {
        fly.position = Vec3::from_array(camera.position);
        fly.yaw = camera.yaw;
        fly.pitch = camera.pitch.clamp(-MAX_PITCH, MAX_PITCH);
        fly.speed = camera.speed;
    }
}

pub(crate) fn build(app: &mut App) {
    let world = app.main_mut().world_mut();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    app.register_event::<SceneEdited>()
        .register_event::<SceneStatus>()
        .register_event::<RevealEntity>()
        .insert_resource(SceneDirty::default())
        .add_listener(open_scene.at_edit_time())
        .add_listener(restore_view.at_edit_time());
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
    use concerto_foundation::assets::AssetId;
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
    fn a_capture_lists_the_assets_the_open_scene_names_not_the_ones_its_file_listed() {
        use concerto_foundation::assets::handle::AssetHandle;
        use concerto_mesh::mesh::MeshComponent;

        let mut app = tools_app();
        let world = app.main_mut().world_mut();
        world.register_type::<MeshComponent>();
        world.trigger(OpenScene(family()));
        let child = world
            .query::<(Entity, &Name), ()>()
            .iter(world)
            .find(|(_, name)| name.as_str() == "child")
            .map(|(entity, _)| entity)
            .unwrap();
        let mesh = AssetId::new();
        world.insert(
            MeshComponent {
                handle: AssetHandle::weak(mesh),
            },
            child,
        );

        let scene = capture_open_scene(world).unwrap();
        let names: Vec<_> = scene.nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["parent", "child"]);
        assert_eq!(scene.nodes[0].children, [1]);
        assert_eq!(scene.referenced_assets, [mesh]);
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

    fn tree() -> Scene {
        Scene {
            nodes: vec![
                node("first", vec![]),
                node("second", vec![2]),
                node("grandchild", vec![]),
            ],
            referenced_assets: vec![],
        }
    }

    fn named(world: &mut World, name: &str) -> Entity {
        world
            .query::<(Entity, &Name), ()>()
            .iter(world)
            .find(|(_, found)| found.as_str() == name)
            .map(|(entity, _)| entity)
            .unwrap()
    }

    fn selected(world: &World) -> Option<Entity> {
        world.get_resource::<Selection>().unwrap().entity()
    }

    fn opened_tree() -> App {
        let mut app = tools_app();
        app.main_mut().world_mut().trigger(OpenScene(tree()));
        app
    }

    #[test]
    fn the_view_names_the_selection_by_its_path_from_the_root() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();

        let grandchild = named(world, "grandchild");
        world.trigger(Select(Some(grandchild)));
        assert_eq!(scene_view(world).selection, Some(vec![1, 0]));

        let first = named(world, "first");
        world.trigger(Select(Some(first)));
        assert_eq!(scene_view(world).selection, Some(vec![0]));
    }

    #[test]
    fn a_selected_root_is_the_empty_path_and_no_selection_is_none() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();
        assert_eq!(scene_view(world).selection, Some(vec![]));

        world.trigger(Select(None));
        assert_eq!(scene_view(world).selection, None);

        let outside = world.spawn(());
        world.trigger(Select(Some(outside)));
        assert_eq!(scene_view(world).selection, None);
        assert_eq!(scene_view(world).camera, None);
    }

    #[test]
    fn restoring_a_view_selects_the_entity_at_its_path() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();

        world.trigger(RestoreView {
            selection: Some(vec![1, 0]),
            camera: None,
        });
        assert_eq!(selected(world), Some(named(world, "grandchild")));

        world.trigger(RestoreView {
            selection: None,
            camera: None,
        });
        assert_eq!(selected(world), None);
    }

    #[test]
    fn a_path_that_no_longer_resolves_selects_the_root() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();
        let root = roots(world)[0];
        world.trigger(Select(None));

        world.trigger(RestoreView {
            selection: Some(vec![1, 4]),
            camera: None,
        });

        assert_eq!(selected(world), Some(root));
    }

    #[test]
    fn restoring_a_view_leaves_the_scene_clean() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();

        world.trigger(RestoreView {
            selection: Some(vec![1]),
            camera: None,
        });

        assert!(!dirty(world));
        assert!(written::<SceneEdited>(&app).is_empty());
    }

    #[test]
    fn restoring_a_view_places_the_fly_camera_with_its_speed() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();
        world.insert_resource(FlyCamera::default());

        world.trigger(RestoreView {
            selection: None,
            camera: Some(CameraState {
                position: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: 3.0,
                speed: 20.0,
            }),
        });

        let fly = world.get_resource::<FlyCamera>().unwrap();
        assert_eq!(fly.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!((fly.yaw, fly.pitch, fly.speed), (0.5, MAX_PITCH, 20.0));
        assert_eq!(
            scene_view(world).camera,
            Some(CameraState {
                position: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: MAX_PITCH,
                speed: 20.0,
            })
        );
    }

    #[test]
    fn restoring_without_a_fly_camera_ignores_the_camera() {
        let mut app = opened_tree();
        let world = app.main_mut().world_mut();

        world.trigger(RestoreView {
            selection: Some(vec![0]),
            camera: Some(CameraState {
                position: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: 0.1,
                speed: 20.0,
            }),
        });

        assert!(world.get_resource::<FlyCamera>().is_none());
        assert_eq!(selected(world), Some(named(world, "first")));
    }

    #[derive(Resource, Clone, Default)]
    struct Placed(std::sync::Arc<std::sync::Mutex<Option<(Vec3, Vec3)>>>);

    fn record_camera(
        exclusive: concerto_ecs::Res<concerto_render::ExclusiveCamera>,
        transforms: Query<&Transform>,
        placed: concerto_ecs::Res<Placed>,
    ) {
        if let Some(transform) = transforms.get_entity(exclusive.0) {
            *placed.0.lock().unwrap() =
                Some((transform.translation, transform.rotation * Vec3::NEG_Z));
        }
    }

    fn game(app: &mut App) {
        app.register_plugin(concerto::DefaultPlugins);
    }

    #[test]
    fn a_restored_camera_places_the_editor_camera_on_the_next_step() {
        let Some(gpu) = concerto_render::gpu::Gpu::request_for_test() else {
            return;
        };
        let placed = Placed::default();
        let recorder = placed.clone();
        let pending = crate::hosted::begin(
            crate::hosted::HostRequest {
                gpu: Some(&gpu),
                recipe: game,
                size: (64, 48),
                scale_factor: 1.0,
                role: crate::hosted::HostRole::Edit,
                clipboard: None,
            },
            move |app| {
                app.register_plugin(crate::tools::EditToolsPlugin)
                    .insert_resource(recorder)
                    .add_system(
                        concerto_app::schedule_groups::LateUpdate,
                        record_camera.at_edit_time(),
                    );
            },
            |_| {},
        );
        let mut hosted = crate::hosted::finish_blocking(pending);
        hosted.step_for_test();
        let camera = CameraState {
            position: [1.0, 2.0, 3.0],
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            speed: 12.0,
        };

        hosted.trigger_for_test(RestoreView {
            selection: None,
            camera: Some(camera),
        });
        hosted.step_for_test();

        let fly = hosted.world().get_resource::<FlyCamera>().unwrap();
        assert_eq!(fly.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(fly.speed, 12.0);
        let (at, facing) = placed.0.lock().unwrap().unwrap();
        assert_eq!(at, Vec3::new(1.0, 2.0, 3.0));
        assert!((facing - Vec3::NEG_X).length() < 1e-4, "{facing:?}");
        assert_eq!(scene_view(hosted.world()).camera, Some(camera));
    }
}
