/// The balance node, which keeps a body on its own feet.
pub mod balance;
/// The dying node, which drains the body's strength until it is dead.
pub mod dying;
/// The flinch node, which makes the body give where it is hit and recover.
pub mod flinch;
/// The hold node, which holds a mask's parts with a strength profile's muscles.
pub mod hold;
/// The limp node, which lets the whole body go.
pub mod limp;
/// The shot node, which answers a gunshot with the whole body.
pub mod shot;
/// The stagger node, which makes a balancing body stumble.
pub mod stagger;

pub use balance::{BalanceNode, BalanceSettings, BalanceState, Hindrance};
pub use dying::{DyingNode, DyingState};
pub use flinch::{FlinchNode, FlinchState};
pub use hold::{HoldNode, HoldState};
pub use limp::LimpNode;
pub use shot::{ShotNode, ShotState};
pub use stagger::{StaggerNode, StaggerState};
