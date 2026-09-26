use std::any::TypeId;

use typle::typle;

use crate::system::{
    BoxedSystem, IntoSystem,
    set::{InternedSystemSet, SystemSet},
};

/// A system or [`SystemSet`] that another system can be ordered against.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DependencyTarget {
    /// Every system of this type in the same schedule.
    System { id: TypeId, name: &'static str },
    /// Every member of this set in the same schedule.
    Set(InternedSystemSet),
}

#[doc(hidden)]
pub struct SystemTarget<M>(M);

#[doc(hidden)]
pub struct SetTarget;

/// Converts a system function or a [`SystemSet`] into a [`DependencyTarget`].
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

#[derive(Default)]
pub(crate) struct NodeConfig {
    pub(crate) sets: Vec<InternedSystemSet>,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

pub(crate) struct SystemEntry {
    pub(crate) system: BoxedSystem,
    pub(crate) config: NodeConfig,
}

enum SystemNode {
    Single(SystemEntry),
    Group(Vec<SystemConfig>),
}

/// One system or a group of systems, with their set memberships and ordering constraints.
///
/// Ordering targets are references: `a.after(b)` orders `a` after the `b` registered in the
/// same schedule and does not register `b`. Constraints on a group apply to every system in it.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSystemConfig, Schedule};
///
/// fn a() {}
/// fn b() {}
/// fn c() {}
///
/// let mut schedule = Schedule::new();
/// schedule.add_system(a).add_system((b, c).chain().after(a));
/// ```
pub struct SystemConfig(SystemNode);

impl SystemConfig {
    /// Orders every system in this config after `target`.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.config.after.push(target));
        self
    }

    /// Orders every system in this config before `target`.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.config.before.push(target));
        self
    }

    /// Adds every system in this config to `set`.
    pub fn in_set(mut self, set: impl SystemSet) -> Self {
        let set = set.intern();
        self.for_each_entry(&mut |entry| entry.config.sets.push(set));
        self
    }

    /// Orders the children of a group so each runs before the next.
    ///
    /// A nested group is ordered as a unit: in `(a, (b, c), d).chain()`, `b` and `c` both run
    /// after `a` and before `d`.
    pub fn chain(mut self) -> Self {
        if let SystemNode::Group(children) = &mut self.0 {
            for index in 1..children.len() {
                let mut previous = Vec::new();
                children[index - 1].collect_targets(&mut previous);
                children[index].for_each_entry(&mut |entry| entry.config.after.extend(&previous));
            }
        }
        self
    }

    pub(crate) fn into_entries(self, entries: &mut Vec<SystemEntry>) {
        match self.0 {
            SystemNode::Single(entry) => entries.push(entry),
            SystemNode::Group(children) => {
                for child in children {
                    child.into_entries(entries);
                }
            }
        }
    }

    fn for_each_entry(&mut self, f: &mut impl FnMut(&mut SystemEntry)) {
        match &mut self.0 {
            SystemNode::Single(entry) => f(entry),
            SystemNode::Group(children) => {
                for child in children {
                    child.for_each_entry(f);
                }
            }
        }
    }

    fn collect_targets(&self, targets: &mut Vec<DependencyTarget>) {
        match &self.0 {
            SystemNode::Single(entry) => targets.push(DependencyTarget::System {
                id: entry.system.system_type(),
                name: entry.system.name(),
            }),
            SystemNode::Group(children) => {
                for child in children {
                    child.collect_targets(targets);
                }
            }
        }
    }
}

/// Converts a system, a [`SystemConfig`], or a tuple or `Vec<SystemConfig>` of them into a
/// [`SystemConfig`].
///
/// A `Vec` must hold configs rather than bare functions: collecting functions coerces them
/// to one function-pointer type, which cannot be told apart as an ordering target.
pub trait IntoSystemConfig<Marker>: Sized {
    fn into_config(self) -> SystemConfig;

    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().after(target)
    }

    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().before(target)
    }

    fn in_set(self, set: impl SystemSet) -> SystemConfig {
        self.into_config().in_set(set)
    }

    fn chain(self) -> SystemConfig {
        self.into_config().chain()
    }
}

impl<M, F: IntoSystem<M> + 'static> IntoSystemConfig<M> for F {
    fn into_config(self) -> SystemConfig {
        SystemConfig(SystemNode::Single(SystemEntry {
            system: self.into_system(),
            config: NodeConfig::default(),
        }))
    }
}

#[doc(hidden)]
pub struct AlreadyConfigured;

impl IntoSystemConfig<AlreadyConfigured> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}

#[doc(hidden)]
pub struct ConfigVec;

impl IntoSystemConfig<ConfigVec> for Vec<SystemConfig> {
    fn into_config(self) -> SystemConfig {
        SystemConfig(SystemNode::Group(self))
    }
}

#[doc(hidden)]
pub struct ConfigTuple<M>(M);

#[allow(unused_mut)]
#[typle(Tuple for 0..=12)]
impl<T, M> IntoSystemConfig<ConfigTuple<M>> for T
where
    T: Tuple,
    M: Tuple,
    typle_bound!(i in .. => T<{i}>): IntoSystemConfig<M<{ i }>>,
{
    fn into_config(self) -> SystemConfig {
        let mut children = Vec::with_capacity(T::LEN);
        for typle_index!(i) in 0..T::LEN {
            children.push(self[[i]].into_config());
        }
        SystemConfig(SystemNode::Group(children))
    }
}
