//! The chrome around the scene: what is open, and what the editor last had to say.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_ecs::{Component, Query, Res, command::CommandQueue};
use concerto_ui::{
    material::UIMaterial,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::fonts::{MEDIUM, glyph, icon};
use crate::marks::TRANSPARENT;
use crate::project::ProjectState;
use crate::scene::SceneState;
use crate::tabs::{TabScroll, TabStrip, TabStripContent};

pub const BRAND_ID: &str = "concerto.brand";
pub const CHATTER_ID: &str = "concerto.chatter";

pub struct ShellPlugin;

impl Plugin for ShellPlugin {
    fn build(&self, app: &mut App) {
        app.add_panel(PanelDescriptor {
            id: BRAND_ID,
            title: "Concerto",
            region: Region::Brand,
        });
        app.add_panel(PanelDescriptor {
            id: CHATTER_ID,
            title: "Chatter",
            region: Region::Foot,
        });
        app.add_system(Startup, build_chrome)
            .add_system(LateUpdate, refresh_chrome);
    }
}

#[derive(Component)]
enum Label {
    Chatter,
    ChatterGlyph,
}

fn text(theme: &UITheme, value: &str) -> UIText {
    UIText {
        text: value.into(),
        font_size: theme.font_size_md,
        line_height: theme.line_height(theme.font_size_md),
        ..Default::default()
    }
}

fn build_chrome(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(brand) = registry.body(BRAND_ID) {
        cmd.entity(brand).add_child_with(
            UINode {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                align_items: Some(taffy::AlignItems::Center),
                gap: glam::Vec2::new(theme.spacing_sm, 0.0),
                ..Default::default()
            },
            |mut row| {
                row = row.add_child_with(
                    (
                        UINode {
                            width: UIValue::Px(13.0),
                            height: UIValue::Px(19.0),
                            flex_shrink: 0.0,
                            align_items: Some(taffy::AlignItems::Center),
                            padding: UIRect {
                                top: 4.0,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        UIMaterial {
                            corner_radius: 6.0,
                            ..UIMaterial::with_border(TRANSPARENT, theme.accent, 1.5)
                        },
                    ),
                    |mark| {
                        mark.add_child((
                            UINode {
                                width: UIValue::Px(4.0),
                                height: UIValue::Px(4.0),
                                flex_shrink: 0.0,
                                ..Default::default()
                            },
                            UIMaterial {
                                corner_radius: 2.0,
                                ..UIMaterial::flat(theme.accent)
                            },
                        ));
                    },
                );

                row = row.add_child((
                    UINode::default(),
                    UIText {
                        font_weight: MEDIUM,
                        ..text(&theme, "Concerto")
                    },
                ));

                row.add_child_with(
                    (
                        UINode {
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Row,
                            align_items: Some(taffy::AlignItems::Center),
                            gap: glam::Vec2::new(2.0, 0.0),
                            min_width: UIValue::Px(0.0),
                            height: UIValue::Px(30.0),
                            z_index: 70,
                            overflow_x: taffy::Overflow::Clip,
                            overflow_y: taffy::Overflow::Clip,
                            ..Default::default()
                        },
                        TabStrip,
                        concerto_ui::interaction::Interactable,
                        crate::window_chrome::WindowChromeControl,
                        TabScroll::default(),
                    ),
                    |tabs| {
                        tabs.add_child((
                            UINode {
                                flex_direction: FlexDirection::Row,
                                align_items: Some(taffy::AlignItems::Center),
                                gap: glam::Vec2::new(2.0, 0.0),
                                flex_shrink: 0.0,
                                height: UIValue::Px(30.0),
                                position: taffy::Position::Absolute,
                                inset: concerto_ui::node::UIInset {
                                    left: UIValue::Px(0.0),
                                    top: UIValue::Px(0.0),
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            TabStripContent,
                        ));
                    },
                );
            },
        );
    }

    if let Some(chatter) = registry.body(CHATTER_ID) {
        cmd.entity(chatter).add_child_with(
            (
                UINode {
                    height: UIValue::Px(30.0),
                    flex_shrink: 0.0,
                    align_items: Some(taffy::AlignItems::Center),
                    padding: UIRect::axes(0.0, theme.spacing_md),
                    ..Default::default()
                }
                .clipped(),
                UIMaterial {
                    corner_radius: theme.radius_md,
                    ..UIMaterial::flat(theme.surface)
                },
            ),
            |strip| {
                strip
                    .add_child((
                        UINode {
                            width: UIValue::Px(18.0),
                            flex_shrink: 0.0,
                            ..Default::default()
                        },
                        UIText {
                            color: theme.text_muted,
                            ..icon(&theme, glyph::INFO, theme.font_size_md)
                        },
                        Label::ChatterGlyph,
                    ))
                    .add_child((
                        UINode {
                            flex_grow: 1.0,
                            ..Default::default()
                        },
                        UIText {
                            color: theme.text_muted,
                            wrap: false,
                            ellipsis: true,
                            ..text(&theme, "")
                        },
                        Label::Chatter,
                    ));
            },
        );
    }
}

fn refresh_chrome(
    project: Res<ProjectState>,
    scenes: Res<SceneState>,
    theme: Res<UITheme>,
    labels: Query<(&Label, &mut UIText)>,
) {
    let chatter = if project.busy() || scenes.status.is_empty() {
        project.status.clone()
    } else {
        format!("{} · {}", project.status, scenes.status)
    };
    for (label, mut component) in labels.iter() {
        let value = match label {
            Label::Chatter => chatter.clone(),
            Label::ChatterGlyph => {
                let lowered = chatter.to_lowercase();
                let warning = lowered.contains("fail") || lowered.contains("error");
                let color = if warning {
                    theme.accent
                } else {
                    theme.text_muted
                };
                if component.color != color {
                    component.color = color;
                }
                if warning { glyph::WARNING } else { glyph::INFO }.to_string()
            }
        };
        if component.text != value {
            component.text = value;
        }
    }
}
