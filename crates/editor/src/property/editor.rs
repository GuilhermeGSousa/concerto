use std::{any::Any, sync::Arc};

use concerto_ecs::{Entity, command::CommandQueue};
use concerto_reflect::Reflect;
use concerto_ui::{node::UINode, theme::UITheme};

use super::rows::{EditContext, EditError, PropertyRowValue, standard_row_node};

/// A registered property editor with its value and edit types erased.
pub trait ErasedEditor: Send + Sync {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError>;
    /// The layout node of the row, read once when the row is spawned.
    fn row_node(&self, _snapshot: &PropertyRowValue, theme: &UITheme) -> Result<UINode, EditError> {
        Ok(standard_row_node(theme))
    }
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &PropertyRowValue,
        theme: &UITheme,
    ) -> Result<(), EditError>;
    fn apply(
        &self,
        value: &mut dyn Reflect,
        edit: &dyn Any,
        context: &EditContext,
    ) -> Result<(), EditError>;
    /// The erased edit that [`Self::apply`] takes to set the field to `value`, or `None` when the editor offers no value edit.
    fn edit_to(&self, value: &dyn Reflect)
    -> Result<Option<Arc<dyn Any + Send + Sync>>, EditError>;
    fn follows_snapshot(&self) -> bool;
}
