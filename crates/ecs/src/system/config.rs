use crate::system::{BoxedSystem, IntoSystem};

/// A system bundled with explicit ordering constraints.
///
/// Created by calling [`.after()`](IntoSystemConfig::after) or
/// [`.before()`](IntoSystemConfig::before) on any system function, and passed to
/// [`Schedule::add_system`](crate::system::schedule::Schedule::add_system).
///
/// Owned dep systems are registered into the schedule before the main system when
/// [`Schedule::add_system`](crate::system::schedule::Schedule::add_system) is called.
///
/// # Example
/// ```
/// use concerto_ecs::{Schedule, IntoSystemConfig};
///
/// fn system_a() {}
/// fn system_b() {}
/// fn system_c() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(system_a.after(system_b).before(system_c));
/// ```
pub struct SystemConfig {
    pub(crate) system: BoxedSystem,
    /// Systems that must run *before* this one. Each is registered into the schedule
    /// when this config is added, and a `dep → this` edge is inserted in the graph.
    pub(crate) after: Vec<SystemConfig>,
    /// Systems that must run *after* this one. Each is registered into the schedule
    /// when this config is added, and a `this → dep` edge is inserted in the graph.
    pub(crate) before: Vec<SystemConfig>,
}

impl SystemConfig {
    /// Declares that `dep` must run before this system.
    ///
    /// `dep` is owned by this config and will be automatically registered into the
    /// schedule alongside this system.
    pub fn after<M>(mut self, dep: impl IntoSystemConfig<M>) -> Self {
        self.after.push(dep.into_config());
        self
    }

    /// Declares that `dep` must run after this system.
    ///
    /// `dep` is owned by this config and will be automatically registered into the
    /// schedule alongside this system.
    pub fn before<M>(mut self, dep: impl IntoSystemConfig<M>) -> Self {
        self.before.push(dep.into_config());
        self
    }
}

/// Converts a system function or [`SystemConfig`] into a [`SystemConfig`].
///
/// Implemented for all functions whose parameters implement
/// [`SystemInput`](crate::system::system_input::SystemInput), and for
/// [`SystemConfig`] itself (passthrough).
///
/// The default `.after()` and `.before()` methods allow fluent chaining:
///
/// ```
/// # use concerto_ecs::{Schedule, IntoSystemConfig};
/// # fn a() {} fn b() {} fn c() {}
/// let mut schedule = Schedule::new();
/// schedule.add_system(a.after(b).before(c));
/// ```
pub trait IntoSystemConfig<Marker>: Sized {
    /// Wraps `self` into a [`SystemConfig`].
    fn into_config(self) -> SystemConfig;

    /// Declares that `dep` must run before this system.
    fn after<M>(self, dep: impl IntoSystemConfig<M>) -> SystemConfig {
        self.into_config().after(dep)
    }

    /// Declares that `dep` must run after this system.
    fn before<M>(self, dep: impl IntoSystemConfig<M>) -> SystemConfig {
        self.into_config().before(dep)
    }
}

/// Blanket impl: any function that implements [`IntoSystem`] can be turned into a [`SystemConfig`].
impl<M, F: IntoSystem<M> + 'static> IntoSystemConfig<M> for F {
    fn into_config(self) -> SystemConfig {
        SystemConfig {
            system: self.into_system(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}

/// Marker type used to implement [`IntoSystemConfig`] for [`SystemConfig`] itself,
/// allowing already-configured systems to be passed anywhere a config is expected
/// (e.g. as a dep in `.after(other_system.after(dep))`).
pub struct AlreadyConfigured;

impl IntoSystemConfig<AlreadyConfigured> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}
