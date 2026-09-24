use concerto_ecs::resource::Resource;

use egui::Context;
use egui_wgpu::{
    wgpu::{Device, TextureFormat},
    Renderer,
};
use egui_winit::{winit::window::Window, State};

#[derive(Resource)]
pub struct UIRenderer {
    pub(crate) renderer: Renderer,
    pub(crate) state: State,
}

impl UIRenderer {
    pub fn new(
        window: &Window,
        device: &Device,
        output_color_format: TextureFormat,
        output_depth_format: Option<TextureFormat>,
        msaa_samples: u32,
        dithering: bool,
    ) -> Self {
        let egui_context = Context::default();

        let renderer = Renderer::new(
            device,
            output_color_format,
            output_depth_format,
            msaa_samples,
            dithering,
        );

        let state = egui_winit::State::new(
            egui_context,
            egui::viewport::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(2 * 1024), // default dimension is 2048
        );

        UIRenderer { renderer, state }
    }

    pub fn context(&self) -> &Context {
        self.state.egui_ctx()
    }
}

#[derive(Resource)]
pub struct UIWindow(pub Window);
