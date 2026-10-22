//! Small themed widget building blocks. These components describe interaction
//! without introducing editor concepts into the UI crate.
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::Entity,
    query::Query,
    resource::Res,
    signal::{EntitySignal, On, Signal},
};
use concerto_window::input::MouseButton;

use crate::{
    interaction::{HoveredNode, UIClick},
    node::UINode,
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
#[derive(Component)]
pub struct UITabBody {
    pub strip: Entity,
    pub index: usize,
}

#[derive(Component, Default)]
pub struct UIPropertyRow;

/// Sent to a [`UICollapsibleSection`] when it expands or collapses.
pub struct UICollapsibleChanged {
    pub expanded: bool,
}

impl Signal for UICollapsibleChanged {}
impl EntitySignal for UICollapsibleChanged {}

/// Sent to a [`UITabStrip`] when one of its tabs is selected.
pub struct UITabChanged {
    pub selected: usize,
}

impl Signal for UITabChanged {}
impl EntitySignal for UITabChanged {}

/// Click listener that expands or collapses the [`UICollapsibleSection`] it sits on.
pub fn toggle_collapsible(
    on: On<UIClick>,
    collapsibles: Query<&mut UICollapsibleSection>,
    nodes: Query<&mut UINode>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(mut section) = collapsibles.get_entity(on.entity()) else {
        return;
    };
    section.expanded = !section.expanded;
    if let Some(mut node) = nodes.get_entity(section.content) {
        node.visible = section.expanded;
    }
    cmd.entity(on.entity()).trigger(UICollapsibleChanged {
        expanded: section.expanded,
    });
}

/// Click listener that selects the [`UITab`] it sits on in its strip.
pub fn select_tab(
    on: On<UIClick>,
    tabs: Query<&UITab>,
    strips: Query<&mut UITabStrip>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(tab) = tabs.get_entity(on.entity()) else {
        return;
    };
    let Some(mut strip) = strips.get_entity(tab.strip) else {
        return;
    };
    strip.selected = tab.index;
    cmd.entity(tab.strip).trigger(UITabChanged {
        selected: tab.index,
    });
}

/// Shows the body whose index matches its strip's selection, and hides the rest.
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
