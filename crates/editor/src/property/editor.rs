use std::{any::Any, sync::Arc};

use concerto_ecs::{Entity, command::CommandQueue};
use concerto_reflect::Reflect;
use concerto_ui::{node::UINode, theme::UITheme};

use super::rows::{EditContext, EditError, PropertyRowValue, standard_row_node};

/// A registered property editor with its value and edit types erased.
pub trait ErasedEditor: Send + Sync {
    /// Reads `value` into the row value this editor shows.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `value` is not of a type this editor reads.
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError>;
    /// The layout node of the row, read once when the row is spawned.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `snapshot` was not taken by this editor.
    fn row_node(&self, _snapshot: &PropertyRowValue, theme: &UITheme) -> Result<UINode, EditError> {
        Ok(standard_row_node(theme))
    }
    /// Builds the row's widgets under `row` from `snapshot`.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `snapshot` was not taken by this editor.
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError>;
    /// Applies `edit` to `value`.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `value` or `edit` is not of this editor's types,
    /// and another [`EditError`], such as [`EditError::Rejected`] or [`EditError::NotFound`], when
    /// the editor refuses the edit.
    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        context: &EditContext,
    ) -> Result<(), EditError>;
    /// The erased edit that [`Self::apply`] takes to set the field to `value`.
    ///
    /// `None` when the editor offers no value edit.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `value` is not the editor's value type.
    fn edit_to(&self, value: &dyn Reflect)
    -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError>;
    /// Whether the editor's widgets keep themselves current from the row's snapshot, as
    /// [`PropertyEditor::follows_snapshot`](super::PropertyEditor::follows_snapshot) says.
    fn follows_snapshot(&self) -> bool;
}
