use concerto_ecs::resource::Resource;
use wgpu::{TextureView, TextureViewDescriptor};

#[derive(Resource)]
pub struct RenderWindow {
    view: Option<TextureView>,
    texture: Option<wgpu::SurfaceTexture>,
    size: (u32, u32),
}

impl RenderWindow {
    pub fn new() -> Self {
        RenderWindow {
            view: None,
            texture: None,
            size: (0, 0),
        }
    }

    pub fn set_swapchain_texture(&mut self, frame: wgpu::SurfaceTexture) {
        self.view = Some(frame.texture.create_view(&TextureViewDescriptor::default()));
        self.texture = Some(frame);
    }

    pub fn get_view(&self) -> Option<&TextureView> {
        self.view.as_ref()
    }

    pub(crate) fn set_size(&mut self, size: (u32, u32)) {
        self.size = size;
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn present(&mut self) {
        if let Some(texture) = self.texture.take() {
            texture.present();
            self.view = None;
        }
    }
}

impl Default for RenderWindow {
    fn default() -> Self {
        Self::new()
    }
}
