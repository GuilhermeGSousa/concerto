use concerto_ecs::SystemSet;

/// The ordered phases of the UI frame in `LateUpdate`, from input to post-layout.
///
/// Order systems against these sets to run relative to the UI without naming its
/// systems.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::LateUpdate};
/// use concerto_ecs::IntoSystemConfig;
/// use concerto_ui::sets::UiSet;
///
/// fn update_inventory_panel() {}
///
/// let mut app = App::new();
/// app.add_system(
///     LateUpdate,
///     update_inventory_panel.after(UiSet::Input).before(UiSet::Layout),
/// );
/// ```
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum UiSet {
    /// Pointer hit testing, focus and text capture.
    Input,
    /// Widget behaviour driven by this frame's input.
    Widgets,
    /// One-off construction of widget visuals.
    Setup,
    /// Projecting widget state onto their visual nodes.
    Project,
    /// Syncing widget state into materials.
    Materials,
    /// Computing layout.
    Layout,
    /// Work that needs this frame's layout.
    PostLayout,
}
