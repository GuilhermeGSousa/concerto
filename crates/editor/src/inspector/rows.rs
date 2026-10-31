use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{Component, Resource, command::EntityCommandQueue, component::Tick};
use concerto_reflect::{PropertyPath, Reflect};
use concerto_ui::{node::UINode, theme::UITheme};

use super::list::ListLength;
use super::target::{PropertyAddress, PropertyTarget, TargetWorld};
pub use crate::tools::edits::{EditContext, EditError, PropertyRow};

/// Presentation and validated editing for a concrete Rust type.
pub trait PropertyEditor<T: Reflect>: Default + Send + Sync + 'static {
    type Snapshot: Clone + PartialEq + Send + Sync + 'static;
    type Edit: Send + Sync + 'static;

    fn snapshot(&self, value: &T) -> Self::Snapshot;
    /// The layout node of the row that holds this editor's widgets. It is read once, when the row is spawned, so an editor whose node depends on the snapshot must return `false` from [`Self::follows_snapshot`].
    fn row_node(&self, _snapshot: &Self::Snapshot, theme: &UITheme) -> UINode {
        super::sync::standard_row_node(theme)
    }
    fn build(&self, row: &mut EntityCommandQueue, snapshot: &Self::Snapshot, theme: &UITheme);
    /// Validate before mutating. On error the target must remain unchanged.
    fn apply(
        &self,
        value: &mut T,
        edit: &Self::Edit,
        context: &EditContext,
    ) -> Result<(), EditError>;
    /// The edit that sets the field to `value` when applied, or `None` when this editor offers no value edit.
    fn edit_to(&self, _value: &T) -> Option<Self::Edit> {
        None
    }
    /// Whether the widgets `build` spawns keep themselves current from the row's [`PropertyRowValue`], so a value change leaves the row in place.
    fn follows_snapshot(&self) -> bool {
        false
    }
}

#[derive(Clone)]
struct Snapshot {
    data: Arc<dyn Any + Send + Sync>,
    value_type: TypeId,
    editor_type: TypeId,
    equals: fn(&dyn Any, &dyn Any) -> bool,
}

/// Owned snapshot refreshed independently of the widget's focused edit buffer.
#[derive(Component, Clone)]
pub struct PropertyRowValue(Snapshot);

impl PropertyRowValue {
    pub(crate) fn new<T: Reflect, E: PropertyEditor<T>>(value: E::Snapshot) -> Self {
        Self::erased::<E, E::Snapshot>(TypeId::of::<T>(), value)
    }

    /// A snapshot from editor `E` of a value whose type is known only by id.
    pub(crate) fn erased<E: 'static, S: Clone + PartialEq + Send + Sync + 'static>(
        value_type: TypeId,
        value: S,
    ) -> Self {
        Self(Snapshot {
            data: Arc::new(value),
            value_type,
            editor_type: TypeId::of::<E>(),
            equals: |a, b| match (a.downcast_ref::<S>(), b.downcast_ref::<S>()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        })
    }

    /// Returns `None` for a different value/adapter type.
    pub fn snapshot<T: Reflect, E: PropertyEditor<T>>(&self) -> Option<&E::Snapshot> {
        if self.0.value_type != TypeId::of::<T>() || self.0.editor_type != TypeId::of::<E>() {
            return None;
        }
        self.0.data.downcast_ref()
    }

    pub(crate) fn downcast<S: Any>(&self) -> Option<&S> {
        self.0.data.downcast_ref()
    }
}

impl PartialEq for PropertyRowValue {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (&self.0, &other.0);
        a.value_type == b.value_type
            && a.editor_type == b.editor_type
            && (a.equals)(a.data.as_ref(), b.data.as_ref())
    }
}

/// One inspector row, selected by consulting adapters before visiting children.
#[derive(Clone)]
pub struct Property {
    pub path: PropertyPath,
    pub type_id: TypeId,
    pub kind: PropertyKind,
}

/// What a [`Property`]'s row shows.
#[derive(Clone)]
pub enum PropertyKind {
    /// The snapshot of the editor that claimed the value.
    Value(PropertyRowValue),
    /// A list element with no row of its own: its index and remove button, ahead of its fields' rows.
    Heading,
    /// A value no editor claims.
    Unsupported,
}

impl Property {
    /// The snapshot of the editor that claimed this property, if one did.
    pub fn value(&self) -> Option<&PropertyRowValue> {
        match &self.kind {
            PropertyKind::Value(value) => Some(value),
            PropertyKind::Heading | PropertyKind::Unsupported => None,
        }
    }

    /// The length of the list this property shows, when it is a list's own row.
    pub(crate) fn list_length(&self) -> Option<usize> {
        let ListLength(length) = self.value()?.downcast()?;
        Some(*length)
    }

    /// The editor's row for this property on `target`, or `None` when no editor claimed it.
    pub fn row(&self, target: Arc<dyn PropertyTarget>, registry_tick: Tick) -> Option<PropertyRow> {
        Some(PropertyRow {
            editor_type: self.value()?.0.editor_type,
            address: PropertyAddress {
                target,
                path: self.path.clone(),
                type_id: self.type_id,
            },
            registry_tick,
        })
    }
}

/// A row with no value of its own: a heading, or a value no editor claims.
#[derive(Component, Clone)]
pub struct PlainRow {
    pub path: PropertyPath,
    pub type_id: TypeId,
}

/// An owned edit and its captured target. Construct through [`Self::new`].
pub struct PropertyCommit {
    pub row: PropertyRow,
    pub(crate) edit: Arc<dyn Any + Send + Sync>,
}

impl PropertyCommit {
    pub fn new<T: Reflect, E: PropertyEditor<T>>(
        row: &PropertyRow,
        edit: E::Edit,
    ) -> Result<Self, EditError> {
        if row.address.type_id != TypeId::of::<T>() {
            return Err(EditError::TypeMismatch);
        }
        Self::erased::<E>(row, edit)
    }

    /// An edit from editor `E` to a row whose value type is known only by id.
    pub(crate) fn erased<E: 'static>(
        row: &PropertyRow,
        edit: impl Any + Send + Sync,
    ) -> Result<Self, EditError> {
        if row.editor_type != TypeId::of::<E>() {
            return Err(EditError::TypeMismatch);
        }
        Ok(Self {
            row: row.clone(),
            edit: Arc::new(edit),
        })
    }

    fn lives_in(&self, world: TargetWorld) -> bool {
        self.row.address.target.world() == world
    }
}

/// Edits waiting to be checked against the widget registry and sent to their target.
#[derive(Resource, Default)]
pub struct PropertyCommits(pub Vec<PropertyCommit>);

impl PropertyCommits {
    pub fn push<T: Reflect, E: PropertyEditor<T>>(
        &mut self,
        row: &PropertyRow,
        edit: E::Edit,
    ) -> Result<(), EditError> {
        self.0.push(PropertyCommit::new::<T, E>(row, edit)?);
        Ok(())
    }

    /// Whether a waiting edit's target lives in `world`.
    pub(crate) fn holds_for(&self, world: TargetWorld) -> bool {
        self.0.iter().any(|commit| commit.lives_in(world))
    }

    /// Takes the edits whose targets live in `world`, leaving the others in place.
    pub(crate) fn take_for(&mut self, world: TargetWorld) -> Vec<PropertyCommit> {
        let (taken, left) = std::mem::take(&mut self.0)
            .into_iter()
            .partition(|commit| commit.lives_in(world));
        self.0 = left;
        taken
    }
}
