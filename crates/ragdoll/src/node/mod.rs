pub mod balance;
pub mod dying;
pub mod fall;
pub mod flinch;
pub mod hold;
pub mod limp;
pub mod shot;
pub mod stagger;

pub use balance::{BalanceNode, BalanceSettings, BalanceState, Hindrance};
pub use dying::{DyingNode, DyingState};
pub use fall::{FallNode, FallState};
pub use flinch::{FlinchNode, FlinchState};
pub use hold::{HoldNode, HoldState};
pub use limp::LimpNode;
pub use shot::{ShotNode, ShotState};
pub use stagger::{StaggerNode, StaggerState};

pub(crate) struct Decided<E> {
    value: Option<E>,
    raised: bool,
}

impl<E> Default for Decided<E> {
    fn default() -> Self {
        Self {
            value: None,
            raised: false,
        }
    }
}

impl<E: Clone> Decided<E> {
    pub(crate) fn get(&self) -> Option<&E> {
        self.value.as_ref()
    }

    pub(crate) fn is_decided(&self) -> bool {
        self.value.is_some()
    }

    pub(crate) fn decide(&mut self, value: E) {
        self.value.get_or_insert(value);
    }

    pub(crate) fn unraised(&mut self) -> Option<E> {
        let value = self.value.clone().filter(|_| !self.raised)?;
        self.raised = true;
        Some(value)
    }
}
