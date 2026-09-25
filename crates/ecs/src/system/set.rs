use crate::{
    define_label,
    intern::Interned,
    system::config::{DependencyTarget, IntoDependencyTarget},
};

pub use concerto_ecs_macros::SystemSet;

define_label!(
    /// A named group of systems that can be ordered as a unit.
    SystemSet
);

/// A cheap, copyable handle to a [`SystemSet`].
pub type InternedSystemSet = Interned<dyn SystemSet>;

/// A set together with its ordering constraints.
pub struct SetConfig {
    pub(crate) set: InternedSystemSet,
    pub(crate) after: Vec<DependencyTarget>,
    pub(crate) before: Vec<DependencyTarget>,
}

impl SetConfig {
    /// Declares that `target` must run before every member of this set.
    pub fn after<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.after.push(target.into_target());
        self
    }

    /// Declares that `target` must run after every member of this set.
    pub fn before<M>(mut self, target: impl IntoDependencyTarget<M>) -> Self {
        self.before.push(target.into_target());
        self
    }
}

/// Turns a set into a configurable [`SetConfig`].
pub trait IntoSetConfig: Sized {
    fn into_set_config(self) -> SetConfig;

    /// Declares that `target` must run before every member of this set.
    fn after<M>(self, target: impl IntoDependencyTarget<M>) -> SetConfig {
        self.into_set_config().after(target)
    }

    /// Declares that `target` must run after every member of this set.
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
pub trait IntoSetConfigs {
    fn into_set_configs(self) -> Vec<SetConfig>;

    /// Orders the sets so each runs before the next.
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

macro_rules! impl_into_set_configs_for_tuple {
    ($($name:ident),*) => {
        impl<$($name: IntoSetConfig),*> IntoSetConfigs for ($($name,)*) {
            #[allow(non_snake_case)]
            fn into_set_configs(self) -> Vec<SetConfig> {
                let ($($name,)*) = self;
                vec![$($name.into_set_config()),*]
            }
        }
    };
}

impl_into_set_configs_for_tuple!(A, B);
impl_into_set_configs_for_tuple!(A, B, C);
impl_into_set_configs_for_tuple!(A, B, C, D);
impl_into_set_configs_for_tuple!(A, B, C, D, E);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_into_set_configs_for_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
