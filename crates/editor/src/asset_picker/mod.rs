use std::{any::Any, sync::Arc};

use concerto_app::{App, Plugin, schedule_groups::LateUpdate};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut,
    command::CommandQueue,
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
use concerto_ui::elements::{Label, Pressable, Text, prelude::*};
use concerto_ui::{
    anchor::UIAnchoredPanel,
    interaction::UIClick,
    items::UIItemActivated,
    node::{UINode, UIRect},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use crate::fonts::{glyph, icon};
use crate::hosted::EditTarget;
use crate::inspector::{
    EditContext, EditError, ErasedEditor, PropertyCommit, PropertyCommits, PropertyRow,
    PropertyRowValue,
};
use crate::picker_menu::{picker_list, picker_panel, show_picker_menu};

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

/// The menu every picker shares, pointed at the row whose button opened it last, if a row did.
#[derive(Component)]
pub struct AssetPickerMenu {
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
            picker_button(theme).on_click(toggle_asset_picker(entity, self.handle.kind())),
            |mut button| {
                button
                    .add_child((picker_label(theme), AssetPickerLabel { row: entity }))
                    .add_child(picker_caret(theme));
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

    fn edit_to(
        &self,
        value: &dyn Reflect,
    ) -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError> {
        let id = self.handle.id(value).ok_or(EditError::TypeMismatch)?;
        Ok(Some(Arc::new(id)))
    }
}

fn held(values: &Query<&PropertyRowValue>, row: Entity) -> Option<AssetId> {
    values
        .get_entity(row)
        .and_then(|value| value.downcast::<AssetId>().copied())
}

/// The name of the asset `id` names, `None` for a nil id and `Missing` for one not in `registry`.
pub fn asset_name(registry: &AssetRegistry, id: AssetId) -> String {
    if id.is_nil() {
        return "None".into();
    }
    registry
        .get(id)
        .map_or_else(|| "Missing".into(), |entry| entry.name().into())
}

/// A picker's button, which takes the free space of the row it sits in.
pub fn picker_button(theme: &UITheme) -> Pressable {
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
}

/// The text inside a [`picker_button`], for the name of the asset it holds.
pub fn picker_label(theme: &UITheme) -> Label {
    theme
        .label("")
        .font_size(theme.font_size_sm)
        .single_line()
        .grow()
        .shrink(1.0)
        .min_width(UIValue::Px(0.0))
}

/// The caret that closes a [`picker_button`].
pub fn picker_caret(theme: &UITheme) -> (UINode, Text) {
    (
        UINode::default().with_flex_shrink(0.0),
        icon(theme, glyph::CARET_DOWN, theme.font_size_sm).muted(),
    )
}

/// Opens `row`'s assets in the shared menu, or closes it if the click came from its own button.
fn toggle_asset_picker(row: Entity, kind: &'static str) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           values: Query<&PropertyRowValue>,
           mut menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
           registry: Res<AssetRegistry>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        open_asset_menu(
            on.entity(),
            kind,
            Some(row),
            held(&values, row),
            pick_asset,
            &mut menus,
            &registry,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

/// Opens the shared menu under `button` on `kind`'s assets, or closes it if `button` opened it.
///
/// The menu is searchable by name and folder, and each item's pick is built by `pick`. `row` is
/// `None` for a menu that commits to no property.
pub fn open_asset_menu(
    button: Entity,
    kind: &str,
    row: Option<Entity>,
    current: Option<AssetId>,
    pick: impl Fn(AssetId) -> Listener<UIItemActivated>,
    menus: &mut Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
    registry: &AssetRegistry,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) {
    let mut menu = None;
    if let Some((entity, _, mut panel)) = menus.iter_mut().next() {
        if panel.open && panel.owner == Some(button) {
            panel.open = false;
            return;
        }
        menu = Some(entity);
    }
    let list = picker_list(theme)
        .searchable("Search assets…")
        .empty_text(format!("No {kind} assets"))
        .items(registry.of_kind(kind).map(|entry| {
            theme
                .list_item(entry.name())
                .detail(entry.folder())
                .selected(current == Some(entry.id))
                .on_activate(pick(entry.id))
        }));
    show_picker_menu(
        menu,
        AssetPickerMenu { row },
        list,
        |field| picker_panel(button, field),
        theme,
        cmd,
    );
}

/// Commits `id` to the row the open menu was opened for, then closes the menu.
fn pick_asset(id: AssetId) -> Listener<UIItemActivated> {
    (move |_: On<UIItemActivated>,
           mut menus: Query<(&AssetPickerMenu, &mut UIAnchoredPanel)>,
           rows: Query<(&PropertyRow, &PropertyRowValue)>,
           mut commits: ResMut<PropertyCommits>| {
        for (menu, mut panel) in menus.iter_mut() {
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
    mut menus: Query<(&mut AssetPickerMenu, &mut UIAnchoredPanel)>,
) {
    for (mut menu, mut panel) in menus.iter_mut() {
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
    mut labels: Query<(&AssetPickerLabel, &mut UIText)>,
    registry: Res<AssetRegistry>,
) {
    for (label, mut text) in labels.iter_mut() {
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
