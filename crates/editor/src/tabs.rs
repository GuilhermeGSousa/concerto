//! The document tab strip.
use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::query::filter::With;
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut,
    command::CommandQueue,
    entity::hierarchy::ChildOf,
    events::event_reader::EventReader,
    signal::{On, listener::IntoListener},
};
use concerto_ui::{
    elements::prelude::*,
    interaction::HoveredNode,
    interaction::{Interactable, UIClick, UIInteractionStyle},
    node::{UILayout, UINode, UIRect},
    sets::UiSet,
    text::UIText,
    theme::{ButtonVariant, UITheme},
    transform::UIValue,
};
use concerto_window::events::{ScrollDelta, WindowEvent};
use concerto_window::input::MouseButton;

use crate::window_chrome::WindowChromeControl;
use crate::{
    asset_editor::{
        ActiveEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry, EditorDocument,
        Unsaved,
    },
    fonts::{glyph, icon},
    hosted::{PlayState, toggle_play},
};
use concerto_foundation::assets::Asset;
use concerto_scene::scene::Scene;

/// Entity in the tab bar that activates the associated document.
#[derive(Component)]
pub struct EditorTab {
    pub document: Entity,
}

/// Close button nested in an [`EditorTab`].
#[derive(Component)]
pub struct EditorTabClose {
    pub document: Entity,
}

/// Play/Stop button nested in the Scene tab, which cannot be closed.
#[derive(Component)]
pub struct EditorTabPlay {
    pub document: Entity,
}

#[derive(Component)]
struct TabLabel {
    document: Entity,
}

#[derive(Component)]
pub struct TabStrip;

#[derive(Component, Default)]
pub struct TabStripContent;

#[derive(Component, Default)]
pub struct TabScroll {
    offset: f32,
    revealed: Option<Entity>,
}

pub struct TabsPlugin;

impl Plugin for TabsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, scroll_tabs)
            .add_system(LateUpdate, sync_tabs.before(UiSet::Materials))
            .add_system(
                LateUpdate,
                sync_play_buttons
                    .after(UiSet::Input)
                    .before(UiSet::Materials),
            );
    }
}

fn activate_tab(
    on: On<UIClick>,
    tabs: Query<&EditorTab>,
    mut commands: ResMut<AssetEditorCommands>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if let Some(tab) = tabs.get_entity(on.entity()) {
        commands
            .0
            .push_back(AssetEditorCommand::Activate(tab.document));
    }
}

fn close_tab(
    on: On<UIClick>,
    closes: Query<&EditorTabClose>,
    mut commands: ResMut<AssetEditorCommands>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if let Some(close) = closes.get_entity(on.entity()) {
        commands
            .0
            .push_back(AssetEditorCommand::Close(close.document));
    }
}

fn play_label(state: Option<&PlayState>) -> &'static str {
    match state {
        Some(PlayState::Starting) => "Starting…",
        Some(PlayState::Playing) => "Stop",
        Some(PlayState::Editing) | None => "Play",
    }
}

pub(crate) fn press_play(
    on: On<UIClick>,
    plays: Query<&EditorTabPlay>,
    states: Query<&PlayState>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(play) = plays.get_entity(on.entity()) else {
        return;
    };
    toggle_play(
        play.document,
        states.get_entity(play.document).copied(),
        &mut cmd,
    );
}

fn sync_play_buttons(buttons: Query<(&EditorTabPlay, &mut UIText)>, states: Query<&PlayState>) {
    for (play, mut text) in buttons.iter() {
        let label = play_label(states.get_entity(play.document));
        if text.text != label {
            text.text = label.into();
        }
    }
}

fn sync_tabs(
    mut cmd: CommandQueue,
    contents: Query<Entity, With<TabStripContent>>,
    documents: Query<(Entity, &EditorDocument)>,
    unsaved: Query<&Unsaved>,
    labels: Query<(&TabLabel, &mut UIText)>,
    existing_tabs: Query<&EditorTab>,
    tab_button_entities: Query<(Entity, &EditorTab)>,
    tab_buttons: Query<(&EditorTab, &mut UIInteractionStyle)>,
    active: Res<ActiveEditor>,
    theme: Res<UITheme>,
    registry: Res<AssetEditorRegistry>,
    states: Query<&PlayState>,
) {
    let Some(content) = contents.iter().next() else {
        return;
    };
    let mut ordered: Vec<_> = documents
        .iter()
        .map(|(entity, document)| (entity, document.order))
        .collect();
    ordered.sort_by_key(|(_, order)| *order);
    for (entity, _) in ordered {
        let Some((_, document)) = documents.get_entity(entity) else {
            continue;
        };
        let title = document
            .pending
            .as_ref()
            .map(|asset| format!("{} ·", asset.display_name))
            .or_else(|| {
                document
                    .current
                    .as_ref()
                    .map(|asset| asset.display_name.clone())
            })
            .unwrap_or_else(|| {
                if document.status.is_empty() {
                    document.title.clone()
                } else {
                    format!("{} · error", document.title)
                }
            });
        let title = if unsaved.get_entity(entity).is_some() {
            format!("• {title}")
        } else {
            title
        };
        let kind = document
            .pending
            .as_ref()
            .or(document.current.as_ref())
            .map(|asset| asset.kind.as_str())
            .unwrap_or("Asset");
        let is_active = active.0 == Some(entity);
        if let Some((_, mut label)) = labels.iter().find(|(label, _)| label.document == entity) {
            if label.text != title {
                label.text = title.clone();
            }
            let color = if is_active {
                theme.text
            } else {
                theme.text_muted
            };
            if label.color != color {
                label.color = color;
            }
        }
        for (tab, mut interaction) in tab_buttons.iter() {
            if tab.document == entity {
                let style = theme.interaction(ButtonVariant::Tab, is_active);
                if interaction.normal != style.normal {
                    **interaction = style;
                }
            }
        }
        if !existing_tabs.iter().any(|tab| tab.document == entity) {
            let mark = match kind {
                "Scene" => glyph::CUBE,
                "Texture" => glyph::IMAGE,
                _ => glyph::FILE,
            };
            let ink = if is_active {
                theme.text
            } else {
                theme.text_muted
            };
            let closable = registry.closable(document.asset_type);
            let plays = document.asset_type == Scene::name();
            let play = play_label(states.get_entity(entity));
            cmd.entity(content).add_child_with(
                (
                    theme
                        .pressable()
                        .tab()
                        .selected(is_active)
                        .radius(0.0)
                        .height(UIValue::Px(30.0))
                        .row()
                        .padding(UIRect::axes(0.0, theme.spacing_sm))
                        .z_index(70)
                        .on_click(activate_tab),
                    EditorTab { document: entity },
                    WindowChromeControl,
                ),
                |mut button| {
                    let button = button
                        .add_child((
                            UINode::default()
                                .with_width(UIValue::Px(16.0))
                                .with_flex_shrink(0.0),
                            icon(&theme, mark, theme.font_size_sm).color(ink),
                        ))
                        .add_child((
                            theme
                                .label(title.clone())
                                .single_line()
                                .color(ink)
                                .max_width(UIValue::Px(220.0)),
                            TabLabel { document: entity },
                        ));
                    if closable {
                        button.add_child((
                            UINode::default()
                                .with_size(UIValue::Px(18.0), UIValue::Px(24.0))
                                .with_padding(UIRect::axes(3.0, 3.0))
                                .with_flex_shrink(0.0)
                                .with_z_index(71),
                            icon(&theme, glyph::X, theme.font_size_sm).muted(),
                            Interactable,
                            EditorTabClose { document: entity },
                            close_tab.into_listener(),
                            WindowChromeControl,
                        ));
                    } else if plays {
                        button.add_child((
                            theme
                                .button(play)
                                .height(UIValue::Px(22.0))
                                .padding(UIRect::axes(0.0, theme.spacing_sm))
                                .margin(UIRect {
                                    left: theme.spacing_sm,
                                    ..UIRect::default()
                                })
                                .small()
                                .z_index(71)
                                .on_click(press_play),
                            EditorTabPlay { document: entity },
                            WindowChromeControl,
                        ));
                    }
                },
            );
        }
    }

    for (button, tab) in tab_button_entities.iter() {
        if !documents.iter().any(|(entity, _)| entity == tab.document) {
            cmd.despawn(button);
        }
    }
}

fn scroll_tabs(
    mut events: EventReader<WindowEvent>,
    hovered: Res<HoveredNode>,
    parents: Query<&ChildOf>,
    strips: Query<(Entity, &TabStrip, &mut TabScroll, &UILayout)>,
    contents: Query<(&TabStripContent, &mut UINode, &UILayout)>,
    tabs: Query<(&EditorTab, &UILayout)>,
    active: Res<ActiveEditor>,
) {
    let Some((strip_entity, _, mut scroll, strip_layout)) = strips.iter().next() else {
        return;
    };
    let Some((_, mut content, content_layout)) = contents.iter().next() else {
        return;
    };
    let over_strip = (**hovered).is_some_and(|mut node| {
        let mut visited = std::collections::HashSet::new();
        while visited.insert(node) {
            if node == strip_entity {
                return true;
            }
            let Some(parent) = parents.get_entity(node) else {
                break;
            };
            node = parent.parent();
        }
        false
    });
    let mut delta = 0.0;
    for event in events.read() {
        if !over_strip {
            continue;
        }
        if let WindowEvent::MouseWheel { delta: wheel } = event {
            let along = |value: &glam::Vec2| {
                if value.x.abs() > 0.0 {
                    value.x
                } else {
                    value.y
                }
            };
            delta += match wheel {
                ScrollDelta::Lines(lines) => -along(lines) * 28.0,
                ScrollDelta::Pixels(pixels) => -along(pixels),
            };
        }
    }
    scroll.offset += delta;
    let width = strip_layout.rect.size.x;
    let extent = tabs
        .iter()
        .map(|(_, layout)| layout.rect.max().x - content_layout.rect.min.x)
        .fold(0.0_f32, f32::max);
    if scroll.revealed != active.0 {
        if let Some((_, layout)) = tabs.iter().find(|(tab, _)| Some(tab.document) == active.0) {
            let left = layout.rect.min.x - content_layout.rect.min.x;
            let right = layout.rect.max().x - content_layout.rect.min.x;
            if left < scroll.offset {
                scroll.offset = left;
            } else if right > scroll.offset + width {
                scroll.offset = right - width;
            }
            scroll.revealed = active.0;
        } else if active.0.is_none() {
            scroll.revealed = None;
        }
    }
    scroll.offset = scroll.offset.clamp(0.0, (extent - width).max(0.0));
    let left = UIValue::Px(-scroll.offset);
    if content.inset.left != left {
        content.inset.left = left;
    }
}
