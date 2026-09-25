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

/// One system or a group of systems, with their set memberships and ordering
/// constraints.
///
/// Created by [`IntoSystemConfig::into_config`], usually implicitly by calling
/// [`after`](IntoSystemConfig::after), [`before`](IntoSystemConfig::before),
/// [`in_set`](IntoSystemConfig::in_set) or [`chain`](IntoSystemConfig::chain) on a
/// system or a tuple of systems, and passed to
/// [`Schedule::add_system`](crate::Schedule::add_system).
///
/// Ordering targets are references: `a.after(b)` orders `a` after the `b` registered
/// in the same schedule and does not register `b`. A target that is not in the
/// schedule is ignored with a warning when the schedule compiles. Constraints on a
/// group apply to every system in it.
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
pub struct SystemConfig(SystemNode);

impl SystemConfig {
    /// Orders every system in this config after `target`, a system or a [`SystemSet`].
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
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.config.after.push(target));
        self
    }

    /// Orders every system in this config before `target`, a system or a [`SystemSet`].
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
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.config.before.push(target));
        self
    }

    /// Adds every system in this config to `set`. A system may belong to several sets.
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
    /// fn collide() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .add_system(integrate.in_set(Phase::Simulate).in_set(Phase::Debug))
    ///     .add_system((integrate, collide).in_set(Phase::Simulate));
    /// ```
    pub fn in_set(mut self, set: impl SystemSet) -> Self {
        let set = set.intern();
        self.for_each_entry(&mut |entry| entry.config.sets.push(set));
        self
    }

    /// Orders the children of a group so each runs before the next.
    ///
    /// When a child is itself a group, every system in it runs before every system in
    /// the next child. Does nothing on a single system.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSystemConfig, Schedule};
    ///
    /// fn read_input() {}
    /// fn move_player() {}
    /// fn move_enemies() {}
    /// fn update_camera() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_system((read_input, (move_player, move_enemies), update_camera).chain());
    /// ```
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

/// Converts a system, a [`SystemConfig`], or a group of them into a [`SystemConfig`].
///
/// Implemented for every function whose parameters implement
/// [`SystemInput`](crate::system::input::SystemInput), for [`SystemConfig`], for
/// `Vec<SystemConfig>` and for tuples of up to twelve elements that implement this
/// trait. The provided methods start a configuration from any of them.
///
/// A `Vec` holds converted configs rather than bare functions because a `Vec` of
/// functions coerces them to one function-pointer type, and systems that share a
/// type cannot be told apart as ordering targets.
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
/// schedule.add_system(a).add_system((b, c).after(a));
/// ```
pub trait IntoSystemConfig<Marker>: Sized {
    /// Wraps `self` into a [`SystemConfig`].
    fn into_config(self) -> SystemConfig;

    /// Orders every system after `target`. See [`SystemConfig::after`].
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().after(target)
    }

    /// Orders every system before `target`. See [`SystemConfig::before`].
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SystemConfig {
        self.into_config().before(target)
    }

    /// Adds every system to `set`. See [`SystemConfig::in_set`].
    fn in_set(self, set: impl SystemSet) -> SystemConfig {
        self.into_config().in_set(set)
    }

    /// Orders the elements so each runs before the next. See [`SystemConfig::chain`].
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
