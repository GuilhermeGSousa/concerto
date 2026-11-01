//! The viewport panel: shows the active document's picture and tells the host where it is.
use concerto_app::{
    App, Plugin,
    schedule_groups::{First, LateUpdate, Startup, Update},
};
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, command::CommandQueue,
    events::event_reader::EventReader,
};
use concerto_ui::{
    UISharedViewport,
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{HoveredNode, Interactable},
    material::UIMaterial,
    node::{UIInset, UILayout, UINode, UIRect},
    sets::UiSet,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::actions::{ActionFired, ActionMap};
use taffy::Position;

use crate::actions::{FrameAll, FrameSelected, GizmoRotate, GizmoTranslate, ViewportContext};
use crate::asset_editor::ActiveEditor;
use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::hosted::{
    EditWorldSignals, HostedApp,
    input::{InputOwner, ViewportShown, ViewportView},
};
use crate::tools::{
    camera::{FrameSelection, SKY},
    gizmo::{GizmoMode, SetGizmoMode},
};

#[derive(Component)]
pub struct ViewportRegion;

#[derive(Component)]
struct InputHint;

pub struct ViewportPlugin;
impl Plugin for ViewportPlugin {
    fn build(&self, app: &mut App) {
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Scene",
            region: Region::Scene,
        });
        app.add_system(Startup, build_panel)
            .add_system(Update, viewport_shortcuts)
            .add_system(LateUpdate, sync_viewport_context)
            .add_system(LateUpdate, show_active_picture.before(UiSet::Materials))
            .add_system(LateUpdate, show_input_owner.before(UiSet::Materials))
            .add_system(
                First,
                publish_viewport_view.before(crate::hosted::input::route_play_input),
            );
    }
}

pub const PANEL_ID: &str = "concerto.scene";

const INPUT_BORDER: f32 = 2.0;

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        let viewport = spawn_panel(&mut cmd, body);
        cmd.entity(viewport).add_child((
            theme
                .card()
                .padding(UIRect::axes(theme.spacing_xs, theme.spacing_sm))
                .node(|node| {
                    node.with_position(Position::Absolute)
                        .with_inset(UIInset {
                            left: UIValue::Px(theme.spacing_sm),
                            top: UIValue::Px(theme.spacing_sm),
                            ..Default::default()
                        })
                        .with_visible(false)
                }),
            theme.text("").small(),
            InputHint,
        ));
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity) -> Entity {
    cmd.entity(parent)
        .spawn_child((
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_size(UIValue::Px(320.0), UIValue::Px(240.0)),
            UIMaterial::flat(SKY),
            UISharedViewport::new(),
            Interactable,
            ViewportRegion,
        ))
        .entity()
}

fn show_active_picture(
    active: Res<ActiveEditor>,
    hosted: Query<&HostedApp>,
    viewports: Query<(&ViewportRegion, &mut UISharedViewport, &mut UIMaterial)>,
) {
    let picture = active
        .0
        .and_then(|document| hosted.get_entity(document))
        .map(|hosted| hosted.picture().clone());
    let drawn = picture
        .as_ref()
        .is_some_and(|picture| picture.get().is_some());
    let colour = if drawn { Color::WHITE } else { SKY }.to_linear();
    for (_, mut viewport, mut material) in viewports.iter() {
        let unchanged = match (&viewport.picture, &picture) {
            (Some(shown), Some(wanted)) => shown.is_same(wanted),
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            viewport.picture = picture.clone();
        }
        if material.color != colour {
            material.color = colour;
        }
    }
}

fn publish_viewport_view(
    window: Res<concerto_window::plugin::Window>,
    hovered: Res<HoveredNode>,
    focus: Res<FocusedWidget>,
    regions: Query<(Entity, &ViewportRegion, &UILayout)>,
    mut shown: ResMut<ViewportShown>,
) {
    let scale = window.scale_factor() as f32;
    let next = regions.iter().next().and_then(|(region, _, layout)| {
        let size = (
            (layout.rect.size.x * scale).round() as u32,
            (layout.rect.size.y * scale).round() as u32,
        );
        (size.0 > 0 && size.1 > 0).then_some(ViewportView {
            origin: layout.rect.min * scale,
            size,
            over: **hovered == Some(region),
            typing: focus.is_some(),
        })
    });
    if shown.0 != next {
        shown.0 = next;
    }
}

fn show_input_owner(
    active: Res<ActiveEditor>,
    theme: Res<UITheme>,
    owners: Query<&InputOwner>,
    viewports: Query<(&ViewportRegion, &mut UIMaterial)>,
    hints: Query<(&InputHint, &mut UINode, &mut UIText)>,
) {
    let owner = active
        .0
        .and_then(|document| owners.get_entity(document))
        .copied();
    let (border, hint) = match owner {
        Some(InputOwner::Game) => (Some(theme.accent), "Shift+F1 to release"),
        Some(InputOwner::Editor) => (Some(theme.text_muted), "Click to play"),
        None => (None, ""),
    };
    let width = if border.is_some() { INPUT_BORDER } else { 0.0 };
    let colour = border.unwrap_or(Color::TRANSPARENT).to_linear();
    for (_, mut material) in viewports.iter() {
        if material.border_width != width {
            material.border_width = width;
        }
        if material.border_color != colour {
            material.border_color = colour;
        }
    }
    for (_, mut node, mut text) in hints.iter() {
        if node.visible != owner.is_some() {
            node.visible = owner.is_some();
        }
        if text.text != hint {
            text.text = hint.into();
        }
    }
}

fn sync_viewport_context(
    hovered: Res<HoveredNode>,
    regions: Query<&ViewportRegion>,
    mut actions: ResMut<ActionMap>,
) {
    let over = (**hovered).is_some_and(|entity| regions.get_entity(entity).is_some());
    if over {
        actions.push_context(ViewportContext);
    } else {
        actions.pop_context(ViewportContext);
    }
}

fn viewport_shortcuts(mut fired: EventReader<ActionFired>, mut scene: EditWorldSignals) {
    for action in fired.read() {
        if action.is(GizmoTranslate) {
            scene.trigger(SetGizmoMode(GizmoMode::Translate));
        } else if action.is(GizmoRotate) {
            scene.trigger(SetGizmoMode(GizmoMode::Rotate));
        } else if action.is(FrameSelected) {
            scene.trigger(FrameSelection);
        } else if action.is(FrameAll) {
            scene.trigger(crate::tools::camera::FrameAll);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::testing::{host_world, hosted_now, plain_recipe};
    use concerto_app::edit_time::AtEditTime;
    use concerto_ecs::{
        IntoSystem, Resource, System, World, events::event_channel::EventChannel, signal::On,
    };
    use concerto_ui::node::UIBox;
    use concerto_window::{input::actions::ActionLabel, plugin::Window};
    use glam::Vec2;

    #[derive(Resource, Default)]
    struct Sent(Vec<&'static str>);

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn panel_world(scale: f64) -> (World, Entity) {
        let mut world = World::new();
        world.insert_resource(Window::new((1600, 1200), scale));
        world.insert_resource(HoveredNode::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(ViewportShown::default());
        let rect = UIBox {
            min: Vec2::new(100.0, 50.0),
            size: Vec2::new(400.0, 300.0),
        };
        let region = world.spawn((
            ViewportRegion,
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            },
        ));
        (world, region)
    }

    fn view(world: &mut World) -> Option<ViewportView> {
        run(world, publish_viewport_view);
        world.get_resource::<ViewportShown>().unwrap().0
    }

    #[derive(Resource)]
    struct PanelRect(UIBox);

    #[derive(Resource, Default)]
    struct SeenByTheStep(Option<ViewportView>);

    fn lay_out_by_command(
        rect: Res<PanelRect>,
        regions: Query<(Entity, &ViewportRegion)>,
        mut cmd: CommandQueue,
    ) {
        for (region, _) in regions.iter() {
            cmd.insert(
                UILayout {
                    rect: rect.0,
                    content_rect: rect.0,
                    clip_rect: rect.0,
                    paint_order: 0,
                },
                region,
            );
        }
    }

    fn note_what_the_step_saw(shown: Res<ViewportShown>, mut seen: ResMut<SeenByTheStep>) {
        seen.0 = shown.0;
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> UIBox {
        UIBox {
            min: Vec2::new(x, y),
            size: Vec2::new(width, height),
        }
    }

    #[test]
    fn the_step_sees_the_viewport_where_the_layout_of_the_frame_before_put_it() {
        let mut app = App::new();
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .register_plugin(concerto_app::plugins::TimePlugin)
            .register_plugin(concerto_window::plugin::WindowPlugin)
            .insert_resource(ActiveEditor::default())
            .insert_resource(PanelRegistry::default())
            .insert_resource(HoveredNode::default())
            .insert_resource(FocusedWidget::default())
            .insert_resource(UITheme::default())
            .register_plugin(crate::hosted::HostedPlugin)
            .register_plugin(ViewportPlugin)
            .insert_resource(PanelRect(rect(100.0, 50.0, 400.0, 300.0)))
            .insert_resource(SeenByTheStep::default())
            .add_system(LateUpdate, lay_out_by_command.in_set(UiSet::Layout))
            .add_system(
                concerto_app::schedule_groups::First,
                note_what_the_step_saw.after(crate::hosted::input::step_viewport_target),
            );
        let world = app.main_mut().world_mut();
        let document = world.spawn(hosted_now(plain_recipe, |_| {}));
        world.spawn(ViewportRegion);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        world.trigger(crate::hosted::SetEditTarget(Some(document)));
        app.finish_plugin_build();
        for _ in 0..3 {
            app.update();
        }

        app.get_resource_mut::<PanelRect>().unwrap().0 = rect(120.0, 60.0, 500.0, 350.0);
        app.update();
        app.update();

        assert_eq!(
            app.get_resource::<SeenByTheStep>().unwrap().0,
            Some(ViewportView {
                origin: Vec2::new(120.0, 60.0),
                size: (500, 350),
                over: false,
                typing: false,
            })
        );
        let hosted = app
            .main()
            .world()
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert_eq!(hosted.window().unwrap().size(), (500, 350));
    }

    #[test]
    fn the_view_is_the_panel_in_physical_pixels_and_is_over_it_only_when_it_is_the_hovered_node() {
        let (mut world, region) = panel_world(2.0);
        let menu = world.spawn(());

        let away = ViewportView {
            origin: Vec2::new(200.0, 100.0),
            size: (800, 600),
            over: false,
            typing: false,
        };
        assert_eq!(view(&mut world), Some(away));

        **world.get_resource_mut::<HoveredNode>().unwrap() = Some(region);
        assert_eq!(view(&mut world), Some(ViewportView { over: true, ..away }));

        **world.get_resource_mut::<HoveredNode>().unwrap() = Some(menu);
        assert_eq!(
            view(&mut world),
            Some(away),
            "a node drawn above the viewport takes the pointer from it"
        );
    }

    #[test]
    fn the_view_says_when_a_widget_has_the_keyboard() {
        let (mut world, _) = panel_world(1.0);
        let field = world.spawn(());
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(field);

        assert!(view(&mut world).unwrap().typing);
    }

    #[test]
    fn a_panel_with_no_area_is_not_shown_and_an_unchanged_view_is_not_written_again() {
        let (mut world, region) = panel_world(1.0);
        view(&mut world);
        let written = world.resource_changed_tick::<ViewportShown>();
        world.tick();
        view(&mut world);
        assert_eq!(world.resource_changed_tick::<ViewportShown>(), written);

        world
            .get_component_for_entity_mut::<UILayout>(region)
            .unwrap()
            .rect
            .size = Vec2::new(400.0, 0.0);
        assert_eq!(view(&mut world), None);

        world.despawn(region);
        assert_eq!(view(&mut world), None);
    }

    #[test]
    fn the_panel_shows_the_picture_of_the_active_documents_app_or_nothing() {
        let (mut world, document) = host_world(plain_recipe);
        let other = world.spawn(hosted_now(plain_recipe, |_| {}));
        let panel = world.spawn((
            ViewportRegion,
            UISharedViewport::new(),
            UIMaterial::flat(Color::WHITE),
        ));
        let shows = |world: &mut World, document: Option<Entity>| {
            world.get_resource_mut::<ActiveEditor>().unwrap().0 = document;
            run(world, show_active_picture);
            let shown = world
                .get_component_for_entity::<UISharedViewport>(panel)
                .unwrap()
                .picture
                .clone();
            match (shown, document) {
                (Some(shown), Some(document)) => shown.is_same(
                    world
                        .get_component_for_entity::<HostedApp>(document)
                        .unwrap()
                        .picture(),
                ),
                (None, None) => true,
                _ => false,
            }
        };

        assert!(shows(&mut world, Some(document)));
        assert!(shows(&mut world, Some(other)));
        assert!(shows(&mut world, None));

        let bare = world.spawn(());
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(bare);
        run(&mut world, show_active_picture);
        assert!(
            world
                .get_component_for_entity::<UISharedViewport>(panel)
                .unwrap()
                .picture
                .is_none()
        );
    }

    #[test]
    fn a_panel_with_nothing_drawn_to_show_is_the_colour_of_an_empty_scene() {
        let (mut world, document) = host_world(plain_recipe);
        let mut build = (|mut cmd: CommandQueue| {
            let parent = cmd.spawn(UINode::default()).entity();
            spawn_panel(&mut cmd, parent);
        })
        .into_system();
        build.initialize(&mut world);
        build.run_and_apply((), &mut world);
        let colour = |world: &mut World| {
            run(world, show_active_picture);
            world
                .query::<(&ViewportRegion, &UIMaterial), ()>()
                .iter(world)
                .next()
                .unwrap()
                .1
                .color
        };
        let empty = SKY.to_linear();
        assert!(colour(&mut world) == empty, "its App has drawn nothing yet");

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        assert!(colour(&mut world) == empty);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        world
            .query::<(&ViewportRegion, &mut UIMaterial), ()>()
            .iter(&mut world)
            .next()
            .unwrap()
            .1
            .color = Color::WHITE.to_linear();
        assert!(colour(&mut world) == empty);
    }

    fn noting_recipe(app: &mut App) {
        plain_recipe(app);
        app.insert_resource(Sent::default())
            .add_listener(
                (|on: On<SetGizmoMode>, mut sent: ResMut<Sent>| {
                    sent.0.push(match on.signal().0 {
                        GizmoMode::Translate => "translate",
                        GizmoMode::Rotate => "rotate",
                    });
                })
                .at_edit_time(),
            )
            .add_listener(
                (|_: On<FrameSelection>, mut sent: ResMut<Sent>| {
                    sent.0.push("frame selection");
                })
                .at_edit_time(),
            )
            .add_listener(
                (|_: On<crate::tools::camera::FrameAll>, mut sent: ResMut<Sent>| {
                    sent.0.push("frame all");
                })
                .at_edit_time(),
            );
    }

    #[test]
    fn the_viewport_shows_who_has_input_while_playing() {
        let mut world = World::new();
        world.insert_resource(UITheme::default());
        let document = world.spawn(());
        world.insert_resource(ActiveEditor(Some(document)));
        let region = world.spawn((ViewportRegion, UIMaterial::flat(Color::WHITE)));
        let hint = world.spawn((
            UINode::default().with_visible(false),
            UIText::default(),
            InputHint,
        ));
        let shown = |world: &mut World| {
            run(world, show_input_owner);
            let border = world
                .get_component_for_entity::<UIMaterial>(region)
                .unwrap()
                .border_width;
            let visible = world
                .get_component_for_entity::<UINode>(hint)
                .unwrap()
                .visible;
            let text = world
                .get_component_for_entity::<UIText>(hint)
                .unwrap()
                .text
                .clone();
            (border > 0.0, visible, text)
        };

        assert_eq!(shown(&mut world), (false, false, String::new()));

        world.insert(InputOwner::Game, document);
        assert_eq!(
            shown(&mut world),
            (true, true, "Shift+F1 to release".to_owned())
        );

        world.insert(InputOwner::Editor, document);
        assert_eq!(shown(&mut world), (true, true, "Click to play".to_owned()));

        world.remove_component::<InputOwner>(document);
        let (border, visible, _) = shown(&mut world);
        assert!(!border && !visible);
    }

    #[test]
    fn each_viewport_shortcut_triggers_its_signal_in_the_edit_target() {
        let (mut world, document) = host_world(noting_recipe);
        world.insert_resource(EventChannel::<ActionFired>::default());
        let channel = world
            .get_resource_mut::<EventChannel<ActionFired>>()
            .unwrap();
        for action in [
            GizmoRotate.intern(),
            GizmoTranslate.intern(),
            FrameSelected.intern(),
            FrameAll.intern(),
            crate::actions::Save.intern(),
        ] {
            channel.push_event(ActionFired { action });
        }

        run(&mut world, viewport_shortcuts);

        let hosted = world
            .get_component_for_entity::<HostedApp>(document)
            .unwrap();
        assert_eq!(
            hosted.world_for_test().get_resource::<Sent>().unwrap().0,
            ["rotate", "translate", "frame selection", "frame all"]
        );
    }
}
