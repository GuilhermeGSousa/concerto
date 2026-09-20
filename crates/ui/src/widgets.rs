//! Small themed widget building blocks. These components describe interaction
//! without introducing editor concepts into the UI crate.
use color::Color;
use ecs::{
    component::Component,
    entity::Entity,
    events::{Event, event_reader::EventReader, event_writer::EventWriter},
    query::Query,
    resource::Res,
};

use crate::{
    interaction::{HoveredNode, Interactable, UIClick, UIInteractionStyle},
    material::UIMaterial,
    node::{UINode, UIRect},
    theme::UITheme,
    transform::UIValue,
};

#[derive(Component, Default)]
pub struct UIButton;

#[derive(Component, Default)]
pub struct UIIconButton;

#[derive(Component, Default)]
pub struct UISearchField;

#[derive(Component)]
pub struct UITooltip {
    pub target: Entity,
}

#[derive(Component, Default)]
pub struct UIPopupMenu {
    pub open: bool,
}

#[derive(Component)]
pub struct UIPopupTrigger {
    pub menu: Entity,
}

#[derive(Component)]
pub struct UICollapsibleSection {
    pub expanded: bool,
    pub content: Entity,
}

#[derive(Component)]
pub struct UITab {
    pub strip: Entity,
    pub index: usize,
}

#[derive(Component, Default)]
pub struct UITabStrip {
    pub selected: usize,
}

/// The content shown when its tab is the selected one.
///
/// Without this a strip renders every panel stacked on top of the others: the
/// strip tracks a selection, but a selection nothing acts on is just a number.
#[derive(Component)]
pub struct UITabBody {
    pub strip: Entity,
    pub index: usize,
}

#[derive(Component, Default)]
pub struct UIPropertyRow;

#[derive(Event)]
pub struct UICollapsibleChanged {
    pub entity: Entity,
    pub expanded: bool,
}

#[derive(Event)]
pub struct UITabChanged {
    pub entity: Entity,
    pub selected: usize,
}

/// Standard compact Wonderland button visuals. Callers add their own action
/// component and optional `TextComponent` to the returned bundle.
pub fn button(
    theme: &UITheme,
) -> (
    UINode,
    UIMaterial,
    Interactable,
    UIInteractionStyle,
    UIButton,
) {
    (
        UINode {
            height: UIValue::Px(theme.control_height),
            flex_shrink: 0.0,
            padding: UIRect::axes(theme.spacing_sm, theme.spacing_md),
            ..Default::default()
        },
        UIMaterial::with_border(theme.surface_raised, theme.border, 1.0),
        Interactable,
        UIInteractionStyle {
            normal: theme.surface_raised,
            hovered: theme.surface_hovered,
            pressed: theme.accent,
            disabled: Color::srgba(0.09, 0.075, 0.11, 0.55),
        },
        UIButton,
    )
}

pub(crate) fn update_widgets(
    mut clicks: EventReader<UIClick>,
    collapsibles: Query<&mut UICollapsibleSection>,
    tabs: Query<&UITab>,
    strips: Query<&mut UITabStrip>,
    nodes: Query<&mut UINode>,
    mut collapsible_events: EventWriter<UICollapsibleChanged>,
    mut tab_events: EventWriter<UITabChanged>,
) {
    for click in clicks.read() {
        if let Some(mut section) = collapsibles.get_entity(click.entity) {
            section.expanded = !section.expanded;
            if let Some(mut node) = nodes.get_entity(section.content) {
                node.visible = section.expanded;
            }
            collapsible_events.write(UICollapsibleChanged {
                entity: click.entity,
                expanded: section.expanded,
            });
        }
        if let Some(tab) = tabs.get_entity(click.entity)
            && let Some(mut strip) = strips.get_entity(tab.strip)
        {
            strip.selected = tab.index;
            tab_events.write(UITabChanged {
                entity: tab.strip,
                selected: tab.index,
            });
        }
    }
}

/// Shows the body whose index matches its strip's selection, and hides the
/// rest. A selection past the end — a panel removed while its tab was current —
/// hides everything rather than falling back to showing all of them.
pub fn sync_tab_bodies(strips: Query<&UITabStrip>, bodies: Query<(&UITabBody, &mut UINode)>) {
    for (body, mut node) in bodies.iter() {
        let Some(strip) = strips.get_entity(body.strip) else {
            continue;
        };
        let visible = strip.selected == body.index;
        if node.visible != visible {
            node.visible = visible;
        }
    }
}

pub(crate) fn update_tooltips(
    hovered: Res<HoveredNode>,
    tooltips: Query<(&UITooltip, &mut UINode)>,
) {
    for (tooltip, mut node) in tooltips.iter() {
        node.visible = **hovered == Some(tooltip.target);
    }
}

pub(crate) fn update_popup_menus(
    mut clicks: EventReader<UIClick>,
    triggers: Query<&UIPopupTrigger>,
    menus: Query<(&mut UIPopupMenu, &mut UINode)>,
) {
    for click in clicks.read() {
        let Some(trigger) = triggers.get_entity(click.entity) else {
            continue;
        };
        let Some((mut menu, mut node)) = menus.get_entity(trigger.menu) else {
            continue;
        };
        menu.open = !menu.open;
        node.visible = menu.open;
    }
}
