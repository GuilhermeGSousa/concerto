use concerto_ecs::SystemSet;

/// The ordered phases of the render schedules.
///
/// `Lights`, `Shadows` and `Draw` run in that order in
/// [`Render`](concerto_app::schedule_groups::Render); `Present` runs in
/// [`LateRender`](concerto_app::schedule_groups::LateRender) after the frame is finished.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub enum RenderSet {
    Lights,
    Shadows,
    Draw,
    Present,
}
