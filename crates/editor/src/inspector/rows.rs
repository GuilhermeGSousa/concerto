use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{Component, Entity, Resource, command::EntityCommandQueue, component::Tick};
use concerto_reflect::{PropertyPath, Reflect};
use concerto_ui::{node::UINode, theme::UITheme};

use super::list::ListEditor;
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
pub(crate) struct Snapshot {
    data: Arc<dyn Any + Send + Sync>,
    value_type: TypeId,
    editor_type: TypeId,
    equals: fn(&dyn Any, &dyn Any) -> bool,
}

/// Owned snapshot refreshed independently of the widget's focused edit buffer.
#[derive(Component, Clone, Default)]
pub struct PropertyRowValue(pub(crate) Option<Snapshot>);

impl PropertyRowValue {
    pub(crate) fn new<T: Reflect, E: PropertyEditor<T>>(value: E::Snapshot) -> Self {
        Self::erased::<E, E::Snapshot>(TypeId::of::<T>(), value)
    }

    /// A snapshot from editor `E` of a value whose type is known only by id.
    pub(crate) fn erased<E: 'static, S: Clone + PartialEq + Send + Sync + 'static>(
        value_type: TypeId,
        value: S,
    ) -> Self {
        Self(Some(Snapshot {
            data: Arc::new(value),
            value_type,
            editor_type: TypeId::of::<E>(),
            equals: |a, b| match (a.downcast_ref::<S>(), b.downcast_ref::<S>()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }))
    }

    /// Returns `None` for an unsupported row or a different value/adapter type.
    pub fn snapshot<T: Reflect, E: PropertyEditor<T>>(&self) -> Option<&E::Snapshot> {
        let snapshot = self.0.as_ref()?;
        if snapshot.value_type != TypeId::of::<T>() || snapshot.editor_type != TypeId::of::<E>() {
            return None;
        }
        snapshot.data.downcast_ref()
    }

    pub(crate) fn downcast<S: Any>(&self) -> Option<&S> {
        self.0.as_ref()?.data.downcast_ref()
    }
}

impl PartialEq for PropertyRowValue {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.value_type == b.value_type
                    && a.editor_type == b.editor_type
                    && (a.equals)(a.data.as_ref(), b.data.as_ref())
            }
            _ => false,
        }
    }
}

/// One inspector row, selected by consulting adapters before visiting children.
#[derive(Clone)]
pub struct Property {
    pub path: PropertyPath,
    pub type_id: TypeId,
    pub value: PropertyRowValue,
    pub(crate) editor_type: Option<TypeId>,
    pub(crate) heading: bool,
    pub(crate) list_element: Option<usize>,
    pub(crate) registry_tick: Tick,
}

impl Property {
    /// Whether an adapter claimed this property.
    pub fn has_editor(&self) -> bool {
        self.editor_type.is_some()
    }

    /// Whether this is a list's own row, the one [`ListEditor`] claimed.
    pub(crate) fn is_list(&self) -> bool {
        self.editor_type == Some(TypeId::of::<ListEditor>())
    }

    pub fn registry_tick(&self) -> Tick {
        self.registry_tick
    }

    pub fn row(&self, entity: Entity, component: TypeId) -> PropertyRow {
        PropertyRow {
            entity,
            component,
            path: self.path.clone(),
            type_id: self.type_id,
            editor_type: self.editor_type,
            registry_tick: self.registry_tick,
        }
    }
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
        if row.editor_type.is_none() {
            return Err(EditError::StaleEditor);
        }
        if row.type_id != TypeId::of::<T>() {
            return Err(EditError::TypeMismatch);
        }
        Self::erased::<E>(row, edit)
    }

    /// An edit from editor `E` to a row whose value type is known only by id.
    pub(crate) fn erased<E: 'static>(
        row: &PropertyRow,
        edit: impl Any + Send + Sync,
    ) -> Result<Self, EditError> {
        if row.editor_type.is_none() {
            return Err(EditError::StaleEditor);
        }
        if row.editor_type != Some(TypeId::of::<E>()) {
            return Err(EditError::TypeMismatch);
        }
        Ok(Self {
            row: row.clone(),
            edit: Arc::new(edit),
        })
    }
}

/// Edits waiting to be checked against the widget registry and sent to the scene.
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
}
