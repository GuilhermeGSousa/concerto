//! Exercise the real tab systems without a window or GPU.
use concerto_app::{App, schedule_groups::LateUpdate};
use concerto_ecs::{
    Entity, World,
    entity::hierarchy::{ChildOf, Children},
    events::event_channel::EventChannel,
};
use concerto_editor::{
    asset_editor::{ActiveEditor, AssetEditorCommand, AssetEditorCommands, EditorDocument},
    tabs::{EditorTab, EditorTabClose, TabScroll, TabStrip, TabStripContent, TabsPlugin},
};
use concerto_ui::{
    interaction::{HoveredNode, UIClick, UIInteractionStyle},
    node::{UIBox, UILayout, UINode},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;
use concerto_window::winit_events::WindowEvent;
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
fn setup() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(ActiveEditor::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UITheme::default());
    app.insert_resource(HoveredNode::default());
    app.register_event::<UIClick>();
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
    (app, strip, content)
}
fn document(world: &mut World, order: u64) -> Entity {
    world.spawn(EditorDocument {
        asset_type: "Document",
        title: format!("Document {order}"),
        current: None,
        pending: None,
        project_generation: 1,
        request_generation: 1,
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
    let clicks = world.get_resource_mut::<EventChannel<UIClick>>().unwrap();
    clicks.push_event(UIClick {
        entity: children[1],
        position: Vec2::ZERO,
        button: MouseButton::Left,
    });
    clicks.push_event(UIClick {
        entity: close,
        position: Vec2::ZERO,
        button: MouseButton::Left,
    });
    tick(&mut app);
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
        .push_event(WindowEvent::new(winit::event::WindowEvent::MouseWheel {
            device_id: winit::event::DeviceId::dummy(),
            delta: winit::event::MouseScrollDelta::LineDelta(0.0, 100.0),
            phase: winit::event::TouchPhase::Moved,
        }));
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
