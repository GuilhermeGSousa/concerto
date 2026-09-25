use std::any::TypeId;

use typle::typle;

use crate::system::{
    BoxedSystem, IntoSystem,
    set::{InternedSystemSet, SystemSet},
};

/// Something a system can be ordered against: another system, or a [`SystemSet`].
///
/// Built with [`IntoDependencyTarget::into_target`], usually implicitly by
/// [`IntoSystemConfig::after`] and [`IntoSystemConfig::before`].
///
/// # Examples
///
/// ```
/// use concerto_ecs::{SystemSet, system::{DependencyTarget, IntoDependencyTarget}};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// struct Physics;
///
/// fn step() {}
///
/// assert!(matches!(step.into_target(), DependencyTarget::System { .. }));
/// assert_eq!(Physics.into_target(), DependencyTarget::Set(Physics.intern()));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DependencyTarget {
    /// Every system of this type registered in the same schedule.
    System {
        /// The [`System::system_type`](crate::System::system_type) of the target.
        id: TypeId,
        /// The target's [`System::name`](crate::System::name), used in diagnostics.
        name: &'static str,
    },
    /// Every member of this set in the same schedule.
    Set(InternedSystemSet),
}

#[doc(hidden)]
pub struct SystemTarget<M>(M);

#[doc(hidden)]
pub struct SetTarget;

/// Converts a system function or a [`SystemSet`] into a [`DependencyTarget`].
///
/// A system target is identified by its type, so it refers to every copy of that
/// system registered in the same schedule and never registers the system itself.
/// Closures can be converted but never match anything, since no other system
/// shares their type.
///
/// # Examples
///
/// ```
/// use concerto_ecs::system::{DependencyTarget, IntoDependencyTarget};
///
/// fn step() {}
///
/// assert_eq!(step.into_target(), step.into_target());
/// ```
pub trait IntoDependencyTarget<Marker> {
    /// Returns the target this value refers to.
    fn into_target(self) -> DependencyTarget;
}

impl<M, S: IntoSystem<M> + 'static> IntoDependencyTarget<SystemTarget<M>> for S {
    fn into_target(self) -> DependencyTarget {
        let system = self.into_system();
        DependencyTarget::System {
            id: system.system_type(),
            name: system.name(),
        }
    }
}

impl<S: SystemSet> IntoDependencyTarget<SetTarget> for S {
    fn into_target(self) -> DependencyTarget {
        DependencyTarget::Set(self.intern())
    }
}

/// A system bundled with its set memberships and ordering constraints.
///
/// Created by calling [`after`](IntoSystemConfig::after),
/// [`before`](IntoSystemConfig::before) or [`in_set`](IntoSystemConfig::in_set)
/// on a system function, and passed to
/// [`Schedule::add_system`](crate::Schedule::add_system).
///
/// Ordering targets are references: `a.after(b)` orders `a` after the `b` registered
/// in the same schedule and does not register `b`. A target that is not in the
/// schedule is ignored with a warning when the schedule compiles.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystemConfig, Schedule};
///
/// fn system_a() {}
/// fn system_b() {}
/// fn system_c() {}
///
/// let mut schedule = Schedule::new();
/// schedule
///     .add_system(system_b)
///     .add_system(system_c)
///     .add_system(system_a.after(system_b).before(system_c));
/// ```
pub struct SystemConfig {
    pub(crate) system: BoxedSystem,
    pub(crate) sets: Vec<InternedSystemSet>,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

impl SystemConfig {
    /// Orders this system after `target`, a system or a [`SystemSet`].
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfig, Schedule};
    ///
    /// fn load() {}
    /// fn spawn() {}
    /// fn animate() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .add_system(load)
    ///     .add_system(spawn)
    ///     .add_system(animate.after(load).after(spawn));
    /// ```
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Orders this system before `target`, a system or a [`SystemSet`].
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfig, Schedule};
    ///
    /// fn input() {}
    /// fn movement() {}
    /// fn render() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .add_system(movement)
    ///     .add_system(render)
    ///     .add_system(input.before(movement).before(render));
    /// ```
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }

    /// Adds this system to `set`. A system may belong to several sets.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfig, Schedule, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Phase {
    ///     Simulate,
    ///     Debug,
    /// }
    ///
    /// fn integrate() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_system(integrate.in_set(Phase::Simulate).in_set(Phase::Debug));
    /// ```
    pub fn in_set(mut self, set: impl SystemSet) -> Self {
        self.sets.push(set.intern());
        self
    }
}

/// Converts a system function or [`SystemConfig`] into a [`SystemConfig`].
///
/// Implemented for every function whose parameters implement
/// [`SystemInput`](crate::system::input::SystemInput), and for [`SystemConfig`]
/// itself. The provided methods start a configuration from a bare function.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystemConfig, Schedule};
///
/// fn a() {}
/// fn b() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(a).add_system(b.after(a));
/// ```
pub trait IntoSystemConfig<Marker>: Sized {
    /// Wraps `self` into a [`SystemConfig`] with no constraints.
    fn into_config(self) -> SystemConfig;

    /// Orders this system after `target`. See [`SystemConfig::after`].
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().after(target)
    }

    /// Orders this system before `target`. See [`SystemConfig::before`].
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().before(target)
    }

    /// Adds this system to `set`. See [`SystemConfig::in_set`].
    fn in_set(self, set: impl SystemSet) -> SystemConfig {
        self.into_config().in_set(set)
    }
}

impl<M, F: IntoSystem<M> + 'static> IntoSystemConfig<M> for F {
    fn into_config(self) -> SystemConfig {
        SystemConfig {
            system: self.into_system(),
            sets: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}

#[doc(hidden)]
pub struct AlreadyConfigured;

impl IntoSystemConfig<AlreadyConfigured> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}

/// Several systems, optionally grouped into a set or chained in order.
///
/// Implemented for tuples of up to twelve systems or configs, for
/// `Vec<SystemConfig>` and for [`SystemConfig`]. A lone system function goes
/// through [`IntoSystemConfig`] instead.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystemConfigs, Schedule};
///
/// fn read_input() {}
/// fn move_player() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_systems((read_input, move_player));
/// ```
pub trait IntoSystemConfigs<Marker> {
    /// Returns one [`SystemConfig`] per system, in order.
    fn into_configs(self) -> Vec<SystemConfig>;

    /// Adds every system to `set`.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfigs, Schedule, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Gameplay;
    ///
    /// fn spawn_enemies() {}
    /// fn update_score() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_systems((spawn_enemies, update_score).in_set(Gameplay));
    /// ```
    fn in_set(self, set: impl SystemSet) -> Vec<SystemConfig>
    where
        Self: Sized,
    {
        let interned = set.intern();
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.sets.push(interned);
        }
        configs
    }

    /// Orders the systems so each runs before the next.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfigs, Schedule};
    ///
    /// fn read_input() {}
    /// fn move_player() {}
    /// fn update_camera() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_systems((read_input, move_player, update_camera).chain());
    /// ```
    fn chain(self) -> Vec<SystemConfig>
    where
        Self: Sized,
    {
        let mut configs = self.into_configs();
        for index in 1..configs.len() {
            let previous = &configs[index - 1].system;
            let previous = DependencyTarget::System {
                id: previous.system_type(),
                name: previous.name(),
            };
            configs[index].after.push(previous);
        }
        configs
    }
}

#[doc(hidden)]
pub struct SingleConfig;

impl IntoSystemConfigs<SingleConfig> for SystemConfig {
    fn into_configs(self) -> Vec<SystemConfig> {
        vec![self]
    }
}

#[doc(hidden)]
pub struct ConfigVec;

impl IntoSystemConfigs<ConfigVec> for Vec<SystemConfig> {
    fn into_configs(self) -> Vec<SystemConfig> {
        self
    }
}

#[allow(unused_mut)]
#[typle(Tuple for 0..=12)]
impl<T, M> IntoSystemConfigs<M> for T
where
    T: Tuple,
    M: Tuple,
    typle_bound!(i in .. => T<{i}>): IntoSystemConfig<M<{ i }>>,
{
    fn into_configs(self) -> Vec<SystemConfig> {
        let mut configs = Vec::with_capacity(T::LEN);
        for typle_index!(i) in 0..T::LEN {
            configs.push(self[[i]].into_config());
        }
        configs
    }
}
