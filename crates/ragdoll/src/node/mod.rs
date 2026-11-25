pub mod hold;
pub mod limp;

use glam::Quat;

use crate::view::BodyView;

pub use hold::{HoldNode, HoldState};
pub use limp::LimpNode;

fn animated_target(body: &BodyView, part: usize) -> Quat {
    let animated = body.animated();
    let parent = body
        .resolved()
        .settings()
        .parts
        .get(part)
        .and_then(|settings| settings.parent);
    match (
        parent.and_then(|parent| animated.get(parent)),
        animated.get(part),
    ) {
        (Some(parent), Some(child)) => (parent.rotation.inverse() * child.rotation).normalize(),
        _ => Quat::IDENTITY,
    }
}
