/// A length along one axis of a UI node.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub enum UIValue {
    /// Sized by the layout, from content or flex; the default.
    #[default]
    Auto,
    /// A length in logical pixels.
    Px(f32),
    /// A percentage, from 0 to 100, of the parent's size on the same axis.
    Percent(f32),
}
