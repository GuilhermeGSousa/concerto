pub use concerto_foundation_macros::AsAny;

use std::any::Any;

/// A type that can be viewed as [`Any`] for downcasting; derive it with `#[derive(AsAny)]`.
pub trait AsAny: Any + 'static {
    /// Returns the value as [`Any`].
    fn as_any(&self) -> &dyn Any;

    /// Returns the value as [`Any`], mutably.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
