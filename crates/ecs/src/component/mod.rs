use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
};

pub mod bundle;
pub mod name;
pub mod scene;

pub use concerto_ecs_macros::Component;

use crate::{
    entity::Entity,
    signal::{Signal, lifecycle},
    world::{RestrictedWorld, UnsafeWorldCell},
};

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
    pub(crate) signal_add: LifecycleDispatch,
    pub(crate) signal_replace: LifecycleDispatch,
    pub(crate) signal_remove: LifecycleDispatch,
    pub(crate) signal_despawn: LifecycleDispatch,
}

pub(crate) type LifecycleDispatch = unsafe fn(UnsafeWorldCell, Entity);

impl ComponentLifecycleCallbacks {
    pub(crate) fn from_component<T: Component>() -> Self {
        Self {
            on_add: T::on_add(),
            on_replace: T::on_replace(),
            on_remove: T::on_remove(),
            on_despawn: T::on_despawn(),
            signal_add: lifecycle::dispatch::<lifecycle::Add<T>>,
            signal_replace: lifecycle::dispatch::<lifecycle::Replace<T>>,
            signal_remove: lifecycle::dispatch::<lifecycle::Remove<T>>,
            signal_despawn: lifecycle::dispatch::<lifecycle::Despawn<T>>,
        }
    }
}

/// How many counter advances may pass before stored ticks are clamped again.
pub const CHECK_TICK_THRESHOLD: u32 = 518_400_000;

/// The oldest age a stored tick keeps; older ones are clamped to it.
pub const MAX_CHANGE_AGE: u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1);

/// A value of a world's change counter, stamped on writes and handed to each system run.
///
/// Each component and resource stores the tick at which it was last added or
/// mutated.  Filters like [`Added`](crate::query::filter::Added) and
/// [`Changed`](crate::query::filter::Changed) compare the stored tick against
/// the [`SystemTicks`] of the run reading it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tick(u32);

impl Tick {
    /// The tick [`MAX_CHANGE_AGE`] ticks old, relative to zero.
    pub const MAX: Self = Self(MAX_CHANGE_AGE);

    /// Creates a new `Tick` with the given counter value.
    pub const fn new(tick: u32) -> Self {
        Self(tick)
    }

    /// Sets the tick value.
    pub fn set(&mut self, tick: u32) {
        self.0 = tick;
    }

    /// How many ticks `other` lies before `self`, with wrapping arithmetic.
    pub fn relative_to(self, other: Self) -> Self {
        Self(self.0.wrapping_sub(other.0))
    }

    /// Whether this stamp lies after `last_run` and no later than `this_run`.
    pub fn is_newer_than(self, last_run: Tick, this_run: Tick) -> bool {
        let since_stamp = this_run.relative_to(self).0.min(MAX_CHANGE_AGE);
        let since_last_run = this_run.relative_to(last_run).0.min(MAX_CHANGE_AGE);
        since_last_run > since_stamp
    }

    /// Clamps this tick to [`MAX_CHANGE_AGE`] before the check's present tick; returns whether it did.
    pub fn check_tick(&mut self, check: CheckChangeTicks) -> bool {
        let age = check.present_tick().relative_to(*self);
        if age.0 > Self::MAX.0 {
            *self = check.present_tick().relative_to(Self::MAX);
            true
        } else {
            false
        }
    }
}

/// A pass that clamps stored ticks, triggered as a signal so custom tick storage can clamp itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CheckChangeTicks(pub(crate) Tick);

impl CheckChangeTicks {
    /// The counter's value at the check, which stored ticks are compared to.
    pub fn present_tick(self) -> Tick {
        self.0
    }
}

impl Signal for CheckChangeTicks {}

/// The ticks of one system run: the previous run whose body executed, and this one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemTicks {
    /// The `this_run` of the previous run whose body executed.
    pub last_run: Tick,
    /// The tick taken from the world's counter when this run started.
    pub this_run: Tick,
}

impl SystemTicks {
    /// Whether `stamp` was written after `last_run` and no later than `this_run`.
    pub fn is_changed(self, stamp: Tick) -> bool {
        stamp.is_newer_than(self.last_run, self.this_run)
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
