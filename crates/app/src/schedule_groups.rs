use concerto_ecs::system::schedule::ScheduleLabel;

/// The main world's frame: [`First`], fixed updates, [`Update`], then [`LateUpdate`].
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Main;

/// The render world's frame: [`Render`], then [`LateRender`].
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct RenderMain;

/// Runs once in each world after every plugin has finished.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Startup;

/// Runs first each frame in the main world; event buffers are swapped here.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct First;

/// Runs once per frame in the main world.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Update;

/// Runs once per fixed timestep in the main world.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FixedUpdate;

/// Runs once per frame in the main world, after [`Update`].
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateUpdate;

/// Runs after each [`FixedUpdate`] step.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateFixedUpdate;

/// Runs in the render world to copy data out of the main world.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Extract;

/// Runs once per frame in the render world, after [`Extract`].
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Render;

/// Runs after [`Render`]; the frame is finished and presented here.
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateRender;
