//! Exercise the real tab systems without a window or GPU.
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity, Resource, World, events::event_channel::EventChannel, resource::ResMut, signal::On,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument,
    },
    hosted::{PlayState, StopPlay},
    scene::{Play, SceneEditor},
    tabs::{
        EditorTab, EditorTabClose, EditorTabPlay, TabScroll, TabStrip, TabStripContent, TabsPlugin,
    },
};
use concerto_foundation::assets::Asset;
use concerto_scene::scene::Scene;
use concerto_ui::{
    interaction::{HoveredNode, UIClick},
    node::{UIBox, UILayout, UINode},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::events::{ScrollDelta, WindowEvent};
use concerto_window::input::MouseButton;
use glam::Vec2;

fn layout(x: f32, width: f32) -> UILayout {
    let rect = UIBox {
        min: Vec2::new(x, 0.0),
        size: Vec2::new(width, 30.0),
    };
    UILayout {
        rect,
        content_rect: rect,
        clip_rect: rect,
        paint_order: 0,
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Notes;
impl Asset for Notes {
    fn name() -> &'static str {
        "Notes"
    }
}
#[derive(Default)]
struct NotesEditor;
impl AssetEditor for NotesEditor {}

struct Strip {
    app: App,
    strip: Entity,
    content: Entity,
}

fn setup() -> Strip {
    let mut app = App::new();
    let mut registry = AssetEditorRegistry::default();
    registry.register::<Scene, SceneEditor>().unwrap();
    registry.register::<Notes, NotesEditor>().unwrap();
    app.insert_resource(registry);
    app.insert_resource(ActiveEditor::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UITheme::default());
    app.insert_resource(HoveredNode::default());
    app.register_event::<WindowEvent>();
    app.register_plugin(TabsPlugin);
    let world = app.main_mut().world_mut();
    let strip = world.spawn((
        TabStrip,
        TabScroll::default(),
        UINode::default(),
        layout(0.0, 100.0),
    ));
    let content = world.spawn((TabStripContent, UINode::default(), layout(0.0, 300.0)));
    world.entity_mut(strip).add_child(content);
    app.finish_plugin_build();
    Strip {
        app,
        strip,
        content,
    }
}
fn document(world: &mut World, order: u64) -> Entity {
    document_of(world, "Document", order)
}
fn document_of(world: &mut World, asset_type: &'static str, order: u64) -> Entity {
    world.spawn(EditorDocument {
        asset_type,
        title: format!("Document {order}"),
        current: None,
        pending: None,
        order,
        status: String::new(),
        failed: None,
    })
}
fn tick(app: &mut App) {
    app.main_mut().world_mut().run_schedule(LateUpdate);
}
#[derive(Resource, Default)]
struct Heard(Vec<String>);

fn heard_play(on: On<Play>, mut heard: ResMut<Heard>) {
    heard
        .0
        .push(format!("play {}", on.signal().0.to_id_string()));
}

fn heard_stop(on: On<StopPlay>, mut heard: ResMut<Heard>) {
    heard
        .0
        .push(format!("stop {}", on.signal().0.to_id_string()));
}

fn click(world: &mut World, target: Entity) {
    world.trigger_on(
        target,
        UIClick {
            position: Vec2::ZERO,
            button: MouseButton::Left,
        },
    );
}

fn tab_of(world: &mut World, document: Entity) -> Entity {
    world
        .query::<(Entity, &EditorTab), ()>()
        .iter(world)
        .find(|(_, tab)| tab.document == document)
        .unwrap()
        .0
}

fn close_of(world: &mut World, document: Entity) -> Option<Entity> {
    world
        .query::<(Entity, &EditorTabClose), ()>()
        .iter(world)
        .find(|(_, close)| close.document == document)
        .map(|(entity, _)| entity)
}

fn play_of(world: &mut World, document: Entity) -> Option<Entity> {
    world
        .query::<(Entity, &EditorTabPlay), ()>()
        .iter(world)
        .find(|(_, play)| play.document == document)
        .map(|(entity, _)| entity)
}

#[test]
fn a_tab_activates_its_document_its_close_closes_it_and_a_scene_tabs_play_plays_or_stops() {
    let Strip { mut app, .. } = setup();
    let world = app.main_mut().world_mut();
    world.insert_resource(Heard::default());
    world.add_listener(heard_play);
    world.add_listener(heard_stop);
    let notes = document_of(world, Notes::name(), 1);
    let other = document(world, 2);
    let scene = document_of(world, Scene::name(), 3);
    world.insert(PlayState::Editing, scene);
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(scene);
    tick(&mut app);

    let world = app.main_mut().world_mut();
    let tab = tab_of(world, other);
    click(world, tab);
    let close = close_of(world, notes).expect("a custom editor's tab closes");
    click(world, close);
    assert!(play_of(world, notes).is_none());
    assert!(close_of(world, scene).is_none());
    let play = play_of(world, scene).expect("the scene tab plays");
    click(world, play);
    world.insert(PlayState::Playing, scene);
    click(world, play);

    let id = scene.to_id_string();
    assert_eq!(
        app.get_resource::<Heard>().unwrap().0,
        [format!("play {id}"), format!("stop {id}")]
    );
    let commands = app.get_resource_mut::<AssetEditorCommands>().unwrap();
    assert!(
        matches!(commands.0.pop_front(), Some(AssetEditorCommand::Activate(entity)) if entity == other)
    );
    assert!(
        matches!(commands.0.pop_front(), Some(AssetEditorCommand::Close(entity)) if entity == notes)
    );
    assert!(commands.0.is_empty());
}

#[test]
fn the_strip_scrolls_to_the_active_tab_and_the_wheel_stops_at_the_first() {
    let Strip {
        mut app,
        strip,
        content,
    } = setup();
    let world = app.main_mut().world_mut();
    let a = document(world, 1);
    let b = document(world, 2);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    let tab_a = tab_of(world, a);
    let tab_b = tab_of(world, b);
    world.insert(layout(0.0, 150.0), tab_a);
    world.insert(layout(150.0, 150.0), tab_b);
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(b);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert_eq!(
        world
            .get_component_for_entity::<UINode>(content)
            .unwrap()
            .inset
            .left,
        UIValue::Px(-200.0)
    );
    **world.get_resource_mut::<HoveredNode>().unwrap() = Some(strip);
    world
        .get_resource_mut::<EventChannel<WindowEvent>>()
        .unwrap()
        .push_event(WindowEvent::MouseWheel {
            delta: ScrollDelta::Lines(glam::Vec2::new(0.0, 100.0)),
        });
    tick(&mut app);
    assert_eq!(
        app.main()
            .world()
            .get_component_for_entity::<UINode>(content)
            .unwrap()
            .inset
            .left,
        UIValue::Px(0.0)
    );
}
