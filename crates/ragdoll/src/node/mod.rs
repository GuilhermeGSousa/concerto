/// The hold node, which holds a mask's parts with a strength profile's muscles.
pub mod hold;
/// The limp node, which lets the whole body go.
pub mod limp;

pub use hold::{HoldNode, HoldState};
pub use limp::LimpNode;
