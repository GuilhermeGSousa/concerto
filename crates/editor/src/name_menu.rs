use concerto_ecs::{
    Component, Entity, Query, Res, With, command::CommandQueue,
    query::change_detection::DetectChanges, signal::listener::Listener,
};
use concerto_ui::elements::ListItem;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    items::UIItemActivated,
    theme::UITheme,
};

use glam::Vec2;

use crate::asset_editor::ActiveEditor;
use crate::picker_menu::{picker_list, picker_panel, show_picker_menu};

/// The popup listing names to pick from, shared by every name picker.
#[derive(Component)]
pub struct NameMenu;

/// Opens the name menu under `button` listing `names` under a search field, or closes it when it is already open there.
pub fn open_name_menu(
    button: Entity,
    names: &[impl AsRef<str>],
    current: Option<&str>,
    pick: impl Fn(&str) -> Listener<UIItemActivated>,
    menus: &mut Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let mut menu = None;
    if let Some((entity, mut panel)) = menus.iter_mut().next() {
        if panel.open && panel.owner == Some(button) {
            panel.open = false;
            return;
        }
        menu = Some(entity);
    }
    let list = picker_list(theme)
        .searchable("Search…")
        .empty_text("None")
        .items(items(names, current, pick, theme));
    show_picker_menu(
        menu,
        NameMenu,
        list,
        |field| picker_panel(button, field),
        theme,
        cmd,
    );
}

/// Opens the name menu at `position`, a point in the window, listing `names`; a press outside it closes it.
pub fn open_name_menu_at(
    position: Vec2,
    names: &[impl AsRef<str>],
    pick: impl Fn(&str) -> Listener<UIItemActivated>,
    menus: &mut Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let panel = UIAnchoredPanel::new(UIAnchorTarget::from_point(position))
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::Start)
        .with_open(true);
    let list = picker_list(theme)
        .empty_text("None")
        .items(items(names, None, pick, theme));
    let menu = menus.iter_mut().next().map(|(entity, _)| entity);
    show_picker_menu(menu, NameMenu, list, |_| panel, theme, cmd);
}

fn items<'a>(
    names: &'a [impl AsRef<str>],
    current: Option<&'a str>,
    pick: impl Fn(&str) -> Listener<UIItemActivated> + 'a,
    theme: &'a UITheme,
) -> impl Iterator<Item = ListItem> + 'a {
    names.iter().map(move |name| {
        let name = name.as_ref();
        theme
            .list_item(name)
            .selected(current == Some(name))
            .on_activate(pick(name))
    })
}

/// Closes the name menu.
pub fn close_menu(menus: &mut Query<&mut UIAnchoredPanel, With<NameMenu>>) {
    for mut panel in menus.iter_mut() {
        panel.open = false;
    }
}

/// Closes the name menu when another tab comes to the front.
pub fn close_name_menu(
    active: Res<ActiveEditor>,
    mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
) {
    if !active.is_changed() {
        return;
    }
    for mut panel in menus.iter_mut() {
        if panel.open {
            panel.open = false;
        }
    }
}
