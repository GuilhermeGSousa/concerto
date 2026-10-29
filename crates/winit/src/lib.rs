//! The winit host of Concerto: the only crate that opens a window or creates a surface.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use concerto_app::App;
use concerto_render::{draw_target::DrawTarget, gpu::Gpu};
use concerto_window::{clipboard::WindowClipboard, plugin::Window};
use winit::{
    event_loop::{ControlFlow, EventLoop},
    window::Window as WinitWindow,
};

mod apply;
mod handler;
mod translate;

/// Opens a window, builds an `App` with `build`, and drives it until the window closes.
pub fn run(build: impl FnOnce(&mut App) + 'static) {
    profiling::register_thread!("main");

    let event_loop = EventLoop::builder()
        .build()
        .expect("Failed to build event loop");
    event_loop.set_control_flow(if cfg!(target_arch = "wasm32") {
        ControlFlow::Wait
    } else {
        ControlFlow::Poll
    });

    let _ = event_loop.run_app(handler::Handler::new(build));
}

async fn build_app(
    window: Arc<dyn WinitWindow>,
    build: impl FnOnce(&mut App) + 'static,
    slot: Rc<RefCell<Option<App>>>,
) {
    let instance = Gpu::instance();
    let surface = Arc::new(
        instance
            .create_surface(Arc::clone(&window))
            .expect("Error creating surface."),
    );
    let gpu = Gpu::request(&instance, Some(&*surface))
        .await
        .expect("no GPU adapter is available");

    let size = window.surface_size();
    let mut app = App::new();
    app.insert_resource(gpu)
        .insert_resource(DrawTarget::Surface(surface))
        .insert_resource(Window::new(
            (size.width, size.height),
            window.scale_factor(),
        ));
    build(&mut app);
    if let Some(clipboard) = app.get_resource_mut::<WindowClipboard>() {
        clipboard.use_os_clipboard();
    }
    *slot.borrow_mut() = Some(app);
}
