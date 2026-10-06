use concerto_ecs::SystemSet;

/// The ordered phases of the render world: `Lights`, `Shadows`, `Draw` and `Overlay` in
/// [`Render`](concerto_app::schedule_groups::Render), `Present` in
/// [`LateRender`](concerto_app::schedule_groups::LateRender).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum RenderSet {
    /// Uploading changed lights.
    Lights,
    /// Computing and rendering shadow maps.
    Shadows,
    /// Drawing the scene.
    Draw,
    /// Drawing over the finished scene, before any pass that samples a camera's target.
    Overlay,
    /// Presenting the finished frame.
    Present,
}
