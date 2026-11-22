use std::{any::TypeId, marker::PhantomData};

use concerto_animation::param::Param;
use concerto_app::App;
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, With,
    command::{CommandQueue, EntityCommandQueue},
    events::event_reader::EventReader,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_editor::ActiveEditor,
    asset_picker::picker_caret,
    inspector::{
        EditContext, EditError, InspectorRegistry, PropertyCommits, PropertyEditor, PropertyRow,
        PropertyRowValue,
    },
    name_menu::{NameMenu, close_menu, open_name_menu},
};
use concerto_name::Name;
use concerto_reflect::Reflect;
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::UIAnchoredPanel,
    focus::{FocusedWidget, UIFocusLost},
    interaction::UIClick,
    node::UIRect,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::MouseButton;

use super::document::GraphDocument;

/// A text field for the name a `Param<T>` holds, beside a button that lists the names the front graph tab has found for `T`.
pub struct ParamNameEditor<T>(PhantomData<fn() -> T>);

impl<T> Default for ParamNameEditor<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// The text field of a [`ParamNameEditor`] row and the name it last showed.
#[derive(Component)]
pub(crate) struct ParamNameField {
    row: Entity,
    displayed: Name,
    read: fn(&PropertyRowValue) -> Option<Name>,
    commit: fn(&mut PropertyCommits, &PropertyRow, Name) -> Result<(), EditError>,
}

fn read_name<T: Reflect + Default>(value: &PropertyRowValue) -> Option<Name> {
    value.snapshot::<Param<T>, ParamNameEditor<T>>().copied()
}

fn commit_name<T: Reflect + Default>(
    commits: &mut PropertyCommits,
    row: &PropertyRow,
    name: Name,
) -> Result<(), EditError> {
    commits.push::<Param<T>, ParamNameEditor<T>>(row, name)
}

impl<T: Reflect + Default> PropertyEditor<Param<T>> for ParamNameEditor<T> {
    type Snapshot = Name;
    type Edit = Name;

    fn snapshot(&self, value: &Param<T>) -> Name {
        value.0
    }

    fn build(&self, row: &mut EntityCommandQueue, value: &Name, theme: &UITheme) {
        let entity = row.entity();
        row.add_child((
            theme
                .text_field("")
                .small()
                .grow()
                .shrink(1.0)
                .width(UIValue::Px(0.0))
                .min_width(UIValue::Px(0.0))
                .padding(UIRect::axes(
                    ((theme.control_height - theme.line_height(theme.font_size_sm)) / 2.0).max(0.0),
                    theme.spacing_xs,
                ))
                .clipped(),
            ParamNameField {
                row: entity,
                displayed: *value,
                read: read_name::<T>,
                commit: commit_name::<T>,
            },
        ));
        row.add_child_with(
            theme
                .pressable()
                .radius_sm()
                .fixed()
                .padding(UIRect::axes(0.0, theme.spacing_xs))
                .on_click(toggle_param_menu::<T>(entity)),
            |mut button| {
                button.add_child(picker_caret(theme));
            },
        );
    }

    fn apply(&self, value: &mut Param<T>, edit: &Name, _: &EditContext) -> Result<(), EditError> {
        *value = Param::new(*edit);
        Ok(())
    }

    fn edit_to(&self, value: &Param<T>) -> Option<Name> {
        Some(value.0)
    }

    fn follows_snapshot(&self) -> bool {
        true
    }
}

fn toggle_param_menu<T: Reflect + Default>(row: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           active: Res<ActiveEditor>,
           documents: Query<&GraphDocument>,
           values: Query<&PropertyRowValue>,
           mut menus: Query<(Entity, &mut UIAnchoredPanel), With<NameMenu>>,
           theme: Res<UITheme>,
           mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let names: Vec<&'static str> = active
            .0
            .and_then(|document| documents.get_entity(document))
            .map(|state| {
                state
                    .parameters
                    .shown()
                    .filter(|param| param.value_type() == TypeId::of::<T>())
                    .map(|param| param.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let current = values.get_entity(row).and_then(read_name::<T>);
        open_name_menu(
            on.entity(),
            &names,
            current.as_ref().map(Name::as_str),
            |name| pick_param::<T>(row, Name::new(name)),
            &mut menus,
            &theme,
            &mut cmd,
        );
    })
    .into_listener()
}

fn pick_param<T: Reflect + Default>(row: Entity, name: Name) -> Listener<UIClick> {
    (move |on: On<UIClick>,
           mut menus: Query<&mut UIAnchoredPanel, With<NameMenu>>,
           rows: Query<&PropertyRow>,
           mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        close_menu(&mut menus);
        let Some(target) = rows.get_entity(row) else {
            return;
        };
        if let Err(error) = commit_name::<T>(&mut commits, target, name) {
            log::warn!("Parameter pick dropped: {error}");
        }
    })
    .into_listener()
}

fn revert(field: &ParamNameField, input: &mut UITextInput) {
    input.value = field.displayed.as_str().into();
    input.cursor = input.value.len();
    input.selection_anchor = None;
}

pub(crate) fn commit_param_fields(
    mut submitted: EventReader<UITextInputSubmitted>,
    mut lost: EventReader<UIFocusLost>,
    mut cancelled: EventReader<UITextInputCancelled>,
    rows: Query<&PropertyRow>,
    mut fields: Query<(&mut ParamNameField, &mut UITextInput)>,
    mut commits: ResMut<PropertyCommits>,
    mut focused: ResMut<FocusedWidget>,
) {
    for event in submitted.read() {
        let Some((mut field, mut input)) = fields.get_entity_mut(event.entity) else {
            continue;
        };
        let Some(row) = rows.get_entity(field.row) else {
            continue;
        };
        let name = Name::new(input.value.trim());
        if name == field.displayed || (field.commit)(&mut commits, row, name).is_err() {
            revert(&field, &mut input);
            continue;
        }
        field.displayed = name;
    }
    for event in lost.read() {
        if let Some((field, mut input)) = fields.get_entity_mut(event.0) {
            revert(&field, &mut input);
        }
    }
    for event in cancelled.read() {
        let Some((field, mut input)) = fields.get_entity_mut(event.entity) else {
            continue;
        };
        revert(&field, &mut input);
        if **focused == Some(event.entity) {
            **focused = None;
        }
    }
}

pub(crate) fn refresh_param_fields(
    focused: Res<FocusedWidget>,
    rows: Query<&PropertyRowValue>,
    mut fields: Query<(Entity, &mut ParamNameField, &mut UITextInput)>,
) {
    for (entity, mut field, mut input) in fields.iter_mut() {
        if **focused == Some(entity) {
            continue;
        }
        let Some(name) = rows.get_entity(field.row).and_then(field.read) else {
            continue;
        };
        if input.value != name.as_str() {
            input.value = name.as_str().into();
            input.cursor = input.value.len();
            input.selection_anchor = None;
        }
        if field.displayed != name {
            field.displayed = name;
        }
    }
}

/// Gives `Param<T>` fields their editor.
pub trait ParamEditorAppExt {
    /// Registers [`ParamNameEditor`] for `Param<T>`; does nothing in an `App` without the inspector.
    fn register_param_editor<T: Reflect + Default>(&mut self) -> &mut Self;
}

impl ParamEditorAppExt for App {
    fn register_param_editor<T: Reflect + Default>(&mut self) -> &mut Self {
        if let Some(registry) = self.get_resource_mut::<InspectorRegistry>() {
            registry.register_property_editor::<Param<T>, ParamNameEditor<T>>();
        }
        self
    }
}
