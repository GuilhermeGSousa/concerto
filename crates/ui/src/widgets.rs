//! Small themed widget building blocks. These components describe interaction
//! without introducing editor concepts into the UI crate.
use concerto_ecs::{
    command::CommandQueue,
    component::Component,
    entity::Entity,
    query::Query,
    resource::Res,
    signal::{
        EntitySignal, On, Signal,
        listener::{IntoListener, Listener},
    },
};
use concerto_window::input::MouseButton;

use crate::{
    interaction::{HoveredNode, UIClick},
    node::UINode,
};

/// Marks a button, as the themed button element spawns it.
#[derive(Component, Default)]
pub struct UIButton;

/// Marks a button that shows an icon instead of a label.
#[derive(Component, Default)]
pub struct UIIconButton;

/// Marks a search field; nothing reads it, the behaviour is [`crate::search::UISearchField`]'s.
#[derive(Component, Default)]
pub struct UISearchField;

/// A node shown only while the pointer is over `target`.
#[derive(Component)]
pub struct UITooltip {
    /// The node whose hover shows the tooltip.
    pub target: Entity,
}

/// A header that shows and hides `content` when clicked, with [`toggle_collapsible`] as its click
/// listener.
#[derive(Component)]
pub struct UICollapsibleSection {
    /// Whether `content` is shown.
    pub expanded: bool,
    /// The node shown and hidden.
    pub content: Entity,
}

/// A row of tabs, each with [`select_tab`] as its click listener, showing the [`UITabBody`] of the
/// selected one.
#[derive(Component, Default)]
pub struct UITabStrip {
    /// The index of the selected tab.
    pub selected: usize,
}

/// The content shown when its tab is the selected one.
#[derive(Component)]
pub struct UITabBody {
    /// The [`UITabStrip`] entity.
    pub strip: Entity,
    /// The index of the tab that shows this body.
    pub index: usize,
}

/// Marks a row of a property list.
#[derive(Component, Default)]
pub struct UIPropertyRow;

/// Sent to a [`UICollapsibleSection`] when it expands or collapses.
pub struct UICollapsibleChanged {
    /// Whether the section is now expanded.
    pub expanded: bool,
}

impl Signal for UICollapsibleChanged {}
impl EntitySignal for UICollapsibleChanged {}

/// Sent to a [`UITabStrip`] when one of its tabs is selected.
pub struct UITabChanged {
    /// The index of the selected tab.
    pub selected: usize,
}

impl Signal for UITabChanged {}
impl EntitySignal for UITabChanged {}

/// Click listener that expands or collapses the [`UICollapsibleSection`] it sits on.
pub fn toggle_collapsible(
    on: On<UIClick>,
    mut collapsibles: Query<&mut UICollapsibleSection>,
    mut nodes: Query<&mut UINode>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(mut section) = collapsibles.get_entity_mut(on.entity()) else {
        return;
    };
    section.expanded = !section.expanded;
    if let Some(mut node) = nodes.get_entity_mut(section.content) {
        node.visible = section.expanded;
    }
    cmd.entity(on.entity()).trigger(UICollapsibleChanged {
        expanded: section.expanded,
    });
}

/// Click listener for tab `index` of `strip`: selects it there and tells the strip.
pub fn select_tab(strip: Entity, index: usize) -> Listener<UIClick> {
    (move |on: On<UIClick>, mut strips: Query<&mut UITabStrip>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(mut tabs) = strips.get_entity_mut(strip) else {
            return;
        };
        tabs.selected = index;
        cmd.entity(strip).trigger(UITabChanged { selected: index });
    })
    .into_listener()
}

/// Shows the body whose index matches its strip's selection, and hides the rest.
pub fn sync_tab_bodies(strips: Query<&UITabStrip>, mut bodies: Query<(&UITabBody, &mut UINode)>) {
    for (body, mut node) in bodies.iter_mut() {
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
    mut tooltips: Query<(&UITooltip, &mut UINode)>,
) {
    for (tooltip, mut node) in tooltips.iter_mut() {
        let visible = **hovered == Some(tooltip.target);
        if node.visible != visible {
            node.visible = visible;
        }
    }
}
