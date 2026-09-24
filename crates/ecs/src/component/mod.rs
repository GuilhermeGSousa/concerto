use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
};

pub mod bundle;
pub mod name;
pub mod registry;
pub mod scene;

pub use concerto_ecs_macros::Component;

use crate::{entity::Entity, world::RestrictedWorld};

/// A unique identifier for a component type, based on its Rust [`TypeId`].
pub type ComponentId = TypeId;

/// Callback invoked when a component is added to or removed from an entity.
///
/// The callback receives a [`RestrictedWorld`] so it can safely react to the
/// lifecycle event (e.g. queueing insertion or removal of companion components).
/// Structural commands run after the enclosing operation's hook pass and storage
/// update; existing component and resource data can be mutated immediately.
pub type ComponentLifecycleCallback = for<'w> fn(RestrictedWorld<'w>, ComponentLifecycleContext);

/// Context passed to a [`ComponentLifecycleCallback`].
pub struct ComponentLifecycleContext {
    /// The entity whose component triggered the callback.
    pub entity: Entity,
}

/// Marker trait for data that can be attached to an [`Entity`](crate::entity::Entity).
///
/// Implement this trait (or derive it with `#[derive(Component)]`) for any type that
/// should live in the ECS storage.
///
/// # Lifecycle callbacks
/// Override [`on_add`](Component::on_add), [`on_replace`](Component::on_replace),
/// [`on_remove`](Component::on_remove), or [`on_despawn`](Component::on_despawn) to
/// react to component insertion, an insert overwriting an existing value, component
/// removal (including the removal a despawn implies), or the despawn itself.
///
/// # Example
/// ```
/// use concerto_ecs::component::Component;
///
/// #[derive(Component)]
/// struct Velocity {
///     x: f32,
///     y: f32,
/// }
/// ```
pub trait Component: Send + Sync + 'static {
    fn name() -> &'static str
    where
        Self: Sized,
    {
        std::any::type_name::<Self>()
    }

    /// Optional callback invoked immediately after this component is added to an entity.
    fn on_add() -> Option<ComponentLifecycleCallback> {
        None
    }

    /// Optional callback invoked before an insert overwrites this component, while the old
    /// value is still readable. This callback is not invoked by the first insert, by explicit
    /// removal, or by despawning the entity.
    fn on_replace() -> Option<ComponentLifecycleCallback> {
        None
    }

    /// Optional callback invoked before this component is removed, while its data is
    /// readable. Despawning removes every component, so this fires then too; replacement
    /// does not fire it.
    /// Structural changes requested by the callback are queued and applied after
    /// the removal completes; the callback cannot invalidate component storage.
    fn on_remove() -> Option<ComponentLifecycleCallback> {
        None
    }

    /// Optional callback for teardown that only makes sense for a dying entity, invoked
    /// after this component's [`on_remove`](Component::on_remove) during a despawn and not
    /// at all on explicit component removal.
    /// All components remain present throughout the despawn hook pass. Structural
    /// commands queued by callbacks run after the entity has been dropped.
    fn on_despawn() -> Option<ComponentLifecycleCallback> {
        None
    }
}

#[allow(dead_code)]
pub(crate) struct ComponentLifecycleCallbacks {
    pub(crate) on_add: Option<ComponentLifecycleCallback>,
    pub(crate) on_replace: Option<ComponentLifecycleCallback>,
    pub(crate) on_remove: Option<ComponentLifecycleCallback>,
    pub(crate) on_despawn: Option<ComponentLifecycleCallback>,
}

impl ComponentLifecycleCallbacks {
    pub(crate) fn from_component<T: Component>() -> Self {
        Self {
            on_add: T::on_add(),
            on_replace: T::on_replace(),
            on_remove: T::on_remove(),
            on_despawn: T::on_despawn(),
        }
    }
}

/// A monotonically-increasing frame counter used for change detection.
///
/// Each component and resource stores the tick at which it was last added or
/// mutated.  Filters like [`Added`](crate::query::query_filter::Added) and
/// [`Changed`](crate::query::query_filter::Changed) compare the stored tick
/// against the world's current tick to decide whether to include an entity.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tick(u32);

impl Tick {
    /// Creates a new `Tick` with the given counter value.
    pub fn new(tick: u32) -> Self {
        Self(tick)
    }

    /// Sets the tick value.
    pub fn set(&mut self, tick: u32) {
        self.0 = tick;
    }
}

impl Deref for Tick {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Tick {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
