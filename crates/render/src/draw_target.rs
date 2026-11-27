use std::sync::{Arc, Mutex};

use concerto_ecs::resource::Resource;

use crate::render_asset::render_texture::RenderTexture;

/// A slot a hosted `App`'s render world fills with the texture it draws its window into.
#[derive(Clone)]
pub struct SharedTexture {
    format: wgpu::TextureFormat,
    slot: Arc<Mutex<Option<RenderTexture>>>,
}

impl SharedTexture {
    /// Creates an empty slot for textures of `format`.
    pub fn new(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            slot: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the format of the textures the slot holds.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// The texture most recently drawn into, or `None` until the `App` has a non-zero size.
    ///
    /// # Panics
    ///
    /// Panics if a thread panicked while holding the slot's lock.
    pub fn get(&self) -> Option<RenderTexture> {
        self.slot.lock().unwrap().clone()
    }

    /// Whether both handles name the same slot.
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.slot, &other.slot)
    }

    pub(crate) fn publish(&self, texture: RenderTexture) {
        *self.slot.lock().unwrap() = Some(texture);
    }
}

/// Where an `App`'s window cameras and UI draw. The host inserts it before the recipe runs.
#[derive(Resource, Clone)]
pub enum DrawTarget {
    /// A window's surface, presented each frame.
    Surface(Arc<wgpu::Surface<'static>>),
    /// A texture a host shows, such as the editor's game viewport.
    Texture(SharedTexture),
    /// Nowhere: window cameras and UI are not drawn.
    None,
}
