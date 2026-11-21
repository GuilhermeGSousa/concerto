//! The document tab strip.
use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_ecs::query::filter::{With, Without};
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
    material::UIMaterial,
    node::{UIInset, UILayout, UINode, UIRect},
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
    fonts::{
        AssetIcons, ICON_COLUMN, ICON_SIZE, SEMIBOLD, button_icon, draw_row_icon, glyph, icon,
    },
    hosted::{PlayState, toggle_play},
};
use concerto_foundation::assets::Asset;
use concerto_scene::scene::Scene;

const PLAY_HEIGHT: f32 = 24.0;
const PLAY_INSET: f32 = 9.0;
const PLAY_GLYPH: f32 = 13.0;

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
struct PlayGlyph {
    document: Entity,
}

#[derive(Component)]
struct TabLabel {
    document: Entity,
}

#[derive(Component)]
struct TabIcon {
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
        if app.get_resource::<AssetIcons>().is_none() {
            app.insert_resource(AssetIcons::default());
        }
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

struct PlayLook {
    glyph: char,
    ink: Color,
    border: Color,
    style: UIInteractionStyle,
}

fn play_look(theme: &UITheme, state: Option<&PlayState>) -> PlayLook {
    let wash = |color: Color, alpha: f32| {
        let color = color.to_srgba();
        Color::srgba(color.r, color.g, color.b, alpha)
    };
    match state {
        Some(PlayState::Playing) => PlayLook {
            glyph: glyph::STOP,
            ink: theme.error,
            border: theme.error,
            style: UIInteractionStyle {
                normal: wash(theme.error, 0.14),
                hovered: wash(theme.error, 0.3),
                pressed: wash(theme.error, 0.45),
                disabled: Color::TRANSPARENT,
            },
        },
        Some(PlayState::Starting) => PlayLook {
            glyph: glyph::PLAY,
            ink: theme.text_muted,
            border: theme.border,
            style: UIInteractionStyle {
                normal: Color::TRANSPARENT,
                hovered: theme.surface_hovered,
                pressed: theme.surface_hovered,
                disabled: Color::TRANSPARENT,
            },
        },
        Some(PlayState::Editing) | None => PlayLook {
            glyph: glyph::PLAY,
            ink: theme.canvas,
            border: theme.accent,
            style: UIInteractionStyle {
                normal: theme.accent,
                hovered: theme.accent_hovered,
                pressed: theme.accent_secondary,
                disabled: theme.accent,
            },
        },
    }
}

fn sync_play_buttons(
    buttons: Query<(
        &EditorTabPlay,
        &mut UIText,
        &mut UIInteractionStyle,
        &mut UIMaterial,
    )>,
    glyphs: Query<(&PlayGlyph, &mut UIText), Without<EditorTabPlay>>,
    states: Query<&PlayState>,
    theme: Res<UITheme>,
) {
    for (play, mut text, mut interaction, mut material) in buttons.iter() {
        let state = states.get_entity(play.document);
        let label = play_label(state);
        let look = play_look(&theme, state);
        if text.text != label {
            text.text = label.into();
        }
        if text.color != look.ink {
            text.color = look.ink;
        }
        if interaction.normal != look.style.normal {
            **interaction = look.style;
        }
        let border = look.border.to_linear();
        if material.border_color != border {
            material.border_color = border;
        }
    }
    for (mark, mut text) in glyphs.iter() {
        let look = play_look(&theme, states.get_entity(mark.document));
        if !text.text.chars().eq([look.glyph]) {
            text.text = look.glyph.to_string();
        }
        if text.color != look.ink {
            text.color = look.ink;
        }
    }
}

fn sync_tabs(
    mut cmd: CommandQueue,
    contents: Query<Entity, With<TabStripContent>>,
    documents: Query<(Entity, &EditorDocument)>,
    unsaved: Query<&Unsaved>,
    (labels, tab_icons): (
        Query<(&TabLabel, &mut UIText)>,
        Query<(&TabIcon, &mut UIText), Without<TabLabel>>,
    ),
    existing_tabs: Query<&EditorTab>,
    tab_button_entities: Query<(Entity, &EditorTab)>,
    tab_buttons: Query<(&EditorTab, &mut UIInteractionStyle)>,
    active: Res<ActiveEditor>,
    theme: Res<UITheme>,
    (registry, icons): (Res<AssetEditorRegistry>, Res<AssetIcons>),
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
            .map(|asset| format!("{} ·", asset.name()))
            .or_else(|| {
                document
                    .current
                    .as_ref()
                    .map(|asset| asset.name().into_owned())
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
        let mark = icons.glyph(kind);
        if let Some((_, mut text)) = tab_icons.iter().find(|(icon, _)| icon.document == entity) {
            draw_row_icon(&mut text, &theme, mark, is_active);
        }
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
            let ink = if is_active {
                theme.text
            } else {
                theme.text_muted
            };
            let closable = registry.closable(document.asset_type);
            let plays = document.asset_type == Scene::name();
            let state = states.get_entity(entity);
            let play = play_label(state);
            let look = play_look(&theme, state);
            cmd.entity(content).add_child_with(
                (
                    theme
                        .pressable()
                        .tab()
                        .selected(is_active)
                        .radius_md()
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
                                .with_width(UIValue::Px(ICON_COLUMN))
                                .with_flex_shrink(0.0)
                                .with_padding(UIRect::axes(0.0, (ICON_COLUMN - ICON_SIZE) / 2.0))
                                .with_margin(UIRect {
                                    right: theme.spacing_xs,
                                    ..UIRect::default()
                                }),
                            icon(&theme, mark, ICON_SIZE).color(ink),
                            TabIcon { document: entity },
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
                                .with_size(
                                    UIValue::Px(theme.control_height),
                                    UIValue::Px(theme.control_height),
                                )
                                .with_padding(UIRect::axes(
                                    (theme.control_height - theme.line_height(ICON_SIZE)) / 2.0,
                                    (theme.control_height - ICON_SIZE) / 2.0,
                                ))
                                .with_flex_shrink(0.0)
                                .with_z_index(71),
                            button_icon(&theme, glyph::X),
                            Interactable,
                            EditorTabClose { document: entity },
                            close_tab.into_listener(),
                            WindowChromeControl,
                        ));
                    } else if plays {
                        button.add_child_with(
                            UINode::default()
                                .with_flex_shrink(0.0)
                                .with_margin(UIRect {
                                    left: theme.spacing_md,
                                    ..UIRect::default()
                                })
                                .with_z_index(71),
                            |mut pill| {
                                pill.add_child((
                                    theme
                                        .button(play)
                                        .height(UIValue::Px(PLAY_HEIGHT))
                                        .padding(UIRect {
                                            left: PLAY_INSET + PLAY_GLYPH + theme.spacing_xs,
                                            right: PLAY_INSET + 2.0,
                                            top: (PLAY_HEIGHT
                                                - theme.line_height(theme.font_size_md))
                                                / 2.0,
                                            bottom: 0.0,
                                        })
                                        .radius(PLAY_HEIGHT / 2.0)
                                        .border(look.border, 1.0)
                                        .normal(look.style.normal)
                                        .hovered(look.style.hovered)
                                        .pressed(look.style.pressed)
                                        .disabled_color(look.style.disabled)
                                        .weight(SEMIBOLD)
                                        .color(look.ink)
                                        .on_click(press_play),
                                    EditorTabPlay { document: entity },
                                    WindowChromeControl,
                                ))
                                .add_child((
                                    UINode::default()
                                        .with_position(taffy::Position::Absolute)
                                        .with_inset(UIInset {
                                            left: UIValue::Px(PLAY_INSET),
                                            top: UIValue::Px(
                                                (PLAY_HEIGHT - theme.line_height(PLAY_GLYPH)) / 2.0,
                                            ),
                                            ..Default::default()
                                        }),
                                    icon(&theme, look.glyph, PLAY_GLYPH).color(look.ink),
                                    PlayGlyph { document: entity },
                                ));
                            },
                        );
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
