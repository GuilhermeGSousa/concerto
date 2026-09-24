pub use concerto_core_macros::AsAny;

use std::any::Any;

pub trait AsAny: Any + 'static {
    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;
}
