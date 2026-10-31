use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use concerto_app::{
    App,
    edit_time::{AtEditTime, AtPreviewTime},
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::Update,
};
use concerto_ecs::{
    Entity, Event, IntoSystem, Res, ResMut, Resource, System, World,
    command::CommandQueue,
    component::name::Name,
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_cursor::EventCursor, event_writer::EventWriter},
    signal::{On, Signal},
    system::BoxedSystem,
    world_id::WorldId,
};
use concerto_foundation::{
    assets::{
        Asset, AssetId,
        asset_server::AssetServer,
        content::{
            CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, read_content_asset,
            write_content_asset,
        },
        registry::{AssetEntry, AssetRegistry},
    },
    transform::Transform,
};
use concerto_scene::scene::{Scene, SceneNode};
use concerto_ui::{focus::FocusedWidget, node::UINode, theme::UITheme};
use concerto_window::{
    events::WindowEvent,
    plugin::{Window, WindowPlugin},
};

use super::{
    HostRequest, HostRole, Hosted, HostedApp, ParkedEditApp, PendingApp, PendingPlayApp,
    PlayFailed, PlayStarted, PlayState, PlayStopped,
    events::forward_events,
    input::{HostCursor, Routing, ViewportShown, ViewportView},
    seam::EditWorldSignals,
    target::SetEditTarget,
};
use crate::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument, PreviewSpec, Unsaved,
    },
    guard::UnsavedGuard,
    hierarchy::HierarchyState,
    inspector::{ComponentEdits, InspectorData, PropertyCommits},
    preview::PreviewSignalQueue,
    project::{Project, ProjectState},
    scene::{
        SaveFailed, SceneEditor, SceneEditorState, SceneOpenFailed, SceneOpened, SceneRecipe,
        SceneSaved, SceneState,
    },
    tools::{
        edits::{ComponentEdited, PropertyCommitted},
        entity_ops::EntityEdit,
        scene::{RevealEntity, SceneEdited, SceneRoot, SceneStatus},
        selection::Selection,
    },
};

#[derive(Event, Clone, Debug, PartialEq)]
pub struct Echo(pub u32);

pub struct Say(pub u32);
impl Signal for Say {}

#[derive(Resource, Default)]
pub struct Frames(pub usize);

fn say(on: On<Say>, mut echo: EventWriter<Echo>) {
    echo.write(Echo(on.signal().0));
}

fn count_frame(mut frames: ResMut<Frames>) {
    frames.0 += 1;
}

#[derive(Resource, Default)]
pub struct Said(pub Vec<(u32, usize)>);

pub fn remember_said(on: On<Say>, frames: Res<Frames>, mut said: ResMut<Said>) {
    said.0.push((on.signal().0, frames.0));
}

fn previewed_build(app: &mut App) {
    app.insert_resource(Said::default())
        .add_listener(remember_said.at_preview_time());
}

fn broken_build(_: &mut App) {
    panic!("the preview broke");
}

macro_rules! test_asset {
    ($asset:ident, $editor:ident, $preview:expr, $edits_in_preview:expr) => {
        #[derive(serde::Serialize, serde::Deserialize)]
        pub struct $asset;

        impl Asset for $asset {
            fn name() -> &'static str {
                stringify!($asset)
            }
        }

        #[derive(Default)]
        pub struct $editor;

        impl AssetEditor for $editor {
            fn preview(&self) -> Option<PreviewSpec> {
                $preview
            }

            fn edits_in_preview(&self) -> bool {
                $edits_in_preview
            }
        }
    };
}

test_asset!(
    PreviewedAsset,
    PreviewedEditor,
    Some(PreviewSpec {
        build: previewed_build
    }),
    false
);
test_asset!(
    EditedPreviewAsset,
    EditedPreviewEditor,
    Some(PreviewSpec {
        build: previewed_build
    }),
    true
);
test_asset!(
    BrokenPreviewAsset,
    BrokenPreviewEditor,
    Some(PreviewSpec {
        build: broken_build
    }),
    false
);
test_asset!(PlainAsset, PlainEditor, None, false);

pub fn plain_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_event::<Echo>()
        .insert_resource(Frames::default())
        .add_system(Update, count_frame.at_edit_time())
        .add_listener(say.at_edit_time());
}

pub fn request(recipe: fn(&mut App)) -> HostRequest<'static> {
    HostRequest {
        gpu: None,
        recipe,
        size: (320, 240),
        scale_factor: 1.0,
        role: HostRole::Edit,
        clipboard: None,
    }
}

pub fn play_request(recipe: fn(&mut App)) -> HostRequest<'static> {
    HostRequest {
        role: HostRole::Game,
        ..request(recipe)
    }
}

pub fn preview_request(recipe: fn(&mut App)) -> HostRequest<'static> {
    HostRequest {
        role: HostRole::Preview,
        ..request(recipe)
    }
}

pub fn start_play(
    world: &mut World,
    document: Entity,
    recipe: fn(&mut App),
    extra: impl FnOnce(&mut App),
) {
    let pending = super::begin(play_request(recipe), extra, |_| {});
    world.insert(PendingPlayApp::new(pending), document);
    world.insert(PlayState::Starting, document);
    world.trigger(SetEditTarget(None));
}

pub fn finish_play(world: &mut World) {
    let mut system = super::finish_pending_play.into_system();
    system.initialize(world);
    system.run_and_apply((), world);
}

pub fn play_now(
    world: &mut World,
    document: Entity,
    recipe: fn(&mut App),
    extra: impl FnOnce(&mut App),
) {
    start_play(world, document, recipe, extra);
    finish_play(world);
}

pub fn play_state(world: &World, document: Entity) -> Option<PlayState> {
    world
        .get_component_for_entity::<PlayState>(document)
        .copied()
}

pub fn hosted_now(recipe: fn(&mut App), extra: impl FnOnce(&mut App)) -> HostedApp {
    super::begin(request(recipe), extra, |_| {})
        .finish_now()
        .expect("a recipe without asynchronous plugins is ready at once")
}

pub fn preview_now(recipe: fn(&mut App)) -> HostedApp {
    super::begin(preview_request(recipe), |_| {}, |_| {})
        .finish_now()
        .expect("a recipe without asynchronous plugins is ready at once")
}

fn insert_play_outcomes(world: &mut World) {
    world.insert_resource(EventChannel::<PlayStarted>::new());
    world.insert_resource(EventChannel::<PlayFailed>::new());
    world.insert_resource(EventChannel::<PlayStopped>::new());
}

pub fn host_world(recipe: fn(&mut App)) -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(ActiveEditor::default());
    world.insert_resource(Routing::default());
    insert_play_outcomes(&mut world);
    super::install_listeners(&mut world);
    let document = world.spawn(hosted_now(recipe, |_| {}));
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
    world.trigger(SetEditTarget(Some(document)));
    (world, document)
}

pub fn scene_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_scene::plugin::ScenePlugin);
}

#[derive(Resource, Default)]
pub struct GameRan(pub bool);

fn note_game_ran(mut ran: ResMut<GameRan>) {
    ran.0 = true;
}

#[derive(Resource)]
struct HoldReady;

struct ReadyUnlessHeld;

impl concerto_app::Plugin for ReadyUnlessHeld {
    fn build(&self, _: &mut App) {}

    fn ready(&self, app: &App) -> bool {
        app.get_resource::<HoldReady>().is_none()
    }
}

fn fixture_recipe(app: &mut App) {
    scene_recipe(app);
    app.register_plugin(ReadyUnlessHeld);
    app.insert_resource(Frames::default())
        .insert_resource(GameRan::default())
        .add_system(Update, note_game_ran)
        .add_system(Update, count_frame.at_edit_time());
}

pub fn scene_entry(name: &str) -> AssetEntry {
    AssetEntry {
        id: AssetId::new(),
        address: format!("{name}.gasset"),
        kind: Scene::name().into(),
        provenance: Some(ImportProvenance {
            source: "fixture".into(),
            sub_asset: name.into(),
        }),
    }
}

pub fn write_scene_file(root: &std::path::Path, asset: &AssetEntry, nodes: &[&str]) {
    let nodes = nodes
        .iter()
        .map(|name| {
            let mut node = SceneNode {
                name: (*name).into(),
                children: vec![],
                components: vec![],
            };
            node.push_component(&Transform::IDENTITY).unwrap();
            node
        })
        .collect();
    let scene = Scene {
        nodes,
        referenced_assets: vec![],
    };
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: asset.id,
        references: vec![],
        kind: Scene::name().into(),
        provenance: asset.provenance.clone(),
    };
    std::fs::write(
        root.join(&asset.address),
        write_content_asset(&header, &bincode::serialize(&scene).unwrap()).unwrap(),
    )
    .unwrap();
}

fn build_hierarchy_panel(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    crate::hierarchy::spawn_panel(&mut cmd, parent, &theme);
}

pub struct SceneFixture {
    pub world: World,
    pub project_root: PathBuf,
    pub scene: AssetEntry,
    pub other: AssetEntry,
    systems: Vec<BoxedSystem>,
    process_commands: usize,
    delivery: Vec<BoxedSystem>,
    _directory: tempfile::TempDir,
}

impl Default for SceneFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneFixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let project_root = directory.path().to_path_buf();
        let scene = scene_entry("first");
        let other = scene_entry("other");
        write_scene_file(&project_root, &scene, &["first", "second"]);
        write_scene_file(&project_root, &other, &["third"]);

        let mut world = World::new();
        world.insert_resource(SceneState::default());
        world.insert_resource(SceneRecipe {
            recipe: fixture_recipe,
            gpu: None,
            clipboard: None,
        });
        world.insert_resource(HierarchyState::default());
        world.insert_resource(InspectorData::default());
        world.insert_resource(PropertyCommits::default());
        world.insert_resource(ComponentEdits::default());
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(UnsavedGuard::default());
        let mut editors = AssetEditorRegistry::default();
        editors.register_creatable::<Scene, SceneEditor>().unwrap();
        editors
            .register::<PreviewedAsset, PreviewedEditor>()
            .unwrap();
        editors
            .register::<EditedPreviewAsset, EditedPreviewEditor>()
            .unwrap();
        editors
            .register::<BrokenPreviewAsset, BrokenPreviewEditor>()
            .unwrap();
        editors.register::<PlainAsset, PlainEditor>().unwrap();
        world.insert_resource(editors);
        world.insert_resource(PreviewSignalQueue::default());
        let mut project = ProjectState::default();
        let mut registry = AssetRegistry::default();
        registry.insert(scene.clone());
        registry.insert(other.clone());
        project.project = Some(Project {
            root: project_root.clone(),
            content_root: "content".into(),
        });
        world.insert_resource(project);
        world.insert_resource(registry);

        world.insert_resource(Routing::default());
        world.insert_resource(HostCursor::default());
        world.insert_resource(ViewportShown(Some(ViewportView {
            origin: glam::Vec2::ZERO,
            size: (640, 480),
            over: true,
            typing: false,
        })));
        world.insert_resource(Window::new((1280, 720), 1.0));
        world.insert_resource(EventChannel::<WindowEvent>::new());
        world.insert_resource(concerto_window::input::Input::new());
        world.insert_resource(EventChannel::<SceneOpened>::new());
        world.insert_resource(EventChannel::<SceneOpenFailed>::new());
        world.insert_resource(EventChannel::<SceneSaved>::new());
        world.insert_resource(EventChannel::<SaveFailed>::new());
        insert_play_outcomes(&mut world);
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        world.insert_resource(UITheme::default());
        world.insert_resource(FocusedWidget::default());
        super::install_listeners(&mut world);
        crate::scene::install_listeners(&mut world);
        crate::hierarchy::install_listeners(&mut world);
        crate::inspector::install_listeners(&mut world);
        concerto_ui::items::install_listeners(&mut world);
        concerto_ui::tree::install_listeners(&mut world);
        let mut panel = build_hierarchy_panel.into_system();
        panel.initialize(&mut world);
        panel.run_and_apply((), &mut world);

        let mut systems = vec![
            super::input::sync_in_viewport.into_boxed_system(),
            crate::scene::refuse_edited_replacements.into_boxed_system(),
            super::forward_registry.into_boxed_system(),
            super::finish_pending.into_boxed_system(),
            super::finish_pending_play.into_boxed_system(),
            crate::preview::deliver_preview_signals.into_boxed_system(),
            super::input::step_viewport_target.into_boxed_system(),
        ];
        let process_commands = systems.len();
        systems.extend([
            crate::asset_editor::process_editor_commands.into_boxed_system(),
            crate::scene::update_scenes.into_boxed_system(),
            crate::scene::request_scene_saves.into_boxed_system(),
            crate::hierarchy::reconcile_structure.into_boxed_system(),
            concerto_ui::tree::update_trees_system(),
            crate::hierarchy::reconcile_content.into_boxed_system(),
            crate::inspector::collect_inspector_data.into_boxed_system(),
            crate::scene::show_play_failures.into_boxed_system(),
            crate::preview::open_previews.into_boxed_system(),
            crate::preview::report_preview_failures.into_boxed_system(),
            crate::preview::keep_preview_failures_shown.into_boxed_system(),
        ]);
        for system in &mut systems {
            system.initialize(&mut world);
        }
        world.insert_resource(EventChannel::<Hosted<SceneEdited>>::new());
        world.insert_resource(EventChannel::<Hosted<SceneStatus>>::new());
        world.insert_resource(EventChannel::<Hosted<RevealEntity>>::new());
        world.insert_resource(EventChannel::<Hosted<PropertyCommitted>>::new());
        world.insert_resource(EventChannel::<Hosted<ComponentEdited>>::new());
        let mut delivery = vec![
            forward_events::<SceneEdited>.into_boxed_system(),
            forward_events::<SceneStatus>.into_boxed_system(),
            forward_events::<RevealEntity>.into_boxed_system(),
            forward_events::<PropertyCommitted>.into_boxed_system(),
            forward_events::<ComponentEdited>.into_boxed_system(),
            crate::scene::follow_unsaved_edits.into_boxed_system(),
            crate::scene::show_status.into_boxed_system(),
            crate::scene::reveal.into_boxed_system(),
        ];
        for system in &mut delivery {
            system.initialize(&mut world);
        }

        Self {
            world,
            project_root,
            scene,
            other,
            systems,
            process_commands,
            delivery,
            _directory: directory,
        }
    }

    pub fn open(&mut self) -> Entity {
        let scene = self.scene.clone();
        self.request(&scene);
        self.frames_until_idle();
        let document = self
            .world
            .get_resource::<ActiveEditor>()
            .unwrap()
            .0
            .expect("opening a scene makes its document the active one");
        assert!(
            self.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some(),
            "{}",
            self.status(document)
        );
        document
    }

    pub fn request_kind(&mut self, kind: &str) -> Entity {
        let asset = AssetEntry {
            id: AssetId::new(),
            address: format!("{kind}.gasset"),
            kind: kind.into(),
            provenance: Some(ImportProvenance {
                source: "fixture".into(),
                sub_asset: kind.into(),
            }),
        };
        self.add_to_project(&asset);
        self.request(&asset);
        self.world
            .get_resource::<ActiveEditor>()
            .unwrap()
            .0
            .expect("opening an asset makes its document the active one")
    }

    pub fn open_preview(&mut self) -> Entity {
        self.open_preview_of(PreviewedAsset::name())
    }

    pub fn open_preview_of(&mut self, kind: &str) -> Entity {
        let document = self.request_kind(kind);
        self.frames_until("the preview was never built", |f| {
            f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
        });
        document
    }

    pub fn frame(&mut self) {
        for system in &mut self.systems {
            system.run_and_apply((), &mut self.world);
        }
        self.deliver();
        self.world.tick();
    }

    pub fn activate(&mut self, document: Option<Entity>) {
        self.world.get_resource_mut::<ActiveEditor>().unwrap().0 = document;
        self.world.trigger(SetEditTarget(document));
    }

    pub fn deliver(&mut self) {
        for system in &mut self.delivery {
            system.run_and_apply((), &mut self.world);
        }
    }

    pub fn add_delivery(&mut self, mut system: BoxedSystem) {
        system.initialize(&mut self.world);
        self.delivery.push(system);
    }

    pub fn frames_until_idle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.frame();
            if self.idle() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the scene never finished loading"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn idle(&mut self) -> bool {
        let pending = self
            .world
            .query::<&PendingApp, ()>()
            .iter(&mut self.world)
            .count();
        pending == 0
            && self
                .world
                .query::<(&EditorDocument, &SceneEditorState), ()>()
                .iter(&mut self.world)
                .all(|(document, state)| document.pending.is_none() && state.idle())
    }

    pub fn frames_until_building(&mut self, document: Entity) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self
            .world
            .get_component_for_entity::<PendingApp>(document)
            .is_none()
        {
            self.frame();
            assert!(Instant::now() < deadline, "nothing was built");
            std::thread::sleep(Duration::from_millis(1));
        }
        self.world
            .get_component_for_entity_mut::<PendingApp>(document)
            .and_then(|pending| pending.hosted.as_mut())
            .expect("the build is held before it is first asked if it is ready")
            .app
            .insert_resource(HoldReady);
    }

    pub fn wait_until_the_build_is_ready(&mut self, document: Entity) {
        if let Some(hosted) = self
            .world
            .get_component_for_entity_mut::<PendingApp>(document)
            .and_then(|pending| pending.hosted.as_mut())
        {
            hosted
                .app
                .main_mut()
                .world_mut()
                .remove_resource::<HoldReady>();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let ready = self
                .world
                .get_component_for_entity_mut::<PendingApp>(document)
                .and_then(|pending| pending.hosted.as_mut())
                .is_some_and(|hosted| {
                    hosted.app.plugin_state() == concerto_app::plugins::PluginsState::Ready
                });
            if ready {
                return;
            }
            assert!(Instant::now() < deadline, "the build never became ready");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn add_scene(&mut self, name: &str) -> AssetEntry {
        let asset = scene_entry(name);
        write_scene_file(&self.project_root, &asset, &[name]);
        self.add_to_project(&asset);
        asset
    }

    pub fn add_to_project(&mut self, asset: &AssetEntry) {
        self.world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(asset.clone());
    }

    pub fn request(&mut self, asset: &AssetEntry) {
        self.command(AssetEditorCommand::Open(asset.clone()));
    }

    pub fn request_other_scene(&mut self, document: Entity) {
        let other = self.other.clone();
        self.request(&other);
        assert_eq!(
            self.world.get_resource::<ActiveEditor>().unwrap().0,
            Some(document)
        );
    }

    pub fn close(&mut self, document: Entity) {
        self.command(AssetEditorCommand::Close(document));
    }

    pub fn command(&mut self, command: AssetEditorCommand) {
        self.world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(command);
        self.systems[self.process_commands].run_and_apply((), &mut self.world);
    }

    pub fn signal<S: Signal>(&mut self, signal: S) {
        let mut signal = Some(signal);
        let mut system = (move |mut scene: EditWorldSignals| {
            if let Some(signal) = signal.take() {
                scene.trigger(signal);
            }
        })
        .into_system();
        system.initialize(&mut self.world);
        system.run_and_apply((), &mut self.world);
        self.deliver();
    }

    pub fn rename(&mut self, document: Entity, name: &str, renamed: &str) {
        let entity = self.scene_entity(document, name);
        self.signal(EntityEdit::Rename {
            entity,
            name: renamed.into(),
        });
    }

    pub fn scene_world(&self, document: Entity) -> &World {
        self.world
            .get_component_for_entity::<HostedApp>(document)
            .expect("the document hosts a scene")
            .world()
    }

    pub fn scene_world_mut(&mut self, document: Entity) -> &mut World {
        self.world
            .get_component_for_entity_mut::<HostedApp>(document)
            .expect("the document hosts a scene")
            .app
            .main_mut()
            .world_mut()
    }

    pub fn scene_names(&mut self, document: Entity) -> Vec<String> {
        let scene = self.scene_world_mut(document);
        scene
            .query::<&Name, ()>()
            .iter(scene)
            .map(|name| name.as_str().to_owned())
            .collect()
    }

    pub fn scene_entity(&mut self, document: Entity, name: &str) -> Entity {
        let scene = self.scene_world_mut(document);
        scene
            .query::<(Entity, &Name), ()>()
            .iter(scene)
            .find(|(_, found)| found.as_str() == name)
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("the scene has no node called {name}"))
    }

    pub fn scene_root(&mut self, document: Entity) -> Entity {
        let scene = self.scene_world_mut(document);
        scene
            .query::<(Entity, &SceneRoot), ()>()
            .iter(scene)
            .map(|(entity, _)| entity)
            .next()
            .expect("an open scene has a root")
    }

    pub fn selected(&self, document: Entity) -> Option<Entity> {
        self.scene_world(document)
            .get_resource::<Selection>()
            .unwrap()
            .entity()
    }

    pub fn unsaved(&mut self, document: Entity) -> bool {
        self.deliver();
        self.world
            .get_component_for_entity::<Unsaved>(document)
            .is_some()
    }

    pub fn opened(&self) -> Vec<SceneOpened> {
        self.written()
    }

    pub fn open_failures(&self) -> Vec<SceneOpenFailed> {
        self.written()
    }

    pub fn written<E: Event + Clone + 'static>(&self) -> Vec<E> {
        self.world
            .get_resource::<EventChannel<E>>()
            .unwrap()
            .read_from(&mut EventCursor::default())
            .cloned()
            .collect()
    }

    pub fn status(&self, document: Entity) -> String {
        self.document(document).status.clone()
    }

    pub fn document(&self, document: Entity) -> &EditorDocument {
        self.world
            .get_component_for_entity::<EditorDocument>(document)
            .expect("the document is open")
    }

    pub fn saved_scene(&self) -> Scene {
        let bytes = std::fs::read(self.project_root.join(&self.scene.address)).unwrap();
        let (_, payload) = read_content_asset(&bytes).unwrap();
        bincode::deserialize(payload).unwrap()
    }

    pub fn saved_scene_names(&self) -> Vec<String> {
        self.saved_scene()
            .nodes
            .into_iter()
            .map(|node| node.name)
            .collect()
    }

    pub fn make_project_unwritable(&mut self) {
        let file = self.project_root.join(&self.scene.address);
        self.world
            .get_resource_mut::<ProjectState>()
            .unwrap()
            .project
            .as_mut()
            .unwrap()
            .root = file;
    }

    pub fn hosted_frames(&self, document: Entity) -> usize {
        self.scene_world(document)
            .get_resource::<Frames>()
            .unwrap()
            .0
    }

    /// Whether `world`'s registry and its asset server both know `id`.
    pub fn knows(&self, world: &World, id: AssetId) -> bool {
        let resource = world
            .get_resource::<AssetRegistry>()
            .is_some_and(|registry| registry.get(id).is_some());
        let server = world
            .get_resource::<AssetServer>()
            .and_then(AssetServer::registry)
            .is_some_and(|registry| registry.get(id).is_some());
        resource && server
    }

    pub fn play_state(&self, document: Entity) -> Option<PlayState> {
        play_state(&self.world, document)
    }

    pub fn parked_world(&self, document: Entity) -> &World {
        self.world
            .get_component_for_entity::<ParkedEditApp>(document)
            .expect("the edit App is parked")
            .0
            .world()
    }

    pub fn hosted_is_editing(&self, document: Entity) -> bool {
        self.world
            .get_component_for_entity::<HostedApp>(document)
            .expect("the document hosts an App")
            .app
            .is_editing()
    }

    pub fn starting_world(&self, document: Entity) -> Option<WorldId> {
        self.world
            .get_component_for_entity::<PendingPlayApp>(document)
            .and_then(|pending| pending.0.hosted.as_ref())
            .map(|hosted| hosted.world().id())
    }

    pub fn game_ran(&self, document: Entity) -> bool {
        self.scene_world(document)
            .get_resource::<GameRan>()
            .unwrap()
            .0
    }

    pub fn frames_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.frame();
            if done(self) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn frames_until_playing(&mut self, document: Entity) {
        self.frames_until("the game never started", |f| {
            f.play_state(document) == Some(PlayState::Playing)
        });
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct GatedAsset;

impl concerto_foundation::assets::Asset for GatedAsset {
    fn name() -> &'static str {
        "GatedAsset"
    }
}

pub static ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static RELEASED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static GATED_DROPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn lock_gate() -> std::sync::MutexGuard<'static, ()> {
    let guard = GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ENTERED.store(false, std::sync::atomic::Ordering::SeqCst);
    RELEASED.store(false, std::sync::atomic::Ordering::SeqCst);
    guard
}

pub fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::yield_now();
    }
}

impl concerto_foundation::assets::LoadableAsset for GatedAsset {
    fn on_load(
        &mut self,
        _: &concerto_foundation::assets::asset_server::AssetLoadContext,
    ) -> anyhow::Result<()> {
        ENTERED.store(true, std::sync::atomic::Ordering::SeqCst);
        wait_for("the test never released the load", || {
            RELEASED.load(std::sync::atomic::Ordering::SeqCst)
        });
        Ok(())
    }
}

impl Drop for GatedAsset {
    fn drop(&mut self) {
        GATED_DROPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
