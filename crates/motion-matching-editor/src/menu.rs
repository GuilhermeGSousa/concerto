use concerto_ecs::{
    Component, Entity, Query, With,
    command::CommandQueue,
    signal::{On, lifecycle::Remove, listener::Listener},
};
use concerto_editor::hosted::EditTarget;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    interaction::UIClick,
    node::UIRect,
    scroll::{SCROLLBAR_WIDTH, UIScrollArea},
    theme::UITheme,
    transform::UIValue,
};

const MENU_WIDTH: f32 = 220.0;
const ITEM_HEIGHT: f32 = 26.0;
const LIST_MAX_HEIGHT: f32 = 280.0;
const SCROLLBAR_GUTTER: f32 = SCROLLBAR_WIDTH + 2.0;

#[derive(Component)]
pub(crate) struct NameMenu;

fn menu_panel(button: Entity) -> UIAnchoredPanel {
    UIAnchoredPanel::new(UIAnchorTarget::from_node(button))
        .with_owner(button)
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::End)
        .with_gap(4.0)
        .with_open(true)
}

pub(crate) fn open_name_menu(
    button: Entity,
    names: &[String],
    current: Option<&str>,
    pick: impl Fn(&str) -> Listener<UIClick>,
    menus: &Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let mut menu = match menus.iter().next() {
        Some((entity, mut panel)) => {
            if panel.open && panel.owner == Some(button) {
                panel.open = false;
                return;
            }
            **panel = menu_panel(button);
            cmd.entity(entity).despawn_children()
        }
        None => cmd.spawn((
            theme
                .popup()
                .width(UIValue::Px(MENU_WIDTH))
                .padding(theme.spacing_xs),
            menu_panel(button),
            NameMenu,
        )),
    };
    if names.is_empty() {
        menu.add_child(
            theme
                .label("None")
                .muted()
                .font_size(theme.font_size_sm)
                .no_wrap()
                .padding(UIRect::axes(6.0, 8.0)),
        );
        return;
    }
    let extent = names.len() as f32 * ITEM_HEIGHT;
    menu.add_child_with(
        theme
            .column()
            .fixed()
            .height(UIValue::Px(extent.min(LIST_MAX_HEIGHT)))
            .clipped(),
        |mut view| {
            let mut list = view.spawn_child(theme.column().gap(0.0).padding(UIRect {
                right: SCROLLBAR_GUTTER,
                ..UIRect::all(0.0)
            }));
            for name in names {
                list.add_child_with(
                    theme
                        .pressable()
                        .selected(current == Some(name.as_str()))
                        .row()
                        .fixed()
                        .height(UIValue::Px(ITEM_HEIGHT))
                        .padding(UIRect::axes(0.0, 8.0))
                        .on_click(pick(name)),
                    |mut item| {
                        item.add_child(
                            theme
                                .label(name.as_str())
                                .font_size(theme.font_size_sm)
                                .single_line()
                                .grow()
                                .shrink(1.0)
                                .min_width(UIValue::Px(0.0)),
                        );
                    },
                );
            }
            let list = list.entity();
            view.insert(UIScrollArea {
                content: Some(list),
                content_extent: extent,
                ..Default::default()
            });
        },
    );
}

pub(crate) fn close_menu(menus: &Query<&mut UIAnchoredPanel, With<NameMenu>>) {
    for mut panel in menus.iter() {
        panel.open = false;
    }
}

pub(crate) fn close_name_menu(
    _: On<Remove<EditTarget>>,
    menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
) {
    for mut panel in menus.iter() {
        if panel.open {
            panel.open = false;
        }
    }
}
