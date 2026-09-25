use typle::typle;

use crate::{
    define_label,
    intern::Interned,
    system::config::{DependencyTarget, IntoDependencyTarget},
};

pub use concerto_ecs_macros::SystemSet;

define_label!(
    /// A named group of systems that can be ordered as a unit.
    ///
    /// Implement it with `#[derive(SystemSet)]`, which requires `Clone`, `Eq`, `Hash`
    /// and `Debug`. Systems join a set with
    /// [`in_set`](crate::IntoSystemConfig::in_set), sets are ordered with
    /// [`Schedule::configure_sets`](crate::Schedule::configure_sets), and a set can be
    /// the target of [`after`](crate::IntoSystemConfig::after) and
    /// [`before`](crate::IntoSystemConfig::before). Sets are flat: a set cannot
    /// contain another set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSetConfigs, IntoSystemConfig, Schedule, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Frame {
    ///     Input,
    ///     Simulate,
    /// }
    ///
    /// fn poll_gamepad() {}
    /// fn integrate() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule
    ///     .configure_sets((Frame::Input, Frame::Simulate).chain())
    ///     .add_system(integrate.in_set(Frame::Simulate))
    ///     .add_system(poll_gamepad.in_set(Frame::Input));
    /// ```
    SystemSet
);

/// A cheap, copyable handle to a [`SystemSet`], compared by identity.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{SystemSet, system::InternedSystemSet};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// struct Physics;
///
/// let set: InternedSystemSet = Physics.intern();
/// assert_eq!(set, Physics.intern());
/// ```
pub type InternedSystemSet = Interned<dyn SystemSet>;

/// A [`SystemSet`] together with its ordering constraints.
///
/// Created by calling [`after`](IntoSetConfig::after) or
/// [`before`](IntoSetConfig::before) on a set, and passed to
/// [`Schedule::configure_sets`](crate::Schedule::configure_sets).
///
/// # Examples
///
/// ```
/// use concerto_ecs::{Schedule, SystemSet, system::IntoSetConfig};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// enum Frame {
///     Input,
///     Simulate,
///     Present,
/// }
///
/// let mut schedule = Schedule::new();
/// schedule.configure_sets(Frame::Simulate.after(Frame::Input).before(Frame::Present));
/// ```
pub struct SetConfig {
    pub(crate) set: InternedSystemSet,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

impl SetConfig {
    /// Orders every member of this set after `target`, a system or a set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{Schedule, SystemSet, system::IntoSetConfig};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Physics;
    ///
    /// fn read_input() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_system(read_input).configure_sets(Physics.after(read_input));
    /// ```
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Orders every member of this set before `target`, a system or a set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{Schedule, SystemSet, system::IntoSetConfig};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// struct Physics;
    ///
    /// fn render() {}
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.add_system(render).configure_sets(Physics.before(render));
    /// ```
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }
}

/// Converts a [`SystemSet`] or [`SetConfig`] into a [`SetConfig`].
///
/// The provided methods start a configuration from a bare set.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{Schedule, SystemSet, system::IntoSetConfig};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// enum Frame {
///     Input,
///     Simulate,
/// }
///
/// let mut schedule = Schedule::new();
/// schedule.configure_sets(Frame::Simulate.after(Frame::Input));
/// ```
pub trait IntoSetConfig: Sized {
    /// Wraps `self` into a [`SetConfig`] with no constraints.
    fn into_set_config(self) -> SetConfig;

    /// Orders every member of this set after `target`. See [`SetConfig::after`].
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().after(target)
    }

    /// Orders every member of this set before `target`. See [`SetConfig::before`].
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().before(target)
    }
}

impl<S: SystemSet> IntoSetConfig for S {
    fn into_set_config(self) -> SetConfig {
        SetConfig {
            set: self.intern(),
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}

impl IntoSetConfig for SetConfig {
    fn into_set_config(self) -> SetConfig {
        self
    }
}

/// One or more [`SetConfig`]s, optionally chained into a sequence.
///
/// Implemented for any [`SystemSet`] or [`SetConfig`], for `Vec<SetConfig>` and for
/// tuples of up to twelve sets or set configs.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSetConfigs, Schedule, SystemSet};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// enum Frame {
///     Input,
///     Simulate,
/// }
///
/// let mut schedule = Schedule::new();
/// schedule.configure_sets((Frame::Input, Frame::Simulate));
/// ```
pub trait IntoSetConfigs {
    /// Returns one [`SetConfig`] per set, in order.
    fn into_set_configs(self) -> Vec<SetConfig>;

    /// Orders the sets so each runs before the next.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSetConfigs, Schedule, SystemSet};
    ///
    /// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
    /// enum Frame {
    ///     Input,
    ///     Simulate,
    ///     Present,
    /// }
    ///
    /// let mut schedule = Schedule::new();
    /// schedule.configure_sets((Frame::Input, Frame::Simulate, Frame::Present).chain());
    /// ```
    fn chain(self) -> Vec<SetConfig>
    where
        Self: Sized,
    {
        let mut configs = self.into_set_configs();
        for index in 1..configs.len() {
            let previous = configs[index - 1].set;
            configs[index].after.push(DependencyTarget::Set(previous));
        }
        configs
    }
}

impl<S: IntoSetConfig> IntoSetConfigs for S {
    fn into_set_configs(self) -> Vec<SetConfig> {
        vec![self.into_set_config()]
    }
}

impl IntoSetConfigs for Vec<SetConfig> {
    fn into_set_configs(self) -> Vec<SetConfig> {
        self
    }
}

#[allow(unused_mut)]
#[typle(Tuple for 0..=12)]
impl<T> IntoSetConfigs for T
where
    T: Tuple,
    T<_>: IntoSetConfig,
{
    fn into_set_configs(self) -> Vec<SetConfig> {
        let mut configs = Vec::with_capacity(T::LEN);
        for typle_index!(i) in 0..T::LEN {
            configs.push(self[[i]].into_set_config());
        }
        configs
    }
}
