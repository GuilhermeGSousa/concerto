//! The "add component" row and the anchored menu it opens.
use super::*;

use super::registry::InspectionSource;
use concerto_color::Color;
use concerto_ecs::component::Tick;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, toggle_owned_panels},
    text_input::UITextInput,
};

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
        .spawn_child_queue(
            theme
                .pressable()
                .bordered()
                .radius_md()
                .height(UIValue::Px(ROW_HEIGHT))
                .row()
                .gap(7.0)
                .padding(UIRect::axes(0.0, 9.0))
                .on_click(toggle_owned_panels),
        )
        .add_child(theme.label("⌕").muted().font_size(12.0).no_wrap())
        .add_child(
            theme
                .label("add component")
                .muted()
                .font_size(12.0)
                .no_wrap()
                .grow(),
        )
        .add_child(theme.label("⌘⇧A").muted().mono().font_size(10.0).no_wrap())
        .entity()
}

fn spawn_menu(cmd: &mut CommandQueue, row: Entity, theme: &UITheme) {
    let mut menu = cmd.spawn(theme.popup().fill(MENU_FILL).width(UIValue::Px(MENU_WIDTH)));

    let mut header = menu
        .spawn_child_queue(
            theme
                .row()
                .fixed()
                .gap(7.0)
                .padding(UIRect::axes(9.0, 10.0)),
        )
        .add_child(theme.label("⌕").muted().font_size(12.5).no_wrap());
    let search = header
        .spawn_child_queue((
            theme
                .text_field("Search components…")
                .bare()
                .height(UIValue::Auto)
                .padding(0.0)
                .font_size(12.5)
                .single_line()
                .grow()
                .shrink(1.0)
                .min_width(UIValue::Px(0.0)),
            AddComponentSearch,
        ))
        .entity();
    let count = header
        .spawn_child_queue(
            theme
                .label("")
                .muted()
                .mono()
                .font_size(9.5)
                .no_wrap()
                .fixed(),
        )
        .entity();

    menu = menu.add_child(theme.divider());
    let list = menu
        .spawn_child_queue(
            theme
                .column()
                .gap(1.0)
                .max_height(UIValue::Px(MENU_LIST_MAX_HEIGHT))
                .padding(6.0)
                .clipped(),
        )
        .entity();
    menu = menu.add_child(theme.divider());
    menu = menu.add_child_with(
        theme
            .row()
            .fixed()
            .gap(10.0)
            .padding(UIRect::axes(7.0, 10.0)),
        |footer| {
            footer
                .add_child(
                    theme
                        .label("↑↓ move")
                        .muted()
                        .mono()
                        .font_size(9.5)
                        .no_wrap(),
                )
                .add_child(theme.label("↵ add").muted().mono().font_size(9.5).no_wrap())
                .add_child(UINode::default().with_flex_grow(1.0))
                .add_child(
                    theme
                        .label("⇧↵ add & open")
                        .muted()
                        .mono()
                        .font_size(9.5)
                        .no_wrap(),
                );
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
        cmd.insert(
            theme.text(&count).muted().mono().font_size(9.5).no_wrap(),
            menu.count,
        );

        if addable.is_empty() {
            let message = if contents.query.is_empty() {
                "no components to add".to_string()
            } else {
                format!("nothing matches “{}”", contents.query)
            };
            cmd.entity(menu.list).add_child(
                theme
                    .label(&message)
                    .muted()
                    .font_size(12.0)
                    .no_wrap()
                    .padding(UIRect::axes(6.0, 8.0)),
            );
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
    let mut list = cmd.entity(list);
    let row = list
        .spawn_child_queue(
            theme
                .pressable()
                .selected(highlighted)
                .row()
                .gap(8.0)
                .padding(UIRect::axes(6.0, 8.0)),
        )
        .add_child(
            theme
                .canvas()
                .fill(theme.accent)
                .rotation(std::f32::consts::FRAC_PI_4)
                .size(UIValue::Px(MARK_SIZE), UIValue::Px(MARK_SIZE))
                .fixed(),
        )
        .add_child(
            theme
                .label(name)
                .font_size(12.5)
                .single_line()
                .color(if highlighted {
                    theme.accent_hovered
                } else {
                    theme.text
                })
                .grow()
                .min_width(UIValue::Px(0.0)),
        );
    if highlighted {
        row.add_child(theme.label("↵").muted().mono().font_size(9.5).no_wrap());
    }
}
