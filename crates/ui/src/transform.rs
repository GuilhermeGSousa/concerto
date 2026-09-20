/// A length along one axis of a UI node.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub enum UIValue {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}
