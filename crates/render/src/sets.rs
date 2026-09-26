use concerto_ecs::SystemSet;

/// The ordered phases of the render world: `Lights`, `Shadows` and `Draw` in
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
    /// Presenting the finished frame.
    Present,
}
