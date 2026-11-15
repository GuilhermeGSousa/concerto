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

use crate::actions::{
    EditorContextSet, FrameAll, FrameSelected, GizmoRotate, GizmoTranslate, ViewportContext,
};
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
            region: Region::Centre,
        });
        app.add_system(Startup, build_panel)
            .add_system(Update, viewport_shortcuts)
            .add_system(LateUpdate, sync_viewport_context.in_set(EditorContextSet))
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
        let viewport = spawn_panel(&mut cmd, body, &theme);
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

/// Spawns the viewport under `parent`: the picture of the front tab's `App`, cut to a card's corners.
pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) -> Entity {
    cmd.entity(parent)
        .spawn_child((
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_size(UIValue::Px(320.0), UIValue::Px(240.0)),
            UIMaterial {
                corner_radius: theme.radius_lg,
                ..UIMaterial::flat(SKY)
            },
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

pub(crate) fn publish_viewport_view(
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

    #[test]
    fn the_view_is_the_panel_in_physical_pixels_over_it_only_when_hovered_and_says_who_types() {
        let mut world = World::new();
        world.insert_resource(Window::new((1600, 1200), 2.0));
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
        let menu = world.spawn(());
        let view = |world: &mut World| {
            run(world, publish_viewport_view);
            world.get_resource::<ViewportShown>().unwrap().0
        };

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
        assert_eq!(view(&mut world), Some(away));

        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(menu);
        assert_eq!(
            view(&mut world),
            Some(ViewportView {
                typing: true,
                ..away
            })
        );
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
            channel.push_event(ActionFired::new(action));
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
