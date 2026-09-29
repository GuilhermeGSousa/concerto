//! The "add component" row and the anchored menu it opens.
use super::*;

use super::registry::InspectionSource;
use concerto_color::Color;
use concerto_ecs::component::Tick;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    focus::UIFocusable,
    interaction::UIInteractionStyle,
    text::FontFamily,
    text_input::UITextInput,
};

use crate::marks::{TRANSPARENT, selection_tint};

const MENU_WIDTH: f32 = 272.0;
const MENU_LIST_MAX_HEIGHT: f32 = 320.0;
const MENU_FILL: Color = Color::srgba(0.090, 0.102, 0.157, 0.96);
const ROW_HEIGHT: f32 = 30.0;
const MARK_SIZE: f32 = 7.0;

/// The persistent menu panel, re-anchored to each freshly built "add component" row.
#[derive(Component)]
pub(super) struct AddComponentMenu {
    search: Entity,
    count: Entity,
    list: Entity,
}

/// Marks the menu's search field, which outlives the inspector's rebuilds.
#[derive(Component)]
pub(super) struct AddComponentSearch;

/// What the menu's list was last built from.
#[derive(Component, PartialEq)]
pub(super) struct AddComponentMenuContents {
    target: Option<Entity>,
    structural_version: Option<EntityStructuralVersion>,
    registry_tick: Tick,
    query: String,
}

/// Spawns the "add component" row at the end of `stack` and anchors the menu to it.
pub(super) fn spawn_add_component(
    cmd: &mut CommandQueue,
    stack: Entity,
    menus: &Query<(Entity, &AddComponentMenu)>,
    theme: &UITheme,
) {
    let row = spawn_row(cmd, stack, theme);
    match menus.iter().next() {
        Some((entity, menu)) => cmd.insert(menu_panel(row, menu.search), entity),
        None => spawn_menu(cmd, row, theme),
    }
}

fn menu_panel(row: Entity, search: Entity) -> UIAnchoredPanel {
    UIAnchoredPanel::new(UIAnchorTarget::from_node(row))
        .with_owner(row)
        .with_side(UIAnchorSide::Above)
        .with_align(UIAnchorAlign::End)
        .with_gap(6.0)
        .toggled_by_owner()
        .with_focus_on_open(search)
}

fn spawn_row(cmd: &mut CommandQueue, stack: Entity, theme: &UITheme) -> Entity {
    cmd.entity(stack)
        .spawn_child_queue((
            UINode::default()
                .with_height(UIValue::Px(ROW_HEIGHT))
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .with_align_items(taffy::AlignItems::Center)
                .with_gap(glam::Vec2::new(7.0, 0.0))
                .with_padding(UIRect::axes(0.0, 9.0)),
            UIMaterial {
                corner_radius: theme.radius_md,
                ..UIMaterial::with_border(TRANSPARENT, theme.border, 1.0)
            },
            UIInteractionStyle {
                normal: TRANSPARENT,
                hovered: theme.surface_hovered,
                pressed: selection_tint(theme),
                disabled: TRANSPARENT,
            },
            Interactable,
        ))
        .add_child((UINode::default(), muted(theme, "⌕", 12.0)))
        .add_child((
            UINode::default().with_flex_grow(1.0),
            muted(theme, "add component", 12.0),
        ))
        .add_child((UINode::default(), mono(theme, "⌘⇧A", 10.0)))
        .entity()
}

fn spawn_menu(cmd: &mut CommandQueue, row: Entity, theme: &UITheme) {
    let mut menu = cmd.spawn((
        UINode::default()
            .with_width(UIValue::Px(MENU_WIDTH))
            .with_flex_direction(FlexDirection::Column)
            .clipped(),
        UIMaterial {
            corner_radius: theme.radius_md,
            ..UIMaterial::with_border(MENU_FILL, theme.border, 1.0)
        },
    ));

    let mut header = menu
        .spawn_child_queue(
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .with_align_items(taffy::AlignItems::Center)
                .with_gap(glam::Vec2::new(7.0, 0.0))
                .with_padding(UIRect::axes(9.0, 10.0)),
        )
        .add_child((UINode::default(), muted(theme, "⌕", 12.5)));
    let search = header
        .spawn_child_queue((
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_width(UIValue::Px(0.0)),
            UIText {
                wrap: false,
                ellipsis: true,
                ..sized(theme, "", 12.5)
            },
            UITextInput::new("Search components…"),
            Interactable,
            UIFocusable,
            AddComponentSearch,
        ))
        .entity();
    let count = header
        .spawn_child_queue((
            UINode::default().with_flex_shrink(0.0),
            mono(theme, "", 9.5),
        ))
        .entity();

    menu = menu.add_child(divider(theme));
    let list = menu
        .spawn_child_queue(
            UINode::default()
                .with_flex_direction(FlexDirection::Column)
                .with_max_height(UIValue::Px(MENU_LIST_MAX_HEIGHT))
                .with_gap(glam::Vec2::new(0.0, 1.0))
                .with_padding(UIRect::all(6.0))
                .clipped(),
        )
        .entity();
    menu = menu.add_child(divider(theme));
    menu = menu.add_child_with(
        UINode::default()
            .with_flex_shrink(0.0)
            .with_flex_direction(FlexDirection::Row)
            .with_gap(glam::Vec2::new(10.0, 0.0))
            .with_padding(UIRect::axes(7.0, 10.0)),
        |footer| {
            footer
                .add_child((UINode::default(), mono(theme, "↑↓ move", 9.5)))
                .add_child((UINode::default(), mono(theme, "↵ add", 9.5)))
                .add_child(UINode::default().with_flex_grow(1.0))
                .add_child((UINode::default(), mono(theme, "⇧↵ add & open", 9.5)));
        },
    );

    menu.insert((
        menu_panel(row, search),
        AddComponentMenu {
            search,
            count,
            list,
        },
    ));
}

/// Clears the search field while the menu is closed, so it reopens unfiltered.
pub(super) fn clear_add_component_search(
    menus: Query<(&AddComponentMenu, &UIAnchoredPanel)>,
    inputs: Query<&mut UITextInput>,
) {
    for (menu, panel) in menus.iter() {
        if !panel.open
            && let Some(mut input) = inputs.get_entity(menu.search)
            && !input.value.is_empty()
        {
            input.value.clear();
            input.cursor = 0;
            input.selection_anchor = None;
        }
    }
}

/// Rebuilds the open menu's list when the target, the registry or the search text changes.
pub(super) fn populate_add_component_menu(
    source: InspectionSource,
    data: Res<InspectorData>,
    theme: Res<UITheme>,
    menus: Query<(
        Entity,
        &AddComponentMenu,
        &UIAnchoredPanel,
        Option<&AddComponentMenuContents>,
    )>,
    inputs: Query<&UITextInput>,
    mut cmd: CommandQueue,
) {
    let target = data.entity.filter(|&entity| source.entity_is_valid(entity));
    for (entity, menu, panel, shown) in menus.iter() {
        if !panel.open {
            continue;
        }
        let contents = AddComponentMenuContents {
            target,
            structural_version: target.and_then(|entity| source.structural_version(entity)),
            registry_tick: source.registry_tick(),
            query: inputs
                .get_entity(menu.search)
                .map(|input| input.value.trim().to_lowercase())
                .unwrap_or_default(),
        };
        if shown == Some(&contents) {
            continue;
        }

        cmd.entity(menu.list).despawn_children();
        let (mut addable, total) = match target {
            Some(target) => source.addable_components(target),
            None => (Vec::new(), 0),
        };
        addable.retain(|(_, component)| component.name.to_lowercase().contains(&contents.query));
        addable.sort_by(|(_, a), (_, b)| a.name.cmp(b.name));

        let count = if contents.query.is_empty() {
            String::new()
        } else {
            format!("{} of {total}", addable.len())
        };
        cmd.insert(mono(&theme, &count, 9.5), menu.count);

        if addable.is_empty() {
            let message = if contents.query.is_empty() {
                "no components to add".to_string()
            } else {
                format!("nothing matches “{}”", contents.query)
            };
            cmd.entity(menu.list).add_child((
                UINode::default().with_padding(UIRect::axes(6.0, 8.0)),
                muted(&theme, &message, 12.0),
            ));
        }

        for (index, (_, component)) in addable.iter().enumerate() {
            spawn_entry(&mut cmd, menu.list, component.name, index == 0, &theme);
        }

        cmd.insert(contents, entity);
    }
}

fn spawn_entry(
    cmd: &mut CommandQueue,
    list: Entity,
    name: &str,
    highlighted: bool,
    theme: &UITheme,
) {
    let normal = if highlighted {
        selection_tint(theme)
    } else {
        TRANSPARENT
    };

    let mut list = cmd.entity(list);
    let row = list
        .spawn_child_queue((
            UINode::default()
                .with_flex_shrink(0.0)
                .with_flex_direction(FlexDirection::Row)
                .with_align_items(taffy::AlignItems::Center)
                .with_gap(glam::Vec2::new(8.0, 0.0))
                .with_padding(UIRect::axes(6.0, 8.0)),
            UIMaterial {
                corner_radius: theme.radius_sm,
                ..UIMaterial::flat(normal)
            },
            UIInteractionStyle {
                normal,
                hovered: theme.surface_hovered,
                pressed: selection_tint(theme),
                disabled: TRANSPARENT,
            },
            Interactable,
        ))
        .add_child((
            UINode::default()
                .with_size(UIValue::Px(MARK_SIZE), UIValue::Px(MARK_SIZE))
                .with_flex_shrink(0.0),
            UIMaterial {
                rotation: std::f32::consts::FRAC_PI_4,
                ..UIMaterial::flat(theme.accent)
            },
        ))
        .add_child((
            UINode::default()
                .with_flex_grow(1.0)
                .with_min_width(UIValue::Px(0.0)),
            UIText {
                wrap: false,
                ellipsis: true,
                color: if highlighted {
                    theme.accent_hovered
                } else {
                    theme.text
                },
                ..sized(theme, name, 12.5)
            },
        ));
    if highlighted {
        row.add_child((UINode::default(), mono(theme, "↵", 9.5)));
    }
}

fn divider(theme: &UITheme) -> (UINode, UIMaterial) {
    (
        UINode::default()
            .with_height(UIValue::Px(1.0))
            .with_flex_shrink(0.0),
        UIMaterial::flat(theme.border),
    )
}

fn sized(theme: &UITheme, value: &str, size: f32) -> UIText {
    UIText {
        font_size: size,
        line_height: theme.line_height(size),
        wrap: false,
        ..text(theme, value)
    }
}

fn muted(theme: &UITheme, value: &str, size: f32) -> UIText {
    UIText {
        color: theme.text_muted,
        ..sized(theme, value, size)
    }
}

fn mono(theme: &UITheme, value: &str, size: f32) -> UIText {
    UIText {
        font_family: FontFamily::Monospace,
        ..muted(theme, value, size)
    }
}
