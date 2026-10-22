use concerto_ecs::SystemSet;

/// The ordered phases of the UI in `LateUpdate`, for ordering systems against the UI.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum UiSet {
    /// Pointer hit testing and signals, focus and text capture.
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
