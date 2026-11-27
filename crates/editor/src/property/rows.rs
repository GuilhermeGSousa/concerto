use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use concerto_ecs::{
    Component, Resource,
    command::EntityCommandQueue,
    component::{Tick, bundle::IntoBundle},
};
use concerto_foundation::assets::{asset_server::AssetServer, registry::AssetRegistry};
use concerto_reflect::{PropertyPath, Reflect};
use concerto_ui::elements::prelude::*;
use concerto_ui::{node::UINode, theme::UITheme};

use super::target::{PropertyAddress, PropertyTarget, TargetWorld};

/// Why a property edit failed.
///
/// A rejected edit leaves the target unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The editor refused the value, such as an asset of the wrong kind or a number that is not
    /// finite.
    Rejected,
    /// The value or the edit is not of the types the editor handles.
    TypeMismatch,
    /// No property is at the edit's path.
    NotFound,
    /// The editor registered for the property changed since its row was built.
    StaleEditor,
    /// The edited entity, component or document no longer exists.
    MissingTarget,
    /// The edited component's type is not in the type registry.
    UnregisteredComponent,
    /// The target world has no asset server or asset registry.
    MissingAssets,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rejected => "the value was refused",
            Self::TypeMismatch => "the value has the wrong type",
            Self::NotFound => "the property was not found",
            Self::StaleEditor => "its editor changed since it was read",
            Self::MissingTarget => "its entity, component or document no longer exists",
            Self::UnregisteredComponent => "the component type is not registered",
            Self::MissingAssets => "its App has no asset server or asset registry",
        })
    }
}
impl std::error::Error for EditError {}

/// What an edit can read from the world it changes.
pub struct EditContext<'w> {
    /// The asset server of the world being edited.
    pub assets: &'w AssetServer,
    /// The asset registry of the world being edited, for looking up assets by id.
    pub registry: &'w AssetRegistry,
}

/// A row an editor built.
#[derive(Component, Clone)]
pub struct PropertyRow {
    /// The property the row shows and edits.
    pub address: PropertyAddress,
    pub(crate) editor_type: TypeId,
    pub(crate) registry_tick: Tick,
}

impl PropertyRow {
    /// The registry's change tick at collection, which later edits are checked against.
    pub fn registry_tick(&self) -> Tick {
        self.registry_tick
    }
}

/// The snapshot of a list's own row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ListLength(pub(crate) usize);

/// The layout node every property row starts from.
pub fn standard_row_node(theme: &UITheme) -> UINode {
    theme.row().fixed().gap(theme.spacing_xs).into_bundle()
}

/// Presentation and validated editing for a concrete Rust type.
pub trait PropertyEditor<T: Reflect>: Default + Send + Sync + 'static {
    /// What the editor reads from a value to build and refresh its widgets.
    type Snapshot: Clone + PartialEq + Send + Sync + 'static;
    /// One change the editor's widgets queue, applied by [`Self::apply`].
    type Edit: Send + Sync + 'static;

    /// Reads `value` into the snapshot its widgets show.
    fn snapshot(&self, value: &T) -> Self::Snapshot;
    /// The layout node of the row that holds this editor's widgets.
    ///
    /// It is read once, when the row is spawned, so an editor whose node depends on the
    /// snapshot must return `false` from [`Self::follows_snapshot`].
    fn row_node(&self, _snapshot: &Self::Snapshot, theme: &UITheme) -> UINode {
        standard_row_node(theme)
    }
    /// Spawns the editor's widgets under `row`, showing `snapshot`.
    fn build(&self, row: &mut EntityCommandQueue, snapshot: &Self::Snapshot, theme: &UITheme);
    /// Applies `edit` to `value`, validating it before mutating.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when the edit is not valid for `value`; the value must then
    /// be left unchanged.
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
    /// The property's path inside its target.
    pub path: PropertyPath,
    /// The type of the value at `path`.
    pub type_id: TypeId,
    /// What the row shows.
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
    /// The property's path inside its target.
    pub path: PropertyPath,
    /// The type of the value at `path`.
    pub type_id: TypeId,
}

/// An owned edit and its captured target. Construct through [`Self::new`].
pub struct PropertyCommit {
    /// The row the edit came from, which names the property it changes.
    pub row: PropertyRow,
    pub(crate) edit: Arc<dyn Any + Send + Sync>,
}

impl PropertyCommit {
    /// The edit `edit` from editor `E` to `row`, whose value is a `T`.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `row`'s value is not a `T` or its editor is not
    /// `E`.
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
    /// Queues the edit `edit` from editor `E` to `row`, whose value is a `T`.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::TypeMismatch`] when `row`'s value is not a `T` or its editor is not
    /// `E`.
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
