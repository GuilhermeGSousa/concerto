//! The "⋯" button on a component card and the menu it opens.
use super::*;

use super::add_component::MENU_FILL;

use crate::fonts::{glyph, icon};
use concerto_ecs::{component::bundle::IntoBundle, signal::On};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorTarget, UIAnchoredPanel},
    interaction::UIClick,
};
use concerto_window::input::MouseButton;

const MENU_WIDTH: f32 = 168.0;

/// The persistent menu panel, pointed at whichever card's button was clicked last.
#[derive(Component)]
pub(super) struct ComponentMenu {
    target: Option<ComponentMenuButton>,
}

/// A card's "⋯" button and the component it acts on.
#[derive(Component, Clone, Copy)]
pub(super) struct ComponentMenuButton {
    pub entity: Entity,
    pub component: TypeId,
}

#[derive(Component)]
pub(super) struct RemoveComponentItem;

/// The "⋯" button for the card showing `component` of `entity`.
pub(super) fn menu_button(
    theme: &UITheme,
    entity: Entity,
    component: TypeId,
) -> impl IntoBundle<Bundle: 'static> + use<> {
    (
        theme
            .pressable()
            .size(UIValue::Px(22.0), UIValue::Px(20.0))
            .padding(UIRect::axes(0.0, 4.0))
            .on_click(toggle_component_menu),
        icon(theme, glyph::DOTS_THREE, theme.font_size_lg).muted(),
        ComponentMenuButton { entity, component },
    )
}

/// Spawns the shared menu unless one already exists.
pub(super) fn ensure_component_menu(
    cmd: &mut CommandQueue,
    menus: &Query<&ComponentMenu>,
    theme: &UITheme,
) {
    if menus.iter().next().is_some() {
        return;
    }
    cmd.spawn((
        theme
            .context_menu()
            .fill(MENU_FILL)
            .width(UIValue::Px(MENU_WIDTH))
            .padding(theme.spacing_xs)
            .anchor_align(UIAnchorAlign::End),
        ComponentMenu { target: None },
    ))
    .add_child_with(
        (
            theme
                .pressable()
                .row()
                .padding(UIRect::axes(6.0, 8.0))
                .on_click(remove_component),
            RemoveComponentItem,
        ),
        |item| {
            item.add_child(
                theme
                    .label("Remove component")
                    .font_size(12.5)
                    .color(theme.error)
                    .no_wrap(),
            );
        },
    );
}

fn toggle_component_menu(
    on: On<UIClick>,
    buttons: Query<&ComponentMenuButton>,
    menus: Query<(&mut ComponentMenu, &mut UIAnchoredPanel)>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(button) = buttons.get_entity(on.entity()) else {
        return;
    };
    for (mut menu, mut panel) in menus.iter() {
        let closing = panel.open && panel.owner == Some(on.entity());
        menu.target = Some(*button);
        panel.target = UIAnchorTarget::from_node(on.entity());
        panel.owner = Some(on.entity());
        panel.open = !closing;
    }
}

fn remove_component(
    on: On<UIClick>,
    menus: Query<(&ComponentMenu, &mut UIAnchoredPanel)>,
    mut edits: ResMut<ComponentEdits>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    for (menu, mut panel) in menus.iter() {
        if let Some(target) = menu.target.filter(|_| panel.open) {
            edits.0.push(ComponentEdit::Remove {
                entity: target.entity,
                component: target.component,
            });
        }
        panel.open = false;
    }
}
