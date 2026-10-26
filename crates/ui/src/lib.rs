//! The retained-mode UI of Concerto.
//!
//! # Examples
//!
//! ```no_run
//! use concerto_app::App;
//! use concerto_render::plugin::RenderPlugin;
//! use concerto_ui::plugin::UIPlugin;
//! use concerto_window::plugin::WindowPlugin;
//!
//! fn game(app: &mut App) {
//!     app.register_plugin(WindowPlugin)
//!         .register_plugin(RenderPlugin)
//!         .register_plugin(UIPlugin);
//! }
//! ```

pub mod anchor;
pub mod checkbox;
pub mod elements;
pub mod focus;
pub mod frame_stats_overlay;
pub mod interaction;
pub mod items;
pub mod list;
pub mod material;
pub mod node;
pub mod plugin;
pub mod render;
pub mod scroll;
pub mod sets;
pub mod slider;
pub mod text;
pub mod text_input;
pub mod theme;
pub mod transform;
pub mod tree;
pub mod widgets;

mod resources;
mod vertex;

pub use node::{UISharedViewport, UIViewport};
pub use resources::UIRenderDiagnostics;

#[cfg(test)]
mod tests {}

#[cfg(test)]
pub(crate) mod testing {
    use concerto_ecs::{
        Changed, Component, Entity, IntoSystem, Query, ResMut, Resource, System, World,
        system::BoxedSystem,
    };

    #[derive(Resource, Default)]
    struct Changes(Vec<usize>);

    fn count_changed<T: Component>(
        changed: Query<Entity, Changed<T>>,
        mut changes: ResMut<Changes>,
    ) {
        changes.0.push(changed.iter().count());
    }

    pub(crate) struct Watch {
        writer: BoxedSystem,
        watcher: BoxedSystem,
    }

    impl Watch {
        pub(crate) fn new<T: Component, M>(
            world: &mut World,
            writer: impl IntoSystem<(), M>,
        ) -> Self {
            world.insert_resource(Changes::default());
            let mut writer = writer.into_boxed_system();
            let mut watcher = count_changed::<T>.into_boxed_system();
            writer.initialize(world);
            watcher.initialize(world);
            Self { writer, watcher }
        }

        pub(crate) fn run(&mut self, world: &mut World) -> usize {
            self.writer.run_and_apply((), world);
            self.watcher.run_and_apply((), world);
            *world.get_resource::<Changes>().unwrap().0.last().unwrap()
        }
    }
}
