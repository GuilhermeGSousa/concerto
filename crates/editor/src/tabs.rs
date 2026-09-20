//! The document tab strip.
//!
//! Tabs are deliberately just views over editor document entities.  The
//! document owns the asset and its state; this module owns the small amount of
//! chrome needed to activate or close it.
use app::{schedule_groups::LateUpdate, App, Plugin};
use ecs::query::filter::With;
use ecs::{
    command::CommandQueue, entity::hierarchy::ChildOf, events::event_reader::EventReader,
    Component, Entity, Query, Res, ResMut,
};
use taffy::FlexDirection;
use ui::{
    interaction::HoveredNode,
    interaction::{Interactable, UIClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UILayout, UINode, UIRect},
    text::TextComponent,
    theme::UITheme,
    transform::UIValue,
};
use window::winit_events::WindowEvent;
use winit::event::{MouseScrollDelta, WindowEvent as WinitWindowEvent};

use crate::window_chrome::WindowChromeControl;
use crate::{
    asset_editor::{ActiveEditor, AssetEditorCommand, AssetEditorCommands, EditorDocument},
    fonts::{glyph, icon},
};

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

#[derive(Component)]
struct TabLabel {
    document: Entity,
}

/// Marker for the scrolling viewport created by the shell.
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
        app.add_system(LateUpdate, handle_tab_clicks)
            .add_system(LateUpdate, scroll_tabs)
            .add_system(LateUpdate, sync_tabs);
    }
}

fn handle_tab_clicks(
    mut clicks: ecs::events::event_reader::EventReader<UIClick>,
    tabs: Query<&EditorTab>,
    closes: Query<&EditorTabClose>,
    mut commands: ResMut<AssetEditorCommands>,
) {
    for click in clicks.read() {
        if let Some(close) = closes.get_entity(click.entity) {
            commands
                .0
                .push_back(AssetEditorCommand::Close(close.document));
        } else if let Some(tab) = tabs.get_entity(click.entity) {
            commands
                .0
                .push_back(AssetEditorCommand::Activate(tab.document));
        }
    }
}

fn sync_tabs(
    mut cmd: CommandQueue,
    contents: Query<Entity, With<TabStripContent>>,
    documents: Query<(Entity, &EditorDocument)>,
    labels: Query<(&TabLabel, &mut TextComponent)>,
    existing_tabs: Query<&EditorTab>,
    tab_button_entities: Query<(Entity, &EditorTab)>,
    tab_buttons: Query<(&EditorTab, &mut UIMaterial, &mut UIInteractionStyle)>,
    active: Res<ActiveEditor>,
    theme: Res<UITheme>,
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
        for (tab, mut material, mut interaction) in tab_buttons.iter() {
            if tab.document == entity {
                let background = if is_active {
                    theme.surface_raised
                } else {
                    theme.surface
                };
                material.color = background.to_linear();
                interaction.normal = background;
            }
        }
        if !existing_tabs.iter().any(|tab| tab.document == entity) {
            let button = cmd
                .spawn((
                    UINode {
                        height: UIValue::Px(30.0),
                        flex_direction: FlexDirection::Row,
                        align_items: Some(taffy::AlignItems::Center),
                        padding: UIRect::axes(0.0, theme.spacing_sm),
                        flex_shrink: 0.0,
                        z_index: 70,
                        ..Default::default()
                    },
                    UIMaterial::flat(if is_active {
                        theme.surface_raised
                    } else {
                        theme.surface
                    }),
                    UIInteractionStyle {
                        normal: if is_active {
                            theme.surface_raised
                        } else {
                            theme.surface
                        },
                        hovered: theme.surface_hovered,
                        pressed: theme.accent,
                        disabled: theme.surface,
                    },
                    Interactable,
                    EditorTab { document: entity },
                    WindowChromeControl,
                ))
                .entity();
            cmd.add_child(content, button);
            let mark = match kind {
                "Scene" => glyph::CUBE,
                "Texture" => glyph::IMAGE,
                _ => glyph::FILE,
            };
            let mark_entity = cmd
                .spawn((
                    UINode {
                        width: UIValue::Px(16.0),
                        flex_shrink: 0.0,
                        ..Default::default()
                    },
                    TextComponent {
                        color: if is_active {
                            theme.text
                        } else {
                            theme.text_muted
                        },
                        ..icon(&theme, mark, theme.font_size_sm)
                    },
                ))
                .entity();
            cmd.add_child(button, mark_entity);
            let label = cmd
                .spawn((
                    UINode {
                        flex_shrink: 1.0,
                        max_width: UIValue::Px(220.0),
                        ..Default::default()
                    },
                    TextComponent {
                        text: title,
                        wrap: false,
                        ellipsis: true,
                        font_size: theme.font_size_md,
                        line_height: theme.line_height(theme.font_size_md),
                        color: if is_active {
                            theme.text
                        } else {
                            theme.text_muted
                        },
                        ..Default::default()
                    },
                    TabLabel { document: entity },
                ))
                .entity();
            cmd.add_child(button, label);
            let close = cmd
                .spawn((
                    UINode {
                        width: UIValue::Px(18.0),
                        height: UIValue::Px(24.0),
                        padding: UIRect::axes(3.0, 3.0),
                        flex_shrink: 0.0,
                        z_index: 71,
                        ..Default::default()
                    },
                    TextComponent {
                        color: theme.text_muted,
                        ..icon(&theme, glyph::X, theme.font_size_sm)
                    },
                    Interactable,
                    EditorTabClose { document: entity },
                    WindowChromeControl,
                ))
                .entity();
            cmd.add_child(button, close);
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
        if let WinitWindowEvent::MouseWheel { delta: wheel, .. } = &**event {
            delta += match wheel {
                MouseScrollDelta::LineDelta(x, y) => -if x.abs() > 0.0 { *x } else { *y } * 28.0,
                MouseScrollDelta::PixelDelta(value) => {
                    -if value.x.abs() > 0.0 {
                        value.x
                    } else {
                        value.y
                    } as f32
                }
            };
        }
    }
    scroll.offset += delta;
    let width = strip_layout.rect.size.x;
    // Child bounds remain authoritative even when the absolutely positioned
    // content box is constrained to the viewport's available width by layout.
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
