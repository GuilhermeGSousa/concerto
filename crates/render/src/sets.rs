use concerto_ecs::SystemSet;

/// The ordered phases of the render world: `Lights`, `Queue`, `Shadows`, `Background`, `Draw`, `Transparent`, `Tonemap` and `Overlay` in
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
    /// Clearing each camera's view target and drawing what lies behind the scene, such as the sky.
    Background,
    /// Drawing the scene.
    Draw,
    /// Drawing blended surfaces over the scene, back to front.
    Transparent,
    /// Writing each camera's HDR scene to its target, exposed and tonemapped.
    Tonemap,
    /// Drawing over the finished frame on each camera's target, before any pass that samples it.
    Overlay,
    /// Presenting the finished frame.
    Present,
}

/// The ordered steps inside [`RenderSet::Background`]: `Clear`, then `Draw`.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum BackgroundSet {
    /// Clearing each camera's view target, colour and depth, and the window.
    Clear,
    /// Drawing what lies behind the scene, such as the sky, over the cleared target.
    Draw,
}
