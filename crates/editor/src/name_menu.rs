use concerto_ecs::{
    Component, Entity, Query, Res, With, command::CommandQueue,
    query::change_detection::DetectChanges, signal::listener::Listener,
};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    items::UIItemActivated,
    theme::UITheme,
};

use glam::Vec2;

use crate::asset_editor::ActiveEditor;
use crate::picker_menu::{PickerEntry, picker_panel, show_picker_menu, spawn_picker_content};

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
    let content = spawn_picker_content(
        entries(names, current, pick),
        Some("Search…"),
        "None",
        theme,
        cmd,
    );
    show_picker_menu(
        menu,
        picker_panel(button, content.field),
        NameMenu,
        content,
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
    let content = spawn_picker_content(entries(names, None, pick), None, "None", theme, cmd);
    let menu = menus.iter_mut().next().map(|(entity, _)| entity);
    show_picker_menu(menu, panel, NameMenu, content, theme, cmd);
}

fn entries(
    names: &[impl AsRef<str>],
    current: Option<&str>,
    pick: impl Fn(&str) -> Listener<UIItemActivated>,
) -> Vec<PickerEntry> {
    names
        .iter()
        .map(|name| {
            let name = name.as_ref();
            PickerEntry::new(name, pick(name)).with_current(current == Some(name))
        })
        .collect()
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
