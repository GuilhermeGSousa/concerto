use concerto_ecs::system::schedule::ScheduleLabel;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Main;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct RenderMain;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Startup;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct First;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Update;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FixedUpdate;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateUpdate;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateFixedUpdate;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Extract;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Render;

#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateRender;
