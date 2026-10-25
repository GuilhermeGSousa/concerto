use std::marker::PhantomData;

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    signal::{On, lifecycle::Remove},
};
use concerto_foundation::assets::{
    AssetId, LoadableAsset,
    asset_server::AssetServer,
    asset_store::AssetStore,
    handle::AssetHandle,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    interaction::UIClick,
    node::UIRect,
    scroll::UIScrollArea,
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::fonts::{glyph, icon};
use crate::hosted::EditTarget;
use crate::inspector::{
    EditContext, EditError, MENU_FILL, PropertyCommits, PropertyEditor, PropertyRow,
    PropertyRowValue,
};

const MENU_WIDTH: f32 = 240.0;
const ITEM_HEIGHT: f32 = 26.0;
const LIST_MAX_HEIGHT: f32 = 280.0;
const SCROLLBAR_GUTTER: f32 = 10.0;

/// Picks the asset an `AssetHandle<T>` names from the project's assets of type `T`.
pub struct AssetPicker<T: LoadableAsset>(PhantomData<fn() -> T>);

impl<T: LoadableAsset> Default for AssetPicker<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Keeps every asset picker row showing the asset it holds.
pub struct AssetPickerPlugin;

impl Plugin for AssetPickerPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(LateUpdate, refresh_asset_picker_labels)
            .add_listener(forget_scene);
    }
}

/// The button of an [`AssetPicker`] row, which opens the menu.
#[derive(Component)]
pub(crate) struct AssetPickerButton {
    row: Entity,
}

/// The text inside an [`AssetPickerButton`], naming the row's asset.
#[derive(Component)]
pub(crate) struct AssetPickerLabel {
    row: Entity,
}

/// The menu every picker shares, pointed at the row whose button opened it last.
#[derive(Component)]
pub(crate) struct AssetPickerMenu {
    row: Option<Entity>,
}

/// An item of the [`AssetPickerMenu`] and the asset it picks.
#[derive(Component)]
pub(crate) struct AssetPickerItem(pub AssetId);

impl<T: LoadableAsset> PropertyEditor<AssetHandle<T>> for AssetPicker<T> {
    type Snapshot = AssetId;
    type Edit = AssetId;

    fn snapshot(&self, value: &AssetHandle<T>) -> AssetId {
        value.id()
    }

    fn follows_snapshot(&self) -> bool {
        true
    }

    fn build(&self, row: &mut EntityCommandQueue, _: &AssetId, theme: &UITheme) {
        let entity = row.entity();
        row.add_child_with(
            (
                theme
                    .pressable()
                    .bordered()
                    .radius_sm()
                    .row()
                    .gap(6.0)
                    .height(UIValue::Px(theme.control_height))
                    .padding(UIRect::axes(0.0, 8.0))
                    .grow()
                    .shrink(1.0)
                    .min_width(UIValue::Px(0.0))
                    .on_click(toggle_asset_picker::<T>),
                AssetPickerButton { row: entity },
            ),
            |mut button| {
                button
                    .add_child((
                        theme
                            .label("")
                            .font_size(theme.font_size_sm)
                            .single_line()
                            .grow()
                            .shrink(1.0)
                            .min_width(UIValue::Px(0.0)),
                        AssetPickerLabel { row: entity },
                    ))
                    .add_child(icon(theme, glyph::CARET_DOWN, theme.font_size_sm).muted());
            },
        );
    }

    fn apply(
        &self,
        value: &mut AssetHandle<T>,
        edit: &AssetId,
        context: &EditContext,
    ) -> Result<(), EditError> {
        let holds_t = context
            .resource::<AssetRegistry>()
            .and_then(|registry| registry.get(*edit))
            .is_some_and(AssetEntry::is::<T>);
        let (true, Some(assets), Some(_)) = (
            holds_t,
            context.resource::<AssetServer>(),
            context.resource::<AssetStore<T>>(),
        ) else {
            return Err(EditError::Rejected);
        };
        *value = assets.load(*edit);
        Ok(())
    }
}

fn held(values: &Query<&PropertyRowValue>, row: Entity) -> Option<AssetId> {
    values
        .get_entity(row)
        .and_then(|value| value.downcast::<AssetId>().copied())
}

fn asset_name(registry: &AssetRegistry, id: AssetId) -> String {
    if id.is_nil() {
        return "None".into();
    }
    registry
        .get(id)
        .map_or_else(|| "Missing".into(), |entry| entry.name().into())
}

fn menu_panel(button: Entity) -> UIAnchoredPanel {
    UIAnchoredPanel::new(UIAnchorTarget::from_node(button))
        .with_owner(button)
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::End)
        .with_gap(4.0)
        .with_open(true)
}

fn toggle_asset_picker<T: LoadableAsset>(
    on: On<UIClick>,
    buttons: Query<&AssetPickerButton>,
    values: Query<&PropertyRowValue>,
    menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
    registry: Res<AssetRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(button) = buttons.get_entity(on.entity()) else {
        return;
    };
    let mut menu = match menus.iter().next() {
        Some((entity, mut menu, mut panel)) => {
            if panel.open && panel.owner == Some(on.entity()) {
                panel.open = false;
                return;
            }
            menu.row = Some(button.row);
            **panel = menu_panel(on.entity());
            cmd.entity(entity).despawn_children()
        }
        None => cmd.spawn((
            theme
                .popup()
                .fill(MENU_FILL)
                .width(UIValue::Px(MENU_WIDTH))
                .padding(theme.spacing_xs),
            menu_panel(on.entity()),
            AssetPickerMenu {
                row: Some(button.row),
            },
        )),
    };
    fill_menu::<T>(&mut menu, &registry, held(&values, button.row), &theme);
}

fn fill_menu<T: LoadableAsset>(
    menu: &mut EntityCommandQueue,
    registry: &AssetRegistry,
    current: Option<AssetId>,
    theme: &UITheme,
) {
    let entries: Vec<_> = registry.of_type::<T>().collect();
    if entries.is_empty() {
        menu.add_child(
            theme
                .label(&format!("No {} assets", T::name()))
                .muted()
                .font_size(12.0)
                .no_wrap()
                .padding(UIRect::axes(6.0, 8.0)),
        );
        return;
    }
    let extent = entries.len() as f32 * ITEM_HEIGHT;
    menu.add_child_with(
        theme
            .column()
            .fixed()
            .height(UIValue::Px(extent.min(LIST_MAX_HEIGHT)))
            .clipped(),
        |mut view| {
            let mut list = view.spawn_child(theme.column().padding(UIRect {
                right: SCROLLBAR_GUTTER,
                ..UIRect::all(0.0)
            }));
            for entry in &entries {
                list.add_child_with(
                    (
                        theme
                            .pressable()
                            .selected(current == Some(entry.id))
                            .row()
                            .gap(8.0)
                            .fixed()
                            .height(UIValue::Px(ITEM_HEIGHT))
                            .padding(UIRect::axes(0.0, 8.0))
                            .on_click(pick_asset::<T>),
                        AssetPickerItem(entry.id),
                    ),
                    |mut item| {
                        item.add_child(
                            theme
                                .label(entry.name())
                                .font_size(12.5)
                                .single_line()
                                .grow()
                                .shrink(1.0)
                                .min_width(UIValue::Px(0.0)),
                        )
                        .add_child(
                            theme
                                .label(entry.folder())
                                .muted()
                                .font_size(10.5)
                                .no_wrap(),
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

fn pick_asset<T: LoadableAsset>(
    on: On<UIClick>,
    items: Query<&AssetPickerItem>,
    menus: Query<(&AssetPickerMenu, &mut UIAnchoredPanel)>,
    rows: Query<(&PropertyRow, &PropertyRowValue)>,
    mut commits: ResMut<PropertyCommits>,
) {
    if on.signal().button != MouseButton::Left {
        return;
    }
    let Some(item) = items.get_entity(on.entity()) else {
        return;
    };
    for (menu, mut panel) in menus.iter() {
        if let Some((row, value)) = menu
            .row
            .filter(|_| panel.open)
            .and_then(|row| rows.get_entity(row))
            && value.downcast::<AssetId>() != Some(&item.0)
            && let Err(error) = commits.push::<AssetHandle<T>, AssetPicker<T>>(row, item.0)
        {
            log::warn!("Asset pick dropped: {error}");
        }
        panel.open = false;
    }
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    menus: Query<(&mut AssetPickerMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter() {
        if menu.row.is_some() {
            menu.row = None;
        }
        if panel.open {
            panel.open = false;
        }
    }
}

/// Writes the name of each picker row's asset into its label.
pub(crate) fn refresh_asset_picker_labels(
    values: Query<&PropertyRowValue>,
    labels: Query<(&AssetPickerLabel, &mut UIText)>,
    registry: Res<AssetRegistry>,
) {
    for (label, mut text) in labels.iter() {
        let Some(id) = held(&values, label.row) else {
            continue;
        };
        let name = asset_name(&registry, id);
        if text.text != name {
            text.text = name;
        }
    }
}

#[cfg(test)]
mod tests;
