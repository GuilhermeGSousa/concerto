//! A game asset kind edited in the editor's generic inspector tab. Run with `cargo run -p concerto-editor --example custom_asset`.
use anyhow::Result;
use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::{Startup, Update},
};
use concerto_ecs::{
    Entity, Res, World,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    events::event_channel::EventChannel,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument,
        SaveRequested, Unsaved, process_editor_commands,
    },
    content::ContentPlugin,
    dock::PanelRegistry,
    guard::UnsavedGuard,
    hosted::{
        HostedPlugin,
        input::{ViewportShown, ViewportView},
    },
    inspected_asset::{InspectedAssetPlugin, InspectedDocument},
    inspector::{InspectorPlugin, PropertyRow, spawn_panel},
    preview::PreviewHostPlugin,
    project::{EditorCommand, EditorCommands, ProjectPlugin, ProjectState},
    scene::ScenePlugin,
};
use concerto_foundation::assets::{
    Asset, AssetId, CreatableAsset, LoadableAsset,
    asset_server::AssetServer,
    content::{decode_content_asset, save_content_asset_into},
    registry::{AssetEntry, AssetRegistry},
};
use concerto_reflect::Reflect;
use concerto_scene::scene::Scene;
use concerto_ui::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    node::UINode,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::Vec2;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(serde::Serialize, serde::Deserialize, Reflect, Debug, PartialEq)]
pub struct Dialogue {
    pub text: String,
    pub spin_speed: f32,
}
impl Asset for Dialogue {
    fn name() -> &'static str {
        "ExampleDialogue"
    }
}
impl CreatableAsset for Dialogue {
    fn new_asset() -> Self {
        Dialogue {
            text: "Hello".into(),
            spin_speed: 2.0,
        }
    }
}
impl LoadableAsset for Dialogue {}

/// What the game adds for the editor: `Dialogue` as an inspected asset kind.
pub fn register(app: &mut App) {
    app.register_inspected_asset::<Dialogue>();
}

/// The game recipe of the smoke test: no renderer, so a scene builds without a GPU.
pub fn recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(concerto_scene::plugin::ScenePlugin);
    register(app);
}

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
    register(app);
}

/// Writes a project holding one empty scene under `root`.
pub fn write_project(root: &Path) -> Result<AssetId> {
    std::fs::create_dir_all(root.join("content"))?;
    save_content_asset_into(
        &Scene::new_asset(),
        root,
        "content/level.gasset",
        &mut AssetRegistry::default(),
    )
}

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

fn editor_app(root: &Path) -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(PanelRegistry::default())
        .insert_resource(UITheme::default())
        .insert_resource(FocusedWidget::default())
        .insert_resource(AssetServer::new())
        .insert_resource(AssetRegistry::default())
        .insert_resource(ProjectState::default())
        .insert_resource(EditorCommands::default())
        .insert_resource(AssetEditorRegistry::default())
        .insert_resource(AssetEditorCommands::default())
        .insert_resource(ActiveEditor::default())
        .insert_resource(UnsavedGuard::default())
        .insert_resource(Window::new((64, 64), 1.0))
        .insert_resource(ViewportShown(Some(ViewportView {
            origin: Vec2::ZERO,
            size: (64, 64),
            over: false,
            typing: false,
        })))
        .register_event::<UIFocusGained>()
        .register_event::<UIFocusLost>()
        .register_event::<UITextInputSubmitted>()
        .register_event::<UITextInputCancelled>()
        .register_plugin(ProjectPlugin)
        .register_plugin(HostedPlugin)
        .register_plugin(InspectorPlugin)
        .register_plugin(PreviewHostPlugin)
        .register_plugin(ScenePlugin {
            recipe,
            clipboard: None,
        })
        .register_plugin(InspectedAssetPlugin)
        .register_plugin(ContentPlugin)
        .add_system(Startup, build_inspector)
        .add_system(Update, process_editor_commands);
    app.get_resource_mut::<EditorCommands>()
        .expect("EditorCommands was just inserted")
        .0
        .push_back(EditorCommand::OpenProject(root.into()));
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    app
}

fn frames_until(app: &mut App, what: &str, done: impl Fn(&mut App) -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(app) {
        anyhow::ensure!(Instant::now() < deadline, "{what}");
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn world(app: &mut App) -> &mut World {
    app.main_mut().world_mut()
}

fn lists_dialogue(app: &mut App) -> bool {
    world(app)
        .get_resource::<AssetEditorRegistry>()
        .unwrap()
        .creatable()
        .any(|(kind, _)| kind == Dialogue::name())
}

fn create_dialogue(app: &mut App, root: &Path, address: &str) -> Result<AssetEntry> {
    let world = world(app);
    let mut assets = std::mem::take(world.get_resource_mut::<AssetRegistry>().unwrap());
    let created = world
        .get_resource::<AssetEditorRegistry>()
        .unwrap()
        .creatable()
        .find(|(kind, _)| *kind == Dialogue::name())
        .map(|(_, creator)| creator.create(root, address, &mut assets))
        .expect("ExampleDialogue is creatable");
    let entry = created.map(|id| assets.get(id).cloned());
    *world.get_resource_mut::<AssetRegistry>().unwrap() = assets;
    entry?.ok_or_else(|| anyhow::anyhow!("the new dialogue is not in the registry"))
}

fn open_dialogue(app: &mut App, asset: &AssetEntry) -> Result<Entity> {
    world(app)
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::Open(asset.clone()));
    frames_until(app, "the dialogue never opened", |app| {
        held_dialogue(app).is_some()
    })?;
    let document = world(app).get_resource::<ActiveEditor>().unwrap().0;
    document.ok_or_else(|| anyhow::anyhow!("the dialogue's tab is not active"))
}

fn held_dialogue(app: &mut App) -> Option<&Dialogue> {
    let world = world(app);
    let document = world.get_resource::<ActiveEditor>()?.0?;
    world
        .get_component_for_entity::<InspectedDocument>(document)?
        .value
        .as_deref()?
        .downcast_ref::<Dialogue>()
}

fn field_of(world: &mut World, path: &str) -> Option<Entity> {
    let (row, _) = world
        .query::<(Entity, &PropertyRow), ()>()
        .iter(world)
        .find(|(_, row)| {
            row.address.target.name() == Dialogue::name() && row.address.path.to_string() == path
        })?;
    text_field(world, row)
}

fn text_field(world: &World, entity: Entity) -> Option<Entity> {
    if world
        .get_component_for_entity::<UITextInput>(entity)
        .is_some()
    {
        return Some(entity);
    }
    world
        .get_component_for_entity::<Children>(entity)?
        .iter()
        .find_map(|child| text_field(world, *child))
}

fn type_into(app: &mut App, path: &str, value: &str) -> Result<()> {
    frames_until(app, &format!("the inspector never showed {path}"), |app| {
        field_of(world(app), path).is_some()
    })?;
    let world = world(app);
    let field = field_of(world, path).unwrap();
    world
        .get_component_for_entity_mut::<UITextInput>(field)
        .unwrap()
        .value = value.into();
    world
        .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
        .unwrap()
        .push_event(UITextInputSubmitted {
            entity: field,
            value: value.into(),
        });
    app.update();
    Ok(())
}

fn unsaved(app: &mut App, document: Entity) -> bool {
    world(app)
        .get_component_for_entity::<Unsaved>(document)
        .is_some()
}

/// Opens a project with a scene, creates a Dialogue once the scene's game names it, edits it through the inspector and saves it.
pub fn smoke_test() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    write_project(&root)?;
    let mut app = editor_app(&root);

    frames_until(&mut app, "ExampleDialogue never became creatable", |app| {
        lists_dialogue(app)
    })?;
    let address = "content/greeting.gasset";
    let asset = create_dialogue(&mut app, &root, address)?;
    let document = open_dialogue(&mut app, &asset)?;
    assert_eq!(held_dialogue(&mut app), Some(&Dialogue::new_asset()));
    assert!(!unsaved(&mut app, document));

    type_into(&mut app, "text", "Goodbye")?;
    type_into(&mut app, "spin_speed", "5")?;
    let edited = Dialogue {
        text: "Goodbye".into(),
        spin_speed: 5.0,
    };
    frames_until(&mut app, "the edits never reached the dialogue", |app| {
        held_dialogue(app) == Some(&edited)
    })?;
    assert!(unsaved(&mut app, document));

    world(&mut app).insert(SaveRequested, document);
    frames_until(&mut app, "the dialogue never saved", |app| {
        !unsaved(app, document)
    })?;
    let status = &world(&mut app)
        .get_component_for_entity::<EditorDocument>(document)
        .unwrap()
        .status;
    assert_eq!(status, &format!("Saved {address}"));
    let (header, saved) = decode_content_asset::<Dialogue>(&std::fs::read(root.join(address))?)?;
    assert_eq!(header.asset_id, asset.id);
    assert_eq!(saved, edited);
    Ok(())
}

#[allow(dead_code)]
fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    write_project(directory.path())?;
    concerto_editor::run(game, directory.path());
    Ok(())
}
