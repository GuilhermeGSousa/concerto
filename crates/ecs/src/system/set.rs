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
    /// use concerto_ecs::{IntoSetConfig, IntoSystemConfig, Schedule, SystemSet};
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

pub(crate) struct SetEntry {
    pub(crate) set: InternedSystemSet,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

enum SetNode {
    Single(SetEntry),
    Group(Vec<SetConfig>),
}

/// One [`SystemSet`] or a group of sets, with their ordering constraints.
///
/// Created by [`IntoSetConfig::into_set_config`], usually implicitly by calling
/// [`after`](IntoSetConfig::after), [`before`](IntoSetConfig::before) or
/// [`chain`](IntoSetConfig::chain) on a set or a tuple of sets, and passed to
/// [`Schedule::configure_sets`](crate::Schedule::configure_sets). Constraints on a
/// group apply to every set in it.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSetConfig, Schedule, SystemSet};
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
pub struct SetConfig(SetNode);

impl SetConfig {
    /// Orders every member of every set in this config after `target`, a system or a
    /// set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSetConfig, Schedule, SystemSet};
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
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.after.push(target));
        self
    }

    /// Orders every member of every set in this config before `target`, a system or a
    /// set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSetConfig, Schedule, SystemSet};
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
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.before.push(target));
        self
    }

    /// Orders the children of a group so each runs before the next.
    ///
    /// When a child is itself a group, every set in it runs before every set in the
    /// next child. Does nothing on a single set.
    ///
    /// # Examples
    ///
    /// ```
    /// use concerto_ecs::{IntoSetConfig, Schedule, SystemSet};
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
    pub fn chain(mut self) -> Self {
        if let SetNode::Group(children) = &mut self.0 {
            for index in 1..children.len() {
                let mut previous = Vec::new();
                children[index - 1].collect_targets(&mut previous);
                children[index].for_each_entry(&mut |entry| entry.after.extend(&previous));
            }
        }
        self
    }

    pub(crate) fn into_entries(self, entries: &mut Vec<SetEntry>) {
        match self.0 {
            SetNode::Single(entry) => entries.push(entry),
            SetNode::Group(children) => {
                for child in children {
                    child.into_entries(entries);
                }
            }
        }
    }

    fn for_each_entry(&mut self, f: &mut impl FnMut(&mut SetEntry)) {
        match &mut self.0 {
            SetNode::Single(entry) => f(entry),
            SetNode::Group(children) => {
                for child in children {
                    child.for_each_entry(f);
                }
            }
        }
    }

    fn collect_targets(&self, targets: &mut Vec<DependencyTarget>) {
        match &self.0 {
            SetNode::Single(entry) => targets.push(DependencyTarget::Set(entry.set)),
            SetNode::Group(children) => {
                for child in children {
                    child.collect_targets(targets);
                }
            }
        }
    }
}

/// Converts a [`SystemSet`], a [`SetConfig`], or a group of them into a [`SetConfig`].
///
/// Implemented for every [`SystemSet`], for [`SetConfig`], for `Vec<SetConfig>` and for
/// tuples of up to twelve elements that implement this trait. The provided methods
/// start a configuration from any of them.
///
/// # Examples
///
/// ```
/// use concerto_ecs::{IntoSetConfig, Schedule, SystemSet};
///
/// #[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
/// enum Frame {
///     Input,
///     Simulate,
///     Debug,
/// }
///
/// let mut schedule = Schedule::new();
/// schedule.configure_sets((Frame::Simulate, Frame::Debug).after(Frame::Input));
/// ```
pub trait IntoSetConfig: Sized {
    /// Wraps `self` into a [`SetConfig`].
    fn into_set_config(self) -> SetConfig;

    /// Orders every member of every set after `target`. See [`SetConfig::after`].
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().after(target)
    }

    /// Orders every member of every set before `target`. See [`SetConfig::before`].
    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().before(target)
    }

    /// Orders the elements so each runs before the next. See [`SetConfig::chain`].
    fn chain(self) -> SetConfig {
        self.into_set_config().chain()
    }
}

impl<S: SystemSet> IntoSetConfig for S {
    fn into_set_config(self) -> SetConfig {
        SetConfig(SetNode::Single(SetEntry {
            set: self.intern(),
            after: Vec::new(),
            before: Vec::new(),
        }))
    }
}

impl IntoSetConfig for SetConfig {
    fn into_set_config(self) -> SetConfig {
        self
    }
}

impl IntoSetConfig for Vec<SetConfig> {
    fn into_set_config(self) -> SetConfig {
        SetConfig(SetNode::Group(self))
    }
}

#[allow(unused_mut)]
#[typle(Tuple for 0..=12)]
impl<T> IntoSetConfig for T
where
    T: Tuple,
    T<_>: IntoSetConfig,
{
    fn into_set_config(self) -> SetConfig {
        let mut children = Vec::with_capacity(T::LEN);
        for typle_index!(i) in 0..T::LEN {
            children.push(self[[i]].into_set_config());
        }
        SetConfig(SetNode::Group(children))
    }
}
