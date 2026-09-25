use concerto_ecs::SystemSet;

/// The ordered phases of the render world's schedules.
///
/// `Lights`, `Shadows` and `Draw` run in that order in
/// [`Render`](concerto_app::schedule_groups::Render); `Present` runs in
/// [`LateRender`](concerto_app::schedule_groups::LateRender) once the frame is finished.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Render};
/// use concerto_ecs::IntoSystemConfig;
/// use concerto_render::sets::RenderSet;
///
/// fn cull_lights() {}
///
/// let mut app = App::new();
/// app.add_render_system(Render, cull_lights.after(RenderSet::Lights).before(RenderSet::Shadows));
/// ```
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
