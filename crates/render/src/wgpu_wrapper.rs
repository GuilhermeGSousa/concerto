use std::ops::{Deref, DerefMut};

/// A wrapper that marks a value `Send` and `Sync` on every target.
///
/// It is meant for wgpu handles, but the type does not restrict `T` (#1100). It derefs to the
/// value.
#[derive(Debug, Clone)]
pub struct WgpuWrapper<T>(T);

impl<T> WgpuWrapper<T> {
    /// Wraps `value`.
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

// SAFETY: callers wrap only wgpu handles, which are `Send` natively and, with `fragile-send-sync-non-atomic-wasm`, on single-threaded wasm.
unsafe impl<T> Send for WgpuWrapper<T> {}
// SAFETY: callers wrap only wgpu handles, which are `Sync` natively and, with `fragile-send-sync-non-atomic-wasm`, on single-threaded wasm.
unsafe impl<T> Sync for WgpuWrapper<T> {}

impl<T> Deref for WgpuWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for WgpuWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
