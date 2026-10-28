use super::Component;

/// The components a component type requires, filled by `Component::register_required`.
#[derive(Default)]
pub struct RequiredComponents {}

impl RequiredComponents {
    /// Declares that the owning component requires `T`, built by `constructor` when absent.
    pub fn register<T: Component>(&mut self, constructor: fn() -> T) {
        let _ = constructor;
    }
}
