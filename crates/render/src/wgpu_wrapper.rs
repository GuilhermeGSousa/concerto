use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone)]
pub struct WgpuWrapper<T>(T);

impl<T> WgpuWrapper<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

// SAFETY: the wrapper holds wgpu handles, which are `Send` natively and are only `!Send` on wasm, where `fragile-send-sync-non-atomic-wasm` keeps everything on one thread.
unsafe impl<T> Send for WgpuWrapper<T> {}
// SAFETY: the wrapper holds wgpu handles, which are `Sync` natively and are only `!Sync` on wasm, where `fragile-send-sync-non-atomic-wasm` keeps everything on one thread.
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
