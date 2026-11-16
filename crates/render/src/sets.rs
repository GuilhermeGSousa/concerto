use concerto_ecs::SystemSet;

/// The ordered phases of the render world: `Lights`, `Queue`, `Shadows`, `Draw`, `Overlay` and `Resolve` in
/// [`Render`](concerto_app::schedule_groups::Render), `Present` in
/// [`LateRender`](concerto_app::schedule_groups::LateRender).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum RenderSet {
    /// Culling lights, assigning shadow-map layers and uploading the visible lights.
    Lights,
    /// Culling and binning mesh instances into each view's draws.
    Queue,
    /// Computing and rendering shadow maps.
    Shadows,
    /// Drawing the scene.
    Draw,
    /// Drawing over the finished scene, into the camera's multisampled texture when it has one.
    Overlay,
    /// Resolving each multisampled camera into its target, before anything samples it or draws unsampled.
    Resolve,
    /// Presenting the finished frame.
    Present,
}
