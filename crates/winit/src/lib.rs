//! The winit host of Concerto: the only crate that opens a window or creates a surface.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use concerto_app::App;
use concerto_render::{draw_target::DrawTarget, gpu::Gpu};
use concerto_window::{clipboard::WindowClipboard, plugin::Window};
use winit::{
    event_loop::{ControlFlow, EventLoop},
    window::Window as WinitWindow,
};

#[cfg(target_arch = "wasm32")]
use winit::platform::web::EventLoopExtWebSys;

mod apply;
mod handler;
mod translate;

/// Opens a window, builds an `App` with `build`, and drives it until the window closes.
#[allow(deprecated)]
#[allow(unused_mut)]
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

    let mut attributes = WinitWindow::default_attributes().with_title("Concerto");

    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::WindowAttributesExtWebSys;
        attributes = attributes.with_append(true);
    }

    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .expect("create window err."),
    );
    window.set_cursor_visible(true);

    let slot = Rc::new(RefCell::new(None));
    let building = build_app(Arc::clone(&window), build, Rc::clone(&slot));
    let handler = handler::Handler::new(slot, window);

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            wasm_bindgen_futures::spawn_local(building);
            event_loop.spawn_app(handler);
        } else {
            pollster::block_on(building);
            let mut handler = handler;
            let _ = event_loop.run_app(&mut handler);
        }
    }
}

async fn build_app(
    window: Arc<WinitWindow>,
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

    let size = window.inner_size();
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
