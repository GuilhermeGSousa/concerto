use concerto_ecs::SystemSet;

/// The ordered phases of the render world: `Lights`, `Queue`, `Shadows`, `Background`, `Draw` and `Overlay` in
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
    /// Clearing each camera's target and drawing what lies behind the scene, such as the sky.
    Background,
    /// Drawing the scene.
    Draw,
    /// Drawing over the finished scene, before any pass that samples a camera's target.
    Overlay,
    /// Presenting the finished frame.
    Present,
}

/// The ordered steps inside [`RenderSet::Background`]: `Clear`, then `Draw`.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum BackgroundSet {
    /// Clearing each camera's target and depth.
    Clear,
    /// Drawing what lies behind the scene, such as the sky, over the cleared target.
    Draw,
}
