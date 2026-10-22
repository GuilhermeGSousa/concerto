//! Exercise the real tab systems without a window or GPU.
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity, IntoSetConfig, IntoSystemConfig, Query, Resource, World,
    entity::hierarchy::{ChildOf, Children},
    events::event_channel::EventChannel,
    resource::{Res, ResMut},
    signal::On,
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
        EditorDocument, Unsaved,
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
    interaction::{HoveredNode, UIClick, UIInteractionStyle},
    node::{UIBox, UILayout, UINode},
    sets::UiSet,
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
struct NotesEditor;
impl AssetEditor for NotesEditor {}
#[derive(serde::Serialize, serde::Deserialize)]
struct Pinned;
impl Asset for Pinned {
    fn name() -> &'static str {
        "Pinned"
    }
}
struct PinnedEditor;
impl AssetEditor for PinnedEditor {
    fn closable(&self) -> bool {
        false
    }
}

fn setup() -> (App, Entity, Entity) {
    setup_with(|_| {})
}
fn setup_with(extra: impl FnOnce(&mut App)) -> (App, Entity, Entity) {
    let mut app = App::new();
    let mut registry = AssetEditorRegistry::default();
    registry.register::<Scene>(SceneEditor).unwrap();
    registry.register::<Notes>(NotesEditor).unwrap();
    registry.register::<Pinned>(PinnedEditor).unwrap();
    app.insert_resource(registry);
    app.insert_resource(ActiveEditor::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UITheme::default());
    app.insert_resource(HoveredNode::default());
    app.register_event::<WindowEvent>();
    app.register_plugin(TabsPlugin);
    extra(&mut app);
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
    (app, strip, content)
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
    })
}
fn tick(app: &mut App) {
    app.main_mut().world_mut().run_schedule(LateUpdate);
}
fn tabs(world: &mut World) -> Vec<(Entity, Entity)> {
    world
        .query::<(Entity, &EditorTab), ()>()
        .iter(world)
        .map(|(entity, tab)| (entity, tab.document))
        .collect()
}

#[test]
fn a_dirty_document_shows_a_dot_until_it_is_saved() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    let doc = document(world, 10);
    tick(&mut app);
    let labels = |world: &mut World| -> Vec<String> {
        world
            .query::<&concerto_ui::text::UIText, ()>()
            .iter(world)
            .map(|text| text.text.clone())
            .collect()
    };
    let world = app.main_mut().world_mut();
    assert!(labels(world).contains(&"Document 10".to_string()));
    world.insert(Unsaved, doc);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert!(labels(world).contains(&"• Document 10".to_string()));
    world.remove_component::<Unsaved>(doc);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert!(labels(world).contains(&"Document 10".to_string()));
}

#[test]
fn tabs_build_in_order_inside_scroll_content_and_route_clicks() {
    let (mut app, _, content) = setup();
    let world = app.main_mut().world_mut();
    let later = document(world, 20);
    let earlier = document(world, 10);
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(earlier);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    let children: Vec<_> = world
        .get_component_for_entity::<Children>(content)
        .unwrap()
        .iter()
        .copied()
        .collect();
    assert_eq!(children.len(), 2);
    assert_eq!(
        world
            .get_component_for_entity::<EditorTab>(children[0])
            .unwrap()
            .document,
        earlier
    );
    for child in &children {
        assert_eq!(
            world
                .get_component_for_entity::<ChildOf>(*child)
                .unwrap()
                .parent(),
            content
        );
    }
    let close = world
        .query::<(Entity, &EditorTabClose), ()>()
        .iter(world)
        .find(|(_, close)| close.document == earlier)
        .unwrap()
        .0;
    for target in [children[1], close] {
        world.trigger_on(
            target,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );
    }
    let commands = app.get_resource_mut::<AssetEditorCommands>().unwrap();
    assert!(
        matches!(commands.0.pop_front(), Some(AssetEditorCommand::Activate(entity)) if entity == later)
    );
    assert!(
        matches!(commands.0.pop_front(), Some(AssetEditorCommand::Close(entity)) if entity == earlier)
    );
    assert!(commands.0.is_empty());
}

#[test]
fn activation_updates_style_and_closed_documents_remove_buttons() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    let a = document(world, 1);
    let b = document(world, 2);
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(a);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    let buttons = tabs(world);
    let button_b = buttons.iter().find(|(_, doc)| *doc == b).unwrap().0;
    world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(b);
    world.despawn(a);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert_eq!(tabs(world), vec![(button_b, b)]);
    assert_eq!(
        world
            .get_component_for_entity::<UIInteractionStyle>(button_b)
            .unwrap()
            .normal,
        world.get_resource::<UITheme>().unwrap().surface_raised
    );
}

#[test]
fn overflow_reveals_active_tab_and_wheel_clamps_to_remaining_extent() {
    let (mut app, strip, content) = setup();
    let world = app.main_mut().world_mut();
    let a = document(world, 1);
    let b = document(world, 2);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    for (button, doc) in tabs(world) {
        world.insert(layout(if doc == a { 0.0 } else { 150.0 }, 150.0), button);
    }
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

#[derive(Resource, Default)]
struct Heard(Vec<(&'static str, Entity)>);
fn heard_play(on: On<Play>, mut heard: ResMut<Heard>) {
    heard.0.push(("play", on.signal().0));
}
fn heard_stop(on: On<StopPlay>, mut heard: ResMut<Heard>) {
    heard.0.push(("stop", on.signal().0));
}
fn play_button(world: &mut World, document: Entity) -> Option<(Entity, String)> {
    world
        .query::<(Entity, &EditorTabPlay, &concerto_ui::text::UIText), ()>()
        .iter(world)
        .find(|(_, play, _)| play.document == document)
        .map(|(entity, _, text)| (entity, text.text.clone()))
}
fn has_close_button(world: &mut World, document: Entity) -> bool {
    world
        .query::<&EditorTabClose, ()>()
        .iter(world)
        .any(|close| close.document == document)
}

#[test]
fn the_scene_tab_has_a_play_button_in_place_of_its_close_button_that_names_the_play_state() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    let scene = document_of(world, Scene::name(), 1);
    world.insert(PlayState::Editing, scene);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert!(!has_close_button(world, scene));
    assert_eq!(
        play_button(world, scene).map(|(_, label)| label),
        Some("Play".to_string())
    );
    for (state, label) in [
        (PlayState::Starting, "Starting…"),
        (PlayState::Playing, "Stop"),
        (PlayState::Editing, "Play"),
    ] {
        app.main_mut().world_mut().insert(state, scene);
        tick(&mut app);
        assert_eq!(
            play_button(app.main_mut().world_mut(), scene).map(|(_, label)| label),
            Some(label.to_string())
        );
    }
}

#[test]
fn pressing_the_play_button_plays_while_editing_and_stops_otherwise() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    world.insert_resource(Heard::default());
    world.add_listener(heard_play);
    world.add_listener(heard_stop);
    let scene = document_of(world, Scene::name(), 1);
    world.insert(PlayState::Editing, scene);
    tick(&mut app);
    for (state, heard) in [
        (PlayState::Editing, "play"),
        (PlayState::Starting, "stop"),
        (PlayState::Playing, "stop"),
    ] {
        let world = app.main_mut().world_mut();
        world.insert(state, scene);
        let (button, _) = play_button(world, scene).unwrap();
        world.trigger_on(
            button,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Right,
            },
        );
        world.trigger_on(
            button,
            UIClick {
                position: Vec2::ZERO,
                button: MouseButton::Left,
            },
        );
        assert_eq!(
            std::mem::take(&mut world.get_resource_mut::<Heard>().unwrap().0),
            vec![(heard, scene)]
        );
    }
    assert!(
        app.get_resource::<AssetEditorCommands>()
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn a_custom_asset_editors_tab_keeps_its_close_button_and_has_no_play_button() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    let notes = document_of(world, Notes::name(), 1);
    let unknown = document(world, 2);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    for document in [notes, unknown] {
        assert!(has_close_button(world, document));
        assert!(play_button(world, document).is_none());
    }
}

#[test]
fn a_tab_that_cannot_be_closed_has_a_play_button_only_for_a_scene() {
    let (mut app, _, _) = setup();
    let world = app.main_mut().world_mut();
    let pinned = document_of(world, Pinned::name(), 1);
    world.insert(PlayState::Editing, pinned);
    tick(&mut app);
    let world = app.main_mut().world_mut();
    assert!(play_button(world, pinned).is_none());
    assert!(!has_close_button(world, pinned));
}

#[derive(Resource, Default)]
struct Pressed(bool);
fn start_the_game(pressed: Res<Pressed>, states: Query<&mut PlayState>) {
    if !pressed.0 {
        return;
    }
    for mut state in states.iter() {
        **state = PlayState::Starting;
    }
}

#[test]
fn the_button_reads_starting_in_the_frame_a_press_starts_the_game() {
    let (mut app, _, _) = setup_with(|app| {
        app.insert_resource(Pressed::default());
        app.configure_sets(LateUpdate, (UiSet::Input, UiSet::Materials).chain())
            .add_system(LateUpdate, start_the_game.in_set(UiSet::Input));
    });
    let world = app.main_mut().world_mut();
    let scene = document_of(world, Scene::name(), 1);
    world.insert(PlayState::Editing, scene);
    tick(&mut app);
    assert_eq!(
        play_button(app.main_mut().world_mut(), scene).map(|(_, label)| label),
        Some("Play".to_string())
    );
    app.get_resource_mut::<Pressed>().unwrap().0 = true;
    tick(&mut app);
    assert_eq!(
        play_button(app.main_mut().world_mut(), scene).map(|(_, label)| label),
        Some("Starting…".to_string())
    );
}
