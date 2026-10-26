use std::any::Any;

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::{CommandQueue, EntityCommandQueue},
    signal::{
        On,
        lifecycle::Remove,
        listener::{IntoListener, Listener},
    },
};
use concerto_foundation::assets::{
    AssetId,
    handle::{ReflectAssetHandle, SetHandleError},
    registry::AssetRegistry,
};
use concerto_reflect::Reflect;
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
    EditContext, EditError, ErasedEditor, MENU_FILL, PropertyCommit, PropertyCommits, PropertyRow,
    PropertyRowValue,
};

const MENU_WIDTH: f32 = 240.0;
const ITEM_HEIGHT: f32 = 26.0;
const LIST_MAX_HEIGHT: f32 = 280.0;
const SCROLLBAR_GUTTER: f32 = 10.0;

/// Picks the asset a reflected asset handle names from the project's assets of its kind.
pub struct AssetPicker {
    handle: ReflectAssetHandle,
}

impl AssetPicker {
    pub fn new(handle: ReflectAssetHandle) -> Self {
        Self { handle }
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

/// The text inside an [`AssetPicker`] row's button, naming the row's asset.
#[derive(Component)]
pub(crate) struct AssetPickerLabel {
    row: Entity,
}

/// The menu every picker shares, pointed at the row whose button opened it last.
#[derive(Component)]
pub(crate) struct AssetPickerMenu {
    row: Option<Entity>,
}

impl ErasedEditor for AssetPicker {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        let id = self.handle.id(value).ok_or(EditError::TypeMismatch)?;
        Ok(PropertyRowValue::erased::<Self, AssetId>(
            (value as &dyn Any).type_id(),
            id,
        ))
    }

    fn follows_snapshot(&self) -> bool {
        true
    }

    fn build(
        &self,
        cmd: &mut CommandQueue,
        entity: Entity,
        _: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        cmd.entity(entity).add_child_with(
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
                .on_click(toggle_asset_picker(entity, self.handle.kind())),
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
        Ok(())
    }

    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        context: &EditContext,
    ) -> Result<(), EditError> {
        let id = *edit
            .downcast_ref::<AssetId>()
            .ok_or(EditError::TypeMismatch)?;
        let of_kind = context
            .registry
            .get(id)
            .is_some_and(|entry| entry.kind == self.handle.kind());
        if !of_kind {
            return Err(EditError::Rejected);
        }
        self.handle
            .set(value, context.assets, id)
            .map_err(|error| match error {
                SetHandleError::NotThisHandle => EditError::TypeMismatch,
                SetHandleError::AssetNotRegistered => EditError::Rejected,
            })
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

/// Opens the shared menu on `row`'s assets, or closes it if `row`'s button opened it.
fn toggle_asset_picker(row: Entity, kind: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           registry: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let mut menu = match menus.iter().next() {
            Some((entity, mut menu, mut panel)) => {
                if panel.open && panel.owner == Some(on.entity()) {
                    panel.open = false;
                    return;
                }
                menu.row = Some(row);
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
                AssetPickerMenu { row: Some(row) },
            )),
        };
        fill_menu(&mut menu, kind, &registry, held(&values, row), &theme);
    })
    .into_listener()
}

fn fill_menu(
    menu: &mut EntityCommandQueue,
    kind: &str,
    registry: &AssetRegistry,
    current: Option<AssetId>,
    theme: &UITheme,
) {
    let entries: Vec<_> = registry.of_kind(kind).collect();
    if entries.is_empty() {
        menu.add_child(
            theme
                .label(&format!("No {kind} assets"))
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
                    theme
                        .pressable()
                        .selected(current == Some(entry.id))
                        .row()
                        .gap(8.0)
                        .fixed()
                        .height(UIValue::Px(ITEM_HEIGHT))
                        .padding(UIRect::axes(0.0, 8.0))
                        .on_click(pick_asset(entry.id)),
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

/// Commits `id` to the row the open menu was opened for, then closes the menu.
fn pick_asset(id: AssetId) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           menus: Query<(&AssetPickerMenu, &mut UIAnchoredPanel)>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        for (menu, mut panel) in menus.iter() {
            if let Some((row, value)) = menu
                .row
                .filter(|_| panel.open)
                .and_then(|row| rows.get_entity(row))
                && value.downcast::<AssetId>() != Some(&id)
            {
                match PropertyCommit::erased::<AssetPicker>(row, id) {
                    Ok(commit) => commits.0.push(commit),
                    Err(error) => log::warn!("Asset pick dropped: {error}"),
                }
            }
            panel.open = false;
        }
    })
    .into_listener()
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
