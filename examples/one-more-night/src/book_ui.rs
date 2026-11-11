use concerto::{
    color::Color,
    ecs::{CommandQueue, Component, Entity, Query, Res},
    ui::{
        material::UIMaterial,
        node::{FlexDirection, UINode, UIRect},
        text::UIText,
        transform::UIValue,
    },
};
use glam::Vec2;

use crate::book::Casebook;
use crate::casefile::CaseData;
use crate::session::{BookBody, BookScreen, Page, Session};

const HOUR_WIDTH: f32 = 54.0;
/// The fill of a screen's panel.
pub const PANEL: Color = Color::srgba(0.04, 0.04, 0.07, 0.9);
const CLEAR: Color = Color::srgba(0.0, 0.0, 0.0, 0.0);
const CHOSEN: Color = Color::srgba(0.45, 0.33, 0.12, 0.95);
const NOTED: Color = Color::srgba(0.2, 0.22, 0.3, 0.95);
const CELL: Color = Color::srgba(0.1, 0.1, 0.14, 0.9);
const INK: Color = Color::srgba(0.945, 0.925, 0.885, 1.0);
const MUTED: Color = Color::srgba(0.66, 0.64, 0.6, 1.0);
const GOLD: Color = Color::srgba(0.9, 0.72, 0.32, 1.0);
const PORTRAITS: [Color; 3] = [
    Color::srgba(0.5, 0.2, 0.26, 1.0),
    Color::srgba(0.2, 0.34, 0.5, 1.0),
    Color::srgba(0.28, 0.42, 0.26, 1.0),
];

/// The panel a player's open casebook is drawn in, and the screen it last drew.
#[derive(Component)]
pub struct BookPanel {
    pub player: u8,
    shown: Option<BookScreen>,
}

impl BookPanel {
    pub fn new(player: u8) -> Self {
        Self {
            player,
            shown: None,
        }
    }
}

/// The node of an empty, hidden casebook panel.
pub fn book_panel_node() -> UINode {
    UINode::default()
        .with_flex_direction(FlexDirection::Column)
        .with_gap(Vec2::splat(10.0))
        .with_padding(UIRect::all(14.0))
        .with_visible(false)
}

fn label(text: impl Into<String>, size: f32, colour: Color) -> UIText {
    UIText {
        text: text.into(),
        font_size: size,
        line_height: size * 1.35,
        color: colour,
        ..Default::default()
    }
}

fn row() -> UINode {
    UINode::default()
        .with_flex_direction(FlexDirection::Row)
        .with_flex_shrink(0.0)
        .with_gap(Vec2::splat(8.0))
}

fn column() -> UINode {
    UINode::default()
        .with_flex_direction(FlexDirection::Column)
        .with_gap(Vec2::splat(6.0))
}

fn fill(colour: Color, radius: f32) -> UIMaterial {
    UIMaterial {
        corner_radius: radius,
        ..UIMaterial::flat(colour)
    }
}

fn inset(top: f32, left: f32) -> UIRect {
    UIRect {
        top,
        right: 0.0,
        bottom: 0.0,
        left,
    }
}

fn facts_column(cmd: &mut CommandQueue, parent: Entity, facts: &[String]) {
    let list = cmd
        .entity(parent)
        .spawn_child(column().with_flex_grow(1.0))
        .entity();
    if facts.is_empty() {
        cmd.entity(list)
            .add_child((UINode::default(), label("Nothing noted yet.", 17.0, MUTED)));
    }
    for fact in facts {
        cmd.entity(list)
            .add_child((UINode::default(), label(fact.clone(), 17.0, INK)));
    }
}

fn build(cmd: &mut CommandQueue, panel: Entity, screen: &BookScreen) {
    let tabs = cmd.entity(panel).spawn_child(row()).entity();
    for page in Page::ALL {
        let chosen = page == screen.page;
        cmd.entity(tabs).add_child((
            UINode::default().with_padding(UIRect {
                top: 4.0,
                right: 10.0,
                bottom: 4.0,
                left: 10.0,
            }),
            fill(if chosen { CHOSEN } else { CLEAR }, 5.0),
            label(page.title(), 17.0, if chosen { INK } else { MUTED }),
        ));
    }
    match &screen.body {
        BookBody::Subjects {
            subjects,
            selected,
            facts,
        } => {
            let body = cmd
                .entity(panel)
                .spawn_child(row().with_gap(Vec2::splat(18.0)))
                .entity();
            let list = cmd
                .entity(body)
                .spawn_child(column().with_width(UIValue::Percent(42.0)))
                .entity();
            for (index, subject) in subjects.iter().enumerate() {
                let line = cmd
                    .entity(list)
                    .spawn_child((
                        row().with_padding(UIRect::all(5.0)),
                        fill(if index == *selected { CHOSEN } else { CLEAR }, 5.0),
                    ))
                    .entity();
                if let Some(portrait) = subject.portrait {
                    let initial = subject.label.chars().next().unwrap_or(' ').to_string();
                    cmd.entity(line).add_child((
                        UINode::default()
                            .with_size(UIValue::Px(26.0), UIValue::Px(26.0))
                            .with_padding(inset(2.0, 7.0)),
                        fill(PORTRAITS[portrait % PORTRAITS.len()], 13.0),
                        label(initial, 16.0, INK),
                    ));
                }
                cmd.entity(line).add_child((
                    UINode::default().with_flex_grow(1.0),
                    label(subject.label.clone(), 18.0, INK),
                ));
                if subject.count > 0 {
                    cmd.entity(line).add_child((
                        UINode::default(),
                        label(subject.count.to_string(), 16.0, GOLD),
                    ));
                }
            }
            facts_column(cmd, body, facts);
        }
        BookBody::Timeline {
            rooms,
            hours,
            counts,
            selected,
            facts,
        } => {
            let grid = cmd
                .entity(panel)
                .spawn_child(column().with_gap(Vec2::splat(3.0)))
                .entity();
            let width = UIValue::Percent(88.0 / rooms.len().max(1) as f32);
            let header = cmd
                .entity(grid)
                .spawn_child(row().with_gap(Vec2::splat(3.0)))
                .entity();
            cmd.entity(header)
                .add_child(UINode::default().with_width(UIValue::Px(HOUR_WIDTH)));
            for room in rooms {
                cmd.entity(header).add_child((
                    UINode::default().with_width(width),
                    label(room.clone(), 15.0, MUTED),
                ));
            }
            for (hour, name) in hours.iter().enumerate() {
                let line = cmd
                    .entity(grid)
                    .spawn_child(row().with_gap(Vec2::splat(3.0)))
                    .entity();
                cmd.entity(line).add_child((
                    UINode::default().with_width(UIValue::Px(HOUR_WIDTH)),
                    label(name.clone(), 15.0, MUTED),
                ));
                for room in 0..rooms.len() {
                    let cell = hour * rooms.len() + room;
                    let count = counts.get(cell).copied().unwrap_or(0);
                    let colour = if cell == *selected {
                        CHOSEN
                    } else if count > 0 {
                        NOTED
                    } else {
                        CELL
                    };
                    cmd.entity(line).add_child((
                        UINode::default()
                            .with_size(width, UIValue::Px(24.0))
                            .with_padding(inset(1.0, 8.0)),
                        fill(colour, 4.0),
                        label("o ".repeat(count), 15.0, GOLD),
                    ));
                }
            }
            facts_column(cmd, panel, facts);
        }
        BookBody::Lines { lines, cursor } => {
            let list = cmd.entity(panel).spawn_child(column()).entity();
            for (index, line) in lines.iter().enumerate() {
                cmd.entity(list).add_child((
                    UINode::default().with_padding(UIRect::all(5.0)),
                    fill(
                        if Some(index) == *cursor {
                            CHOSEN
                        } else {
                            CLEAR
                        },
                        5.0,
                    ),
                    label(line.clone(), 18.0, INK),
                ));
            }
        }
    }
    cmd.entity(panel)
        .add_child((UINode::default(), label(screen.hint.clone(), 15.0, MUTED)));
}

/// Shows each player's open casebook in their panel, rebuilding it when what it shows changes.
pub fn show_book(
    panels: Query<(Entity, &mut BookPanel, &mut UINode)>,
    session: Res<Session>,
    book: Res<Casebook>,
    data: Res<CaseData>,
    mut cmd: CommandQueue,
) {
    for (entity, mut panel, mut node) in panels.iter() {
        let screen = session.book_screen(panel.player, &book, &data);
        if node.visible != screen.is_some() {
            node.visible = screen.is_some();
        }
        if panel.shown == screen {
            continue;
        }
        cmd.entity(entity).despawn_children();
        if let Some(screen) = &screen {
            build(&mut cmd, entity, screen);
        }
        panel.shown = screen;
    }
}
