//! The "add component" row and the anchored menu it opens.
use super::*;

use super::registry::InspectionSource;
use concerto_ecs::{
    component::Tick, events::event_reader::EventReader, query::filter::With, signal::On,
};
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel, toggle_owned_panels},
    interaction::UIClick,
    text_input::{UITextInput, UITextInputSubmitted},
};
use concerto_window::input::MouseButton;

const MENU_WIDTH: f32 = 272.0;
const MENU_LIST_MAX_HEIGHT: f32 = 320.0;
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

/// A row of the menu's list and the component it adds.
#[derive(Component, Clone, Copy)]
pub(super) struct AddComponentEntry {
    scene_entity: Entity,
    component: TypeId,
}

/// Marks the entry that submitting the search adds.
#[derive(Component)]
pub(super) struct HighlightedEntry;

/// What the menu's list was last built from.
#[derive(Component, PartialEq)]
pub(super) struct AddComponentMenuContents {
    scene_target: Option<Entity>,
    structural_version: Option<EntityStructuralVersion>,
    registry_tick: Tick,
    query: String,
}

pub(super) fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<(Entity, &AddComponentMenu, &mut UIAnchoredPanel)>,
    mut cmd: CommandQueue,
) {
    for (entity, menu, mut panel) in menus.iter() {
        if panel.open {
            panel.open = false;
        }
        cmd.entity(menu.list).despawn_children();
        cmd.remove::<AddComponentMenuContents>(entity);
    }
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
        .spawn_child(
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
    let mut menu = cmd.spawn(theme.popup().width(UIValue::Px(MENU_WIDTH)));

    let mut header = menu.spawn_child(
        theme
            .row()
            .fixed()
            .gap(7.0)
            .padding(UIRect::axes(9.0, 10.0)),
    );
    header.add_child(theme.label("⌕").muted().font_size(12.5).no_wrap());
    let search = header
        .spawn_child((
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
        .spawn_child(
            theme
                .label("")
                .muted()
                .mono()
                .font_size(9.5)
                .no_wrap()
                .fixed(),
        )
        .entity();

    menu.add_child(theme.divider());
    let list = menu
        .spawn_child(
            theme
                .column()
                .gap(1.0)
                .max_height(UIValue::Px(MENU_LIST_MAX_HEIGHT))
                .padding(6.0)
                .clipped(),
        )
        .entity();
    menu.add_child(theme.divider());
    menu.add_child_with(
        theme
            .row()
            .fixed()
            .gap(10.0)
            .padding(UIRect::axes(7.0, 10.0)),
        |mut footer| {
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
    source: EditWorld<InspectionSource>,
    registry: Res<InspectorRegistry>,
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
    let target = data
        .scene_entity
        .filter(|&entity| source.entity_is_valid(entity));
    for (entity, menu, panel, shown) in menus.iter() {
        if !panel.open {
            continue;
        }
        let contents = AddComponentMenuContents {
            scene_target: target,
            structural_version: target.and_then(|entity| source.structural_version(entity)),
            registry_tick: registry.changed_tick(),
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
        addable.retain(|(_, name)| name.to_lowercase().contains(&contents.query));
        addable.sort_by_key(|(_, a)| *a);

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

        for (index, (type_id, name)) in addable.iter().enumerate() {
            let Some(scene_entity) = target else {
                break;
            };
            let entry = AddComponentEntry {
                scene_entity,
                component: *type_id,
            };
            spawn_entry(&mut cmd, menu.list, entry, name, index == 0, &theme);
        }

        cmd.insert(contents, entity);
    }
}

fn spawn_entry(
    cmd: &mut CommandQueue,
    list: Entity,
    entry: AddComponentEntry,
    name: &str,
    highlighted: bool,
    theme: &UITheme,
) {
    let mut list = cmd.entity(list);
    let mut row = list.spawn_child((
        theme
            .pressable()
            .selected(highlighted)
            .row()
            .gap(8.0)
            .padding(UIRect::axes(6.0, 8.0))
            .on_click(add_clicked_component),
        entry,
    ));
    row.add_child(
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
        row.insert(HighlightedEntry);
        row.add_child(theme.label("↵").muted().mono().font_size(9.5).no_wrap());
    }
}

fn add_clicked_component(
    on: On<UIClick>,
    entries: Query<&AddComponentEntry>,
    menus: Query<&mut UIAnchoredPanel, With<AddComponentMenu>>,
    mut edits: ResMut<ComponentEdits>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    if let Some(entry) = entries.get_entity(on.entity()) {
        queue_add(*entry, &menus, &mut edits);
    }
}

/// Adds the highlighted entry when the search field is submitted.
pub(super) fn add_highlighted_component(
    mut submitted: EventReader<UITextInputSubmitted>,
    searches: Query<&AddComponentSearch>,
    entries: Query<&AddComponentEntry, With<HighlightedEntry>>,
    menus: Query<&mut UIAnchoredPanel, With<AddComponentMenu>>,
    mut edits: ResMut<ComponentEdits>,
) {
    for event in submitted.read() {
        if searches.get_entity(event.entity).is_none() {
            continue;
        }
        if let Some(entry) = entries.iter().next() {
            queue_add(*entry, &menus, &mut edits);
        }
    }
}

fn queue_add(
    entry: AddComponentEntry,
    menus: &Query<&mut UIAnchoredPanel, With<AddComponentMenu>>,
    edits: &mut ComponentEdits,
) {
    let mut open = false;
    for mut panel in menus.iter() {
        open |= panel.open;
        panel.open = false;
    }
    if open {
        edits.0.push(ComponentEdit::Add {
            entity: entry.scene_entity,
            component: entry.component,
        });
    }
}
