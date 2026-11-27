use concerto_ecs::resource::Resource;
use wgpu::{TextureView, TextureViewDescriptor};

/// The texture the render world draws the window's frame into this frame: the surface's
/// current texture, or a host's shared texture.
#[derive(Resource)]
pub struct RenderWindow {
    view: Option<TextureView>,
    texture: Option<wgpu::SurfaceTexture>,
    size: (u32, u32),
}

impl RenderWindow {
    /// Creates a window with no texture and a size of zero.
    pub fn new() -> Self {
        RenderWindow {
            view: None,
            texture: None,
            size: (0, 0),
        }
    }

    /// Sets the surface texture to draw into this frame, to be presented by [`Self::present`].
    pub fn set_swapchain_texture(&mut self, frame: wgpu::SurfaceTexture) {
        self.view = Some(frame.texture.create_view(&TextureViewDescriptor::default()));
        self.texture = Some(frame);
    }

    pub(crate) fn set_offscreen(&mut self, view: TextureView) {
        self.view = Some(view);
        self.texture = None;
    }

    /// Returns the view of this frame's texture, or `None` when there is none.
    pub fn get_view(&self) -> Option<&TextureView> {
        self.view.as_ref()
    }

    pub(crate) fn set_size(&mut self, size: (u32, u32)) {
        self.size = size;
    }

    /// Returns the drawable size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Presents this frame's surface texture, if there is one, and drops its view.
    pub fn present(&mut self, queue: &wgpu::Queue) {
        if let Some(texture) = self.texture.take() {
            queue.present(texture);
            self.view = None;
        }
    }
}

impl Default for RenderWindow {
    fn default() -> Self {
        Self::new()
    }
}
