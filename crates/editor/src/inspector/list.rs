use std::{any::Any, sync::Arc};

use concerto_ecs::{
    Entity, Query, ResMut,
    command::CommandQueue,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_reflect::{Reflect, ReflectMut, ReflectRef};
use concerto_ui::elements::{Listening, Pressable, Text, prelude::*};
use concerto_ui::{interaction::UIClick, node::UIRect, theme::UITheme, transform::UIValue};
use concerto_window::input::MouseButton;

use super::{
    EditContext, EditError, ErasedEditor, PropertyCommit, PropertyCommits, PropertyRow,
    PropertyRowValue,
};
use crate::fonts::{glyph, icon};

/// Shows a reflected list's length and adds to it; its elements follow as rows of their own.
pub(crate) struct ListEditor;

/// A change to a reflected list's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListEdit {
    /// Appends a default element.
    Push,
    /// Removes the element at this index.
    Remove(usize),
}

impl ErasedEditor for ListEditor {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        let ReflectRef::List(list) = value.reflect_ref() else {
            return Err(EditError::TypeMismatch);
        };
        Ok(PropertyRowValue::erased::<Self, usize>(
            (value as &dyn Any).type_id(),
            list.len(),
        ))
    }

    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError> {
        let length = *snapshot
            .downcast::<usize>()
            .ok_or(EditError::TypeMismatch)?;
        cmd.entity(row)
            .add_child(
                theme
                    .label(format!("Length {length}"))
                    .small()
                    .muted()
                    .single_line()
                    .grow(),
            )
            .add_child(list_button(
                theme,
                glyph::PLUS,
                edit_list(row, ListEdit::Push),
            ));
        Ok(())
    }

    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        _: &EditContext,
    ) -> Result<(), EditError> {
        let edit = edit
            .downcast_ref::<ListEdit>()
            .ok_or(EditError::TypeMismatch)?;
        let ReflectMut::List(list) = value.reflect_mut() else {
            return Err(EditError::TypeMismatch);
        };
        match *edit {
            ListEdit::Push => list.push_default(),
            ListEdit::Remove(index) => {
                if !list.remove(index) {
                    return Err(EditError::NotFound);
                }
            }
        }
        Ok(())
    }

    fn edit_to(&self, _: &dyn Reflect) -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError> {
        Ok(None)
    }

    fn follows_snapshot(&self) -> bool {
        false
    }
}

/// The button at the end of an element's row, which removes element `index` of the list whose row is `list`.
pub(super) fn remove_button(
    theme: &UITheme,
    list: Entity,
    index: usize,
) -> (Listening<Pressable, UIClick>, Text) {
    list_button(
        theme,
        glyph::MINUS,
        edit_list(list, ListEdit::Remove(index)),
    )
}

fn list_button(
    theme: &UITheme,
    mark: char,
    click: Listener<UIClick>,
) -> (Listening<Pressable, UIClick>, Text) {
    let size = theme.font_size_md;
    let vertical = (theme.control_height - theme.line_height(size)) / 2.0;
    let horizontal = (theme.control_height - size) / 2.0;
    (
        theme
            .pressable()
            .size(
                UIValue::Px(theme.control_height),
                UIValue::Px(theme.control_height),
            )
            .padding(UIRect::axes(vertical, horizontal))
            .on_click(click),
        icon(theme, mark, size).muted(),
    )
}

/// Commits `edit` to the list whose row is `row`.
fn edit_list(row: Entity, edit: ListEdit) -> Listener<UIClick> {
    (move |on: On<UIClick>, rows: Query<&PropertyRow>, mut commits: ResMut<PropertyCommits>| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(target) = rows.get_entity(row) else {
            return;
        };
        match PropertyCommit::erased::<ListEditor>(target, edit) {
            Ok(commit) => commits.0.push(commit),
            Err(error) => log::warn!("List edit dropped: {error}"),
        }
    })
    .into_listener()
}
