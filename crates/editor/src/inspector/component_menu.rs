//! The "⋯" button on a component card and the menu it opens.
use super::*;

use crate::fonts::{glyph, icon};
use concerto_ecs::component::bundle::IntoBundle;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorTarget, UIAnchoredPanel},
    interaction::UIClick,
};
use concerto_window::input::MouseButton;

const MENU_WIDTH: f32 = 168.0;

/// The persistent menu panel, pointed at whichever card's button was clicked last.
#[derive(Component)]
pub(super) struct ComponentMenu {
    scene_target: Option<ComponentMenuButton>,
}

/// A card's "⋯" button and the component it acts on.
#[derive(Component, Clone, Copy)]
pub(super) struct ComponentMenuButton {
    pub scene_entity: Entity,
    pub component: TypeId,
}

#[derive(Component)]
pub(super) struct RemoveComponentItem;

pub(super) fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<(&mut ComponentMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter() {
        if menu.scene_target.is_some() {
            menu.scene_target = None;
        }
        if panel.open {
            panel.open = false;
        }
    }
}

pub(super) fn menu_button(
    theme: &UITheme,
    scene_entity: Entity,
    component: TypeId,
) -> impl IntoBundle<Bundle: 'static> + use<> {
    (
        theme
            .pressable()
            .size(UIValue::Px(22.0), UIValue::Px(20.0))
            .padding(UIRect::axes(0.0, 4.0))
            .on_click(toggle_component_menu),
        icon(theme, glyph::DOTS_THREE, theme.font_size_lg).muted(),
        ComponentMenuButton {
            scene_entity,
            component,
        },
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
            .width(UIValue::Px(MENU_WIDTH))
            .padding(theme.spacing_xs)
            .anchor_align(UIAnchorAlign::End),
        ComponentMenu { scene_target: None },
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
        |mut item| {
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
        menu.scene_target = Some(*button);
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
        if let Some(target) = menu.scene_target.filter(|_| panel.open) {
            edits.0.push(ComponentEdit::Remove {
                entity: target.scene_entity,
                component: target.component,
            });
        }
        panel.open = false;
    }
}
