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
    /// Derive it with `#[derive(SystemSet)]`. Sets are flat: a set cannot contain another set.
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
pub struct SetConfig(SetNode);

impl SetConfig {
    /// Orders every member of every set in this config after `target`.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.after.push(target));
        self
    }

    /// Orders every member of every set in this config before `target`.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        let target = target.into_target();
        self.for_each_entry(&mut |entry| entry.before.push(target));
        self
    }

    /// Orders the children of a group so each runs before the next; see [`SystemConfig::chain`](crate::SystemConfig::chain).
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

/// Converts a [`SystemSet`], a [`SetConfig`], or a tuple or `Vec<SetConfig>` of them into a [`SetConfig`].
pub trait IntoSetConfig: Sized {
    fn into_set_config(self) -> SetConfig;

    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().after(target)
    }

    fn before<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().before(target)
    }

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
