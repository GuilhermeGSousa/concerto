//! The property model: what a value's target, address, rows and editors are, for the inspector panel and for the edit `App`'s tools alike.
mod editor;
mod rows;
mod target;

pub use editor::ErasedEditor;
pub(crate) use rows::ListLength;
pub use rows::{
    EditContext, EditError, PlainRow, Property, PropertyCommit, PropertyCommits, PropertyEditor,
    PropertyKind, PropertyRow, PropertyRowValue, standard_row_node,
};
pub use target::{ComponentTarget, PropertyAddress, PropertyTarget, TargetWorld};
