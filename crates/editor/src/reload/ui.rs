use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::CommandQueue,
    entity::hierarchy::Children,
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, filter::With},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_ui::{
    elements::prelude::*,
    interaction::{Interactable, UIClick},
    node::{UIInset, UILayout, UINode, UIRect},
    scroll::UIScrollArea,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::{
    MouseButton,
    actions::{ActionFired, ActionMap},
};
use taffy::{FlexDirection, Position};

use super::{CancelReload, GameReload, ReloadGameCode, ReloadPhase};
use crate::{
    actions::{BuildErrorsContext, DismissBuildErrors},
    dock::{DockedApp, MARGIN, PanelDescriptor, PanelRegistry, RAIL, Region},
    fonts::{glyph, icon},
    guard::PROMPT_LAYER,
    window_chrome::{CONTROL_LAYER, WindowChromeControl},
};

const PANEL_ID: &str = "concerto.reload";
const ABOVE_CHATTER: f32 = 30.0 + 8.0;
const ERROR_VIEW_MAX_HEIGHT: f32 = 320.0;

pub(super) struct ReloadUiPlugin;

impl Plugin for ReloadUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Reload",
            region: Region::Stats,
        });
        app.add_system(Startup, build_reload_button)
            .add_system(Startup, build_error_list)
            .add_system(Update, dismiss_build_errors)
            .add_system(LateUpdate, sync_reload_button)
            .add_system(LateUpdate, sync_error_list)
            .add_system(LateUpdate, sync_error_scroll);
    }
}

#[derive(Component)]
enum ButtonPart {
    Glyph,
    Label,
}

#[derive(Component)]
pub(super) struct ErrorList;

#[derive(Component)]
pub(super) struct ErrorView;

#[derive(Component)]
pub(super) struct ErrorStack;

#[derive(Component)]
pub(super) enum ErrorText {
    Header,
    Block,
}

enum Press {
    Reload,
    Cancel,
    Ignore,
}

struct Face {
    label: &'static str,
    glyph: char,
    accent: bool,
    press: Press,
}

fn face(reload: &GameReload) -> Face {
    match reload.phase {
        ReloadPhase::Idle => Face {
            label: "Reload",
            glyph: glyph::ARROW_CLOCKWISE,
            accent: false,
            press: Press::Reload,
        },
        ReloadPhase::Building(_) | ReloadPhase::Waiting { .. } | ReloadPhase::Confirming(_) => {
            Face {
                label: "Cancel",
                glyph: glyph::ARROW_CLOCKWISE,
                accent: true,
                press: Press::Cancel,
            }
        }
        ReloadPhase::Closing => Face {
            label: "Restarting…",
            glyph: glyph::ARROW_CLOCKWISE,
            accent: true,
            press: Press::Ignore,
        },
        ReloadPhase::Failed => Face {
            label: "Reload",
            glyph: glyph::WARNING,
            accent: true,
            press: Press::Reload,
        },
    }
}

fn build_reload_button(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    let Some(body) = registry.body(PANEL_ID) else {
        return;
    };
    cmd.entity(body).add_child_with(
        (
            theme
                .pressable()
                .row()
                .gap(theme.spacing_xs + 2.0)
                .height(UIValue::Px(26.0))
                .padding(UIRect::axes(0.0, theme.spacing_sm))
                .margin(UIRect {
                    right: theme.spacing_md,
                    ..Default::default()
                })
                .z_index(CONTROL_LAYER)
                .on_click(press_reload_button()),
            WindowChromeControl,
        ),
        |mut button| {
            button
                .add_child((
                    UINode::default().with_flex_shrink(0.0),
                    icon(&theme, glyph::ARROW_CLOCKWISE, theme.font_size_md).muted(),
                    ButtonPart::Glyph,
                ))
                .add_child((theme.label("Reload").small().no_wrap(), ButtonPart::Label));
        },
    );
}

fn press_reload_button() -> Listener<UIClick> {
    (move |on: On<UIClick>, reload: Res<GameReload>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        match face(&reload).press {
            Press::Reload => cmd.trigger(ReloadGameCode),
            Press::Cancel => cmd.trigger(CancelReload),
            Press::Ignore => {}
        }
    })
    .into_listener()
}

/// Opens or closes the list of the last failed build's errors on a left click; it opens only after a failure.
pub(crate) fn set_build_errors_shown(shown: bool) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut reload: ResMut<GameReload>| {
        if on.signal().button == MouseButton::Left && reload.errors_shown() != shown {
            reload.set_errors_shown(shown);
        }
    })
    .into_listener()
}

fn sync_reload_button(
    reload: Res<GameReload>,
    theme: Res<UITheme>,
    parts: Query<(&ButtonPart, &mut UIText)>,
) {
    let face = face(&reload);
    let glyph_color = if face.accent {
        theme.accent
    } else {
        theme.text_muted
    };
    for (part, mut text) in parts.iter() {
        let value = match part {
            ButtonPart::Glyph => {
                if text.color != glyph_color {
                    text.color = glyph_color;
                }
                face.glyph.to_string()
            }
            ButtonPart::Label => face.label.to_string(),
        };
        if text.text != value {
            text.text = value;
        }
    }
}

pub(super) fn build_error_list(mut cmd: CommandQueue, theme: Res<UITheme>) {
    cmd.spawn((
        UINode::default()
            .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
            .with_z_index(PROMPT_LAYER)
            .with_visible(false),
        ErrorList,
    ))
    .add_child_with(
        (
            theme
                .popup()
                .width(UIValue::Px(720.0))
                .max_width(UIValue::Percent(60.0))
                .max_height(UIValue::Percent(50.0))
                .padding(theme.spacing_md)
                .gap(theme.spacing_sm)
                .node(|node| {
                    node.with_position(Position::Absolute).with_inset(UIInset {
                        left: UIValue::Px(RAIL + MARGIN * 2.0),
                        bottom: UIValue::Px(MARGIN + ABOVE_CHATTER),
                        ..Default::default()
                    })
                }),
            Interactable,
        ),
        |mut card| {
            card.add_child_with(theme.row().fixed(), |mut header| {
                header
                    .add_child((theme.label("").single_line().grow(), ErrorText::Header))
                    .add_child((
                        theme
                            .pressable()
                            .size(UIValue::Px(24.0), UIValue::Px(24.0))
                            .padding(UIRect::axes(4.0, 5.0))
                            .on_click(set_build_errors_shown(false)),
                        icon(&theme, glyph::X, theme.font_size_md).muted(),
                    ));
            })
            .add_child_with(
                (
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_shrink(1.0)
                        .with_min_height(UIValue::Px(0.0))
                        .with_flex_direction(FlexDirection::Column)
                        .clipped(),
                    Interactable,
                    ErrorView,
                ),
                |mut view| {
                    let stack = view
                        .spawn_child((theme.column().fixed().gap(theme.spacing_md), ErrorStack))
                        .entity();
                    view.insert(UIScrollArea {
                        content: Some(stack),
                        ..Default::default()
                    });
                },
            );
        },
    );
}

fn dismiss_build_errors(mut fired: EventReader<ActionFired>, mut reload: ResMut<GameReload>) {
    let dismiss = fired.read().fold(false, |dismiss, action| {
        dismiss | action.is(DismissBuildErrors)
    });
    if dismiss && reload.errors_shown() {
        reload.set_errors_shown(false);
    }
}

fn header(count: usize) -> String {
    format!(
        "Build failed · {count} {}",
        if count == 1 { "error" } else { "errors" }
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sync_error_list(
    reload: Res<GameReload>,
    theme: Res<UITheme>,
    lists: Query<(&ErrorList, &mut UINode)>,
    stacks: Query<(Entity, Option<&Children>), With<ErrorStack>>,
    texts: Query<(&ErrorText, &mut UIText)>,
    views: Query<&mut UIScrollArea, With<ErrorView>>,
    mut actions: ResMut<ActionMap>,
    mut cmd: CommandQueue,
) {
    let shown = reload.errors_shown();
    for (_, mut node) in lists.iter() {
        if node.visible != shown {
            node.visible = shown;
        }
    }
    if actions.is_active(BuildErrorsContext) != shown {
        if shown {
            actions.push_context(BuildErrorsContext);
        } else {
            actions.pop_context(BuildErrorsContext);
        }
    }
    if !reload.is_changed() {
        return;
    }
    let title = header(reload.errors().len());
    for (part, mut text) in texts.iter() {
        if matches!(part, ErrorText::Header) && text.text != title {
            text.text = title.clone();
        }
    }
    for (stack, children) in stacks.iter() {
        let blocks: Vec<Entity> = children
            .into_iter()
            .flat_map(|children| children.iter().copied())
            .collect();
        let listed = blocks.iter().map(|block| {
            texts
                .get_entity(*block)
                .map(|(_, text)| text.text.clone())
                .unwrap_or_default()
        });
        if listed.eq(reload.errors().iter().map(block_text)) {
            continue;
        }
        for block in blocks {
            cmd.despawn(block);
        }
        for error in reload.errors() {
            cmd.entity(stack).add_child((
                theme.label(block_text(error)).small().mono().no_wrap(),
                ErrorText::Block,
            ));
        }
        for mut area in views.iter() {
            if area.offset != 0.0 {
                area.offset = 0.0;
            }
        }
    }
}

fn block_text(error: &super::BuildDiagnostic) -> String {
    error.rendered.trim_end().to_string()
}

fn sync_error_scroll(
    stacks: Query<&UILayout, With<ErrorStack>>,
    views: Query<(&mut UIScrollArea, &mut UINode, &UILayout), With<ErrorView>>,
) {
    let Some(stack) = stacks.iter().next() else {
        return;
    };
    let extent = stack.rect.size.y;
    let height = UIValue::Px(extent.min(ERROR_VIEW_MAX_HEIGHT));
    for (mut area, mut node, view) in views.iter() {
        if node.height != height {
            node.height = height;
        }
        if area.content_extent != extent {
            area.content_extent = extent;
        }
        let offset = area
            .offset
            .clamp(0.0, (extent - view.content_rect.size.y).max(0.0));
        if area.offset != offset {
            area.offset = offset;
        }
    }
}

#[cfg(test)]
pub(super) fn listed_errors(world: &mut concerto_ecs::World) -> Option<Vec<String>> {
    let shown = world
        .query::<(&ErrorList, &UINode), ()>()
        .iter(world)
        .any(|(_, node)| node.visible);
    if !shown {
        return None;
    }
    let stack = world
        .query::<(Entity, &ErrorStack), ()>()
        .iter(world)
        .map(|(entity, _)| entity)
        .next()?;
    let blocks: Vec<Entity> = world
        .get_component_for_entity::<Children>(stack)
        .map(|children| children.iter().copied().collect())
        .unwrap_or_default();
    Some(
        blocks
            .into_iter()
            .filter_map(|block| world.get_component_for_entity::<UIText>(block))
            .map(|text| text.text.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::mpsc};

    use concerto_app::{
        App,
        schedule_groups::{LateUpdate, Update},
    };
    use concerto_color::Color;
    use concerto_ecs::{
        Entity, IntoSystem, ResMut, System, World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
        events::event_writer::EventWriter,
    };
    use concerto_ui::{interaction::UIClick, text::UIText, theme::UITheme};
    use concerto_window::{
        input::{
            KeyCode, MouseButton,
            actions::{ActionFired, ActionLabel, ActionMap, Shortcut},
        },
        plugin::Window,
    };

    use concerto_ui::{
        node::{UIBox, UILayout, UINode},
        transform::UIValue,
    };
    use glam::Vec2;

    use super::{
        ButtonPart, ERROR_VIEW_MAX_HEIGHT, ErrorStack, ErrorView, ReloadUiPlugin, listed_errors,
    };
    use crate::{
        actions::{ActionsPlugin, DismissBuildErrors},
        dock::{DockPlugin, PanelRegistry},
        fonts::glyph,
        project::{Project, ProjectState},
        reload::{
            BuildDiagnostic, GameReload, ReloadFailed, ReloadGameCode, ReloadPhase, ReloadTarget,
            cargo::BuildJob,
        },
        scene::SceneState,
        shell::{CHATTER_ID, ShellPlugin},
        window_chrome::{CONTROL_LAYER, WindowChromeControl},
    };

    fn setup() -> App {
        let mut app = App::new();
        app.insert_resource(PanelRegistry::default());
        app.insert_resource(UITheme::default());
        app.insert_resource(Window::new((1280, 720), 1.0));
        app.insert_resource(ActionMap::default());
        app.insert_resource(ProjectState::default());
        app.insert_resource(SceneState::default());
        app.insert_resource(GameReload::new(Err(
            "Reload is not available under --mcp".into()
        )));
        app.register_event::<ReloadFailed>();
        app.register_event::<ActionFired>();
        app.main_mut().world_mut().register_component::<ChildOf>();
        app.main_mut().world_mut().register_component::<Children>();
        app.register_plugin(DockPlugin);
        app.register_plugin(ActionsPlugin);
        app.register_plugin(ReloadUiPlugin);
        app.register_plugin(ShellPlugin);
        crate::reload::install_listeners(app.main_mut().world_mut());
        app.finish_plugin_build();
        app
    }

    fn tick(app: &mut App) {
        let world = app.main_mut().world_mut();
        world.run_schedule(Update);
        world.run_schedule(LateUpdate);
    }

    fn reload(app: &mut App) -> &mut GameReload {
        app.get_resource_mut::<GameReload>().unwrap()
    }

    fn error(message: &str, line: u32) -> BuildDiagnostic {
        BuildDiagnostic {
            message: message.into(),
            location: Some(("src/lib.rs".into(), line, 5)),
            rendered: format!("error: {message}\n --> src/lib.rs:{line}:5\n"),
        }
    }

    fn listed(errors: &[BuildDiagnostic]) -> Option<Vec<String>> {
        Some(
            errors
                .iter()
                .map(|error| error.rendered.trim_end().to_string())
                .collect(),
        )
    }

    fn fail(app: &mut App, errors: Vec<BuildDiagnostic>) {
        let world = app.main_mut().world_mut();
        let mut system = (move |mut reload: ResMut<GameReload>,
                                mut failed: EventWriter<ReloadFailed>| {
            reload.build_failed(errors.clone(), &mut failed);
        })
        .into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn click(app: &mut App, target: Entity) {
        app.main_mut().world_mut().trigger_on(
            target,
            UIClick {
                position: Default::default(),
                button: MouseButton::Left,
            },
        );
    }

    fn with_text(world: &mut World, value: &str) -> Entity {
        world
            .query::<(Entity, &UIText), ()>()
            .iter(world)
            .find(|(_, text)| text.text == value)
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("nothing reads {value:?}"))
    }

    fn pressable(world: &mut World, label: &str) -> Entity {
        let label = with_text(world, label);
        world
            .get_component_for_entity::<ChildOf>(label)
            .unwrap()
            .parent()
    }

    fn chatter(app: &mut App) -> Entity {
        let world = app.main_mut().world_mut();
        let body = world
            .get_resource::<PanelRegistry>()
            .unwrap()
            .body(CHATTER_ID)
            .unwrap();
        world
            .get_component_for_entity::<Children>(body)
            .unwrap()
            .iter()
            .copied()
            .next()
            .unwrap()
    }

    fn escape(app: &mut App) {
        let world = app.main_mut().world_mut();
        let action = world
            .get_resource::<ActionMap>()
            .unwrap()
            .resolve(Shortcut::key(KeyCode::Escape));
        if let Some(action) = action {
            world
                .get_resource_mut::<EventChannel<ActionFired>>()
                .unwrap()
                .push_event(ActionFired { action });
        }
    }

    fn button(app: &mut App) -> (String, String, Color) {
        let world = app.main_mut().world_mut();
        let parts: Vec<(bool, String, Color)> = world
            .query::<(&ButtonPart, &UIText), ()>()
            .iter(world)
            .map(|(part, text)| {
                (
                    matches!(part, ButtonPart::Label),
                    text.text.clone(),
                    text.color,
                )
            })
            .collect();
        let label = parts.iter().find(|(label, ..)| *label).unwrap();
        let glyph = parts.iter().find(|(label, ..)| !*label).unwrap();
        (label.1.clone(), glyph.1.clone(), glyph.2)
    }

    #[cfg(unix)]
    fn job() -> BuildJob {
        let (_, updates) = mpsc::channel();
        BuildJob::for_test(updates, "game-editor")
    }

    #[cfg(unix)]
    fn reloadable(app: &mut App) {
        let root = std::env::temp_dir().join("concerto-reload-ui");
        reload(app).target = Ok(ReloadTarget::for_test(
            Path::new("cargo"),
            &root.join("editor"),
            "game-editor",
            &root.join("target"),
        ));
        app.get_resource_mut::<ProjectState>().unwrap().project = Some(Project {
            root,
            content_root: "content".into(),
        });
    }

    #[cfg(unix)]
    #[test]
    fn the_button_label_follows_the_phase() {
        let mut app = setup();
        reloadable(&mut app);
        let theme = UITheme::default();
        let arrow = glyph::ARROW_CLOCKWISE.to_string();
        let warning = glyph::WARNING.to_string();
        let phases: [(fn() -> ReloadPhase, &str, &String, Color); 6] = [
            (|| ReloadPhase::Idle, "Reload", &arrow, theme.text_muted),
            (
                || ReloadPhase::Building(Box::new(job())),
                "Cancel",
                &arrow,
                theme.accent,
            ),
            (
                || ReloadPhase::Waiting {
                    executable: "game-editor".into(),
                },
                "Cancel",
                &arrow,
                theme.accent,
            ),
            (
                || ReloadPhase::Confirming(Box::new(job())),
                "Cancel",
                &arrow,
                theme.accent,
            ),
            (|| ReloadPhase::Closing, "Restarting…", &arrow, theme.accent),
            (|| ReloadPhase::Failed, "Reload", &warning, theme.accent),
        ];
        for (phase, label, glyph, color) in phases {
            reload(&mut app).phase = phase();
            app.main_mut().world_mut().run_schedule(LateUpdate);
            assert_eq!(
                button(&mut app),
                (label.to_string(), glyph.clone(), color),
                "{label}"
            );
        }

        reload(&mut app).phase = ReloadPhase::Waiting {
            executable: "game-editor".into(),
        };
        app.main_mut().world_mut().run_schedule(LateUpdate);
        let cancel = pressable(app.main_mut().world_mut(), "Cancel");
        click(&mut app, cancel);
        assert!(matches!(reload(&mut app).phase, ReloadPhase::Idle));

        reload(&mut app).fake_builds.push_back(job());
        app.main_mut().world_mut().run_schedule(LateUpdate);
        let start = pressable(app.main_mut().world_mut(), "Reload");
        click(&mut app, start);
        assert!(matches!(reload(&mut app).phase, ReloadPhase::Building(_)));

        reload(&mut app).phase = ReloadPhase::Closing;
        app.main_mut().world_mut().run_schedule(LateUpdate);
        let restarting = pressable(app.main_mut().world_mut(), "Restarting…");
        click(&mut app, restarting);
        assert!(matches!(reload(&mut app).phase, ReloadPhase::Closing));
    }

    #[test]
    fn the_error_list_opens_on_failure_and_closes_with_escape() {
        let mut app = setup();
        tick(&mut app);
        assert_eq!(listed_errors(app.main_mut().world_mut()), None);
        let errors = vec![
            error("cannot find value `x` in this scope", 2),
            error("cannot find value `y` in this scope", 3),
        ];

        fail(&mut app, errors.clone());
        tick(&mut app);

        assert_eq!(listed_errors(app.main_mut().world_mut()), listed(&errors));
        with_text(app.main_mut().world_mut(), "Build failed · 2 errors");
        assert_eq!(
            app.get_resource::<ActionMap>()
                .unwrap()
                .resolve(Shortcut::key(KeyCode::Escape)),
            Some(DismissBuildErrors.intern())
        );

        escape(&mut app);
        tick(&mut app);

        assert_eq!(listed_errors(app.main_mut().world_mut()), None);
        assert_eq!(
            app.get_resource::<ActionMap>()
                .unwrap()
                .resolve(Shortcut::key(KeyCode::Escape)),
            None
        );
        assert!(matches!(reload(&mut app).phase, ReloadPhase::Failed));
    }

    #[test]
    fn a_click_on_chatter_reopens_the_error_list() {
        let mut app = setup();
        tick(&mut app);
        let strip = chatter(&mut app);
        click(&mut app, strip);
        tick(&mut app);
        assert_eq!(listed_errors(app.main_mut().world_mut()), None);
        let errors = vec![error("cannot find value `x` in this scope", 2)];
        fail(&mut app, errors.clone());
        tick(&mut app);
        let close = with_text(app.main_mut().world_mut(), &glyph::X.to_string());
        click(&mut app, close);
        tick(&mut app);
        assert_eq!(listed_errors(app.main_mut().world_mut()), None);

        click(&mut app, strip);
        tick(&mut app);

        assert_eq!(listed_errors(app.main_mut().world_mut()), listed(&errors));
        with_text(app.main_mut().world_mut(), "Build failed · 1 error");
    }

    #[test]
    fn the_reload_button_is_a_window_control_above_the_drag_strip() {
        let mut app = setup();
        tick(&mut app);
        let world = app.main_mut().world_mut();
        let button = pressable(world, "Reload");

        assert!(
            world
                .get_component_for_entity::<WindowChromeControl>(button)
                .is_some()
        );
        assert_eq!(
            world
                .get_component_for_entity::<UINode>(button)
                .unwrap()
                .z_index,
            CONTROL_LAYER
        );
    }

    fn lay_out(world: &mut World, entity: Entity, height: f32) {
        let rect = UIBox {
            min: Vec2::ZERO,
            size: Vec2::new(700.0, height),
        };
        world.insert(
            UILayout {
                rect,
                content_rect: rect,
                clip_rect: rect,
                paint_order: 0,
            },
            entity,
        );
    }

    fn only<T: concerto_ecs::Component>(world: &mut World) -> Entity {
        world
            .query::<(Entity, &T), ()>()
            .iter(world)
            .map(|(entity, _)| entity)
            .next()
            .unwrap()
    }

    #[test]
    fn the_error_view_is_as_tall_as_its_errors_up_to_a_limit() {
        let mut app = setup();
        tick(&mut app);
        fail(
            &mut app,
            vec![error("cannot find value `x` in this scope", 2)],
        );
        tick(&mut app);
        let world = app.main_mut().world_mut();
        let (view, stack) = (only::<ErrorView>(world), only::<ErrorStack>(world));
        let height = |world: &mut World| {
            world
                .get_component_for_entity::<UINode>(view)
                .unwrap()
                .height
        };

        lay_out(world, view, 0.0);
        lay_out(world, stack, 105.0);
        world.run_schedule(LateUpdate);
        assert_eq!(height(world), UIValue::Px(105.0));

        lay_out(world, stack, ERROR_VIEW_MAX_HEIGHT * 3.0);
        world.run_schedule(LateUpdate);
        assert_eq!(height(world), UIValue::Px(ERROR_VIEW_MAX_HEIGHT));
    }

    #[cfg(unix)]
    #[test]
    fn the_next_build_empties_the_error_list() {
        let mut app = setup();
        reloadable(&mut app);
        fail(
            &mut app,
            vec![error("cannot find value `x` in this scope", 2)],
        );
        tick(&mut app);
        assert!(listed_errors(app.main_mut().world_mut()).is_some());

        reload(&mut app).fake_builds.push_back(job());
        app.main_mut().world_mut().trigger(ReloadGameCode);
        app.main_mut().world_mut().run_schedule(LateUpdate);

        assert!(matches!(reload(&mut app).phase, ReloadPhase::Building(_)));
        assert!(reload(&mut app).errors().is_empty());
        assert_eq!(listed_errors(app.main_mut().world_mut()), None);
        let strip = chatter(&mut app);
        click(&mut app, strip);
        app.main_mut().world_mut().run_schedule(LateUpdate);
        assert_eq!(listed_errors(app.main_mut().world_mut()), None);
    }
}
