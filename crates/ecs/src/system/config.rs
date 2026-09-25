use std::any::TypeId;

use crate::system::{
    BoxedSystem, IntoSystem,
    set::{InternedSystemSet, SystemSet},
};

/// Something a system can be ordered against: another system, or a [`SystemSet`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DependencyTarget {
    System { id: TypeId, name: &'static str },
    Set(InternedSystemSet),
}

/// Marker for the [`IntoDependencyTarget`] impl covering systems.
pub struct SystemTarget<M>(M);

/// Marker for the [`IntoDependencyTarget`] impl covering sets.
pub struct SetTarget;

/// Converts a system function or a [`SystemSet`] into a [`DependencyTarget`].
///
/// A system target is identified by its type, so it refers to whichever copy of that
/// system is registered in the same schedule; it never registers the system itself.
pub trait IntoDependencyTarget<Marker> {
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
/// Created by calling [`.after()`](IntoSystemConfig::after),
/// [`.before()`](IntoSystemConfig::before) or [`.in_set()`](IntoSystemConfig::in_set)
/// on any system function, and passed to
/// [`Schedule::add_system`](crate::system::schedule::Schedule::add_system).
///
/// Ordering targets are references: `a.after(b)` orders `a` after the `b` that is
/// registered in the same schedule, and does not register `b`.
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
    /// Declares that `target` must run before this system.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Declares that `target` must run after this system.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }

    /// Adds this system to `set`.
    pub fn in_set(mut self, set: impl SystemSet) -> Self {
        self.sets.push(set.intern());
        self
    }
}

/// Converts a system function or [`SystemConfig`] into a [`SystemConfig`].
///
/// Implemented for all functions whose parameters implement
/// [`SystemInput`](crate::system::input::SystemInput), and for
/// [`SystemConfig`] itself (passthrough).
pub trait IntoSystemConfig<Marker>: Sized {
    /// Wraps `self` into a [`SystemConfig`].
    fn into_config(self) -> SystemConfig;

    /// Declares that `target` must run before this system.
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().after(target)
    }

    /// Declares that `target` must run after this system.
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().before(target)
    }

    /// Adds this system to `set`.
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

/// Marker type used to implement [`IntoSystemConfig`] for [`SystemConfig`] itself.
pub struct AlreadyConfigured;

impl IntoSystemConfig<AlreadyConfigured> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}

/// Several systems, optionally grouped into a set or chained in order.
///
/// Implemented for tuples of systems, `Vec<SystemConfig>` and [`SystemConfig`].
/// A lone system function goes through [`IntoSystemConfig`] instead.
pub trait IntoSystemConfigs<Marker> {
    fn into_configs(self) -> Vec<SystemConfig>;

    /// Adds every system to `set`.
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

/// Marker for the single-[`SystemConfig`] [`IntoSystemConfigs`] impl.
pub struct SingleConfig;

impl IntoSystemConfigs<SingleConfig> for SystemConfig {
    fn into_configs(self) -> Vec<SystemConfig> {
        vec![self]
    }
}

/// Marker for the `Vec<SystemConfig>` [`IntoSystemConfigs`] impl.
pub struct ConfigVec;

impl IntoSystemConfigs<ConfigVec> for Vec<SystemConfig> {
    fn into_configs(self) -> Vec<SystemConfig> {
        self
    }
}

macro_rules! impl_into_system_configs_for_tuple {
    ($(($name:ident, $marker:ident)),*) => {
        impl<$($name, $marker),*> IntoSystemConfigs<($($marker,)*)> for ($($name,)*)
        where
            $($name: IntoSystemConfig<$marker>,)*
        {
            #[allow(non_snake_case)]
            fn into_configs(self) -> Vec<SystemConfig> {
                let ($($name,)*) = self;
                vec![$($name.into_config()),*]
            }
        }
    };
}

impl_into_system_configs_for_tuple!((A, MA), (B, MB));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME));
impl_into_system_configs_for_tuple!((A, MA), (B, MB), (C, MC), (D, MD), (E, ME), (F, MF));
impl_into_system_configs_for_tuple!(
    (A, MA),
    (B, MB),
    (C, MC),
    (D, MD),
    (E, ME),
    (F, MF),
    (G, MG)
);
impl_into_system_configs_for_tuple!(
    (A, MA),
    (B, MB),
    (C, MC),
    (D, MD),
    (E, ME),
    (F, MF),
    (G, MG),
    (H, MH)
);
