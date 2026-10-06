//! The chrome around the scene: what is open, and what the editor last had to say.
use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup},
};
use concerto_color::Color;
use concerto_ecs::{Component, Query, Res, command::CommandQueue};
use concerto_ui::{
    elements::prelude::*,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::fonts::{MEDIUM, glyph, icon};
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

fn build_chrome(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(brand) = registry.body(BRAND_ID) {
        cmd.entity(brand)
            .add_child_with(theme.row().grow(), |mut row| {
                row = row.add_child_with(
                    theme
                        .canvas()
                        .fill(Color::TRANSPARENT)
                        .border(theme.accent, 1.5)
                        .radius(6.0)
                        .size(UIValue::Px(13.0), UIValue::Px(19.0))
                        .fixed()
                        .align_items(taffy::AlignItems::Center)
                        .padding(UIRect {
                            top: 4.0,
                            ..Default::default()
                        }),
                    |mark| {
                        mark.add_child(
                            theme
                                .canvas()
                                .fill(theme.accent)
                                .radius(2.0)
                                .size(UIValue::Px(4.0), UIValue::Px(4.0))
                                .fixed(),
                        );
                    },
                );

                row = row.add_child(theme.label("Concerto").weight(MEDIUM));

                row.add_child_with(
                    (
                        UINode::default()
                            .with_flex_grow(1.0)
                            .with_flex_direction(FlexDirection::Row)
                            .with_align_items(taffy::AlignItems::Center)
                            .with_gap(glam::Vec2::new(2.0, 0.0))
                            .with_min_width(UIValue::Px(0.0))
                            .with_height(UIValue::Px(30.0))
                            .with_z_index(70)
                            .with_overflow_x(taffy::Overflow::Clip)
                            .with_overflow_y(taffy::Overflow::Clip),
                        TabStrip,
                        concerto_ui::interaction::Interactable,
                        crate::window_chrome::WindowChromeControl,
                        TabScroll::default(),
                    ),
                    |tabs| {
                        tabs.add_child((
                            UINode::default()
                                .with_flex_direction(FlexDirection::Row)
                                .with_align_items(taffy::AlignItems::Center)
                                .with_gap(glam::Vec2::new(2.0, 0.0))
                                .with_flex_shrink(0.0)
                                .with_height(UIValue::Px(30.0))
                                .with_position(taffy::Position::Absolute)
                                .with_inset(concerto_ui::node::UIInset {
                                    left: UIValue::Px(0.0),
                                    top: UIValue::Px(0.0),
                                    ..Default::default()
                                }),
                            TabStripContent,
                        ));
                    },
                );
            });
    }

    if let Some(chatter) = registry.body(CHATTER_ID) {
        cmd.entity(chatter).add_child_with(
            theme
                .panel()
                .radius_md()
                .height(UIValue::Px(30.0))
                .fixed()
                .align_items(taffy::AlignItems::Center)
                .padding(UIRect::axes(0.0, theme.spacing_md))
                .clipped(),
            |strip| {
                strip
                    .add_child((
                        UINode::default()
                            .with_width(UIValue::Px(18.0))
                            .with_flex_shrink(0.0),
                        icon(&theme, glyph::INFO, theme.font_size_md).muted(),
                        Label::ChatterGlyph,
                    ))
                    .add_child((theme.label("").muted().single_line().grow(), Label::Chatter));
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
