//! The parts every picker menu is built from: its list or tree, its placement under the button that
//! opens it, and its sizes.
use concerto_ecs::{Entity, command::CommandQueue, component::Component};
use concerto_ui::elements::{ListBuilder, prelude::*};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    theme::UITheme,
    transform::UIValue,
};

/// The width of every picker menu.
pub const MENU_WIDTH: f32 = 240.0;
/// The height of a picker menu's row.
pub const ITEM_HEIGHT: f32 = 26.0;
/// The height a picker menu's list grows to before it scrolls.
pub const LIST_MAX_HEIGHT: f32 = 280.0;

/// A picker menu's list: rows of [`ITEM_HEIGHT`], at most [`LIST_MAX_HEIGHT`] tall, a click picking a row.
pub fn picker_list(theme: &UITheme) -> ListBuilder {
    menu_rows(theme.list())
}

/// A picker menu's tree, sized and picked like [`picker_list`].
pub fn picker_tree(theme: &UITheme) -> ListBuilder {
    menu_rows(theme.tree())
}

fn menu_rows(list: ListBuilder) -> ListBuilder {
    list.row_height(ITEM_HEIGHT)
        .click_activates()
        .max_height(UIValue::Px(LIST_MAX_HEIGHT))
}

/// A picker menu's placement below `button`, which toggles it, focusing `field` when it opens.
pub fn picker_panel(button: Entity, field: Option<Entity>) -> UIAnchoredPanel {
    let panel = UIAnchoredPanel::new(UIAnchorTarget::from_node(button))
        .with_owner(button)
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::End)
        .with_gap(4.0)
        .with_open(true);
    match field {
        Some(field) => panel.with_focus_on_open(field),
        None => panel,
    }
}

/// Fills the picker menu `menu`, or a new one marked by `marker`, with `list`, placed by `place` given the list's search field.
pub fn show_picker_menu<M: Component>(
    menu: Option<Entity>,
    marker: M,
    list: ListBuilder,
    place: impl FnOnce(Option<Entity>) -> UIAnchoredPanel,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) -> Entity {
    let menu = match menu {
        Some(menu) => {
            cmd.entity(menu).despawn_children();
            cmd.insert(marker, menu);
            menu
        }
        None => cmd
            .spawn((
                theme
                    .popup()
                    .width(UIValue::Px(MENU_WIDTH))
                    .padding(theme.spacing_xs)
                    .gap(theme.spacing_xs),
                marker,
            ))
            .entity(),
    };
    let spawned = list.spawn_in(menu, cmd);
    cmd.insert(place(spawned.field), menu);
    menu
}
