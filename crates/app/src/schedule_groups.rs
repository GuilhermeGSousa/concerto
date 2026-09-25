//! Each label below names a [`Schedule`](concerto_ecs::Schedule) the engine runs.
//!
//! # Examples
//!
//! ```
//! use concerto_app::{App, schedule_groups::Update};
//!
//! fn tick() {}
//!
//! let mut app = App::new();
//! app.add_system(Update, tick);
//! ```

use concerto_ecs::system::schedule::ScheduleLabel;

/// The main world's update schedule: runs [`First`], [`FixedUpdate`] and
/// [`LateFixedUpdate`] as many times as the fixed timestep requires, then [`Update`]
/// and [`LateUpdate`].
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Main};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(Main, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Main;

/// The render world's update schedule: runs [`Render`], then [`LateRender`].
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::RenderMain};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_render_system(RenderMain, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct RenderMain;

/// Runs once in each world, after every plugin has finished building.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Startup};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(Startup, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Startup;

/// Runs at the start of every frame in the main world, before fixed updates.
/// Event buffers are swapped here.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::First};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(First, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct First;

/// Runs once per frame in the main world; where most gameplay systems go.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Update};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(Update, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Update;

/// Runs zero or more times per frame in the main world, once per fixed timestep.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::FixedUpdate};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(FixedUpdate, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FixedUpdate;

/// Runs once per frame in the main world, after [`Update`].
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::LateUpdate};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(LateUpdate, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateUpdate;

/// Runs in the main world after each [`FixedUpdate`] step.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::LateFixedUpdate};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_system(LateFixedUpdate, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateFixedUpdate;

/// Runs in the render world every frame to copy data out of the main world.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Extract};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_render_system(Extract, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Extract;

/// Runs once per frame in the render world, after [`Extract`].
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::Render};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_render_system(Render, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Render;

/// Runs once per frame in the render world, after [`Render`]; the frame is
/// finished and presented here.
///
/// # Examples
///
/// ```
/// use concerto_app::{App, schedule_groups::LateRender};
///
/// fn system() {}
///
/// let mut app = App::new();
/// app.add_render_system(LateRender, system);
/// ```
#[derive(ScheduleLabel, Clone, PartialEq, Eq, Hash, Debug)]
pub struct LateRender;
