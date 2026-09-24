use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{Component, Entity, Resource, command::CommandQueue, component::Tick};
use concerto_editable::{Editable, PropertyPath};
use concerto_ui::theme::UITheme;

/// A rejected edit leaves the target unchanged. Type mismatches and stale
/// registrations are reported without invoking the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    Rejected,
    TypeMismatch,
    NotFound,
    StaleEditor,
    MissingTarget,
    UnregisteredComponent,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EditError {}

/// Presentation and validated editing for a concrete Rust type.
///
/// Register widget systems in `LateUpdate` after `InspectorPlugin`. They read
/// [`PropertyRowValue::snapshot`] and queue edits with [`PropertyCommits::push`].
/// Commits apply in the next `Update`, before transform propagation.
/// Per-row UI state belongs on entities spawned by `build`, not on this adapter.
pub trait PropertyEditor<T: Editable>: Send + Sync + 'static {
    type Snapshot: Clone + PartialEq + Send + Sync + 'static;
    type Edit: Send + Sync + 'static;

    fn snapshot(&self, value: &T) -> Self::Snapshot;
    fn build(
        &self,
        cmd: &mut CommandQueue,
        row: Entity,
        snapshot: &Self::Snapshot,
        theme: &UITheme,
    );
    /// Validate before mutating. On error the target must remain unchanged.
    fn apply(&self, value: &mut T, edit: &Self::Edit) -> Result<(), EditError>;
}

/// Captured at rebuild, so a queued edit always addresses its original target.
#[derive(Component, Clone, Debug)]
pub struct PropertyRow {
    pub entity: Entity,
    pub component: TypeId,
    pub path: PropertyPath,
    pub type_id: TypeId,
    pub(crate) editor_type: Option<TypeId>,
    pub(crate) registry_tick: Tick,
}

impl PropertyRow {
    /// Whether an adapter claimed this row when it was collected.
    pub fn has_editor(&self) -> bool {
        self.editor_type.is_some()
    }

    /// The registry's change tick at collection, which later edits are checked against.
    pub fn registry_tick(&self) -> Tick {
        self.registry_tick
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
/// Unsupported rows have no snapshot.
#[derive(Component, Clone, Default)]
pub struct PropertyRowValue(pub(crate) Option<Snapshot>);

impl PropertyRowValue {
    pub(crate) fn new<T: Editable, E: PropertyEditor<T>>(value: E::Snapshot) -> Self {
        Self(Some(Snapshot {
            data: Arc::new(value),
            value_type: TypeId::of::<T>(),
            editor_type: TypeId::of::<E>(),
            equals: |a, b| match (
                a.downcast_ref::<E::Snapshot>(),
                b.downcast_ref::<E::Snapshot>(),
            ) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }))
    }

    /// Returns `None` for an unsupported row or a different value/adapter type.
    pub fn snapshot<T: Editable, E: PropertyEditor<T>>(&self) -> Option<&E::Snapshot> {
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
    pub(crate) registry_tick: Tick,
}

impl Property {
    /// Whether an adapter claimed this property.
    pub fn has_editor(&self) -> bool {
        self.editor_type.is_some()
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
    pub(crate) edit: Box<dyn Any + Send + Sync>,
}

impl PropertyCommit {
    pub fn new<T: Editable, E: PropertyEditor<T>>(
        row: &PropertyRow,
        edit: E::Edit,
    ) -> Result<Self, EditError> {
        if row.editor_type.is_none() {
            return Err(EditError::StaleEditor);
        }
        if row.type_id != TypeId::of::<T>() || row.editor_type != Some(TypeId::of::<E>()) {
            return Err(EditError::TypeMismatch);
        }
        Ok(Self {
            row: row.clone(),
            edit: Box::new(edit),
        })
    }
}

/// Edits awaiting the inspector's exclusive applying system.
#[derive(Resource, Default)]
pub struct PropertyCommits(pub Vec<PropertyCommit>);

impl PropertyCommits {
    pub fn push<T: Editable, E: PropertyEditor<T>>(
        &mut self,
        row: &PropertyRow,
        edit: E::Edit,
    ) -> Result<(), EditError> {
        self.0.push(PropertyCommit::new::<T, E>(row, edit)?);
        Ok(())
    }
}
