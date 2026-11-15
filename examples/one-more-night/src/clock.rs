use std::time::Duration;

use concerto::{
    app::{
        App, Plugin,
        schedule_groups::{Startup, Update},
    },
    ecs::{
        CommandQueue, Component, Event, IntoSystemConfig, Query, Res, ResMut, Resource, With,
        events::event_writer::EventWriter,
    },
    foundation::time::Time,
    name::Name,
    ui::{
        node::UINode,
        text::{FontFamily, UIText},
        transform::UIValue,
    },
};

const SECONDS_PER_MINUTE: f64 = 60.0;
const SECONDS_PER_HOUR: f64 = 3600.0;

/// A time of the night, in in-game seconds since the midnight before it; 24:00 is the midnight that ends it.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct GameTime(f64);

impl GameTime {
    /// The time `hour`:`minute`, with 24:00 for the midnight that ends the night.
    pub fn hm(hour: u32, minute: u32) -> Self {
        Self(f64::from(hour) * SECONDS_PER_HOUR + f64::from(minute) * SECONDS_PER_MINUTE)
    }

    /// The time `seconds` in-game seconds after the midnight before the night.
    pub fn from_seconds(seconds: f64) -> Self {
        Self(seconds)
    }

    /// In-game seconds since the midnight before the night.
    pub fn seconds(self) -> f64 {
        self.0
    }

    /// The hour on a 24-hour clock face, so 24:00 reads 0.
    pub fn hour(self) -> u32 {
        (self.0 / SECONDS_PER_HOUR).floor() as u32 % 24
    }

    /// The minute past the hour.
    pub fn minute(self) -> u32 {
        ((self.0 % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE).floor() as u32
    }
}

/// The one authoritative in-game time of the night, from `start` to `end`, advancing at `scale` times real time.
#[derive(Resource, Clone, Debug)]
pub struct NightClock {
    now: GameTime,
    start: GameTime,
    end: GameTime,
    scale: f64,
    haste: f64,
    started: bool,
}

impl Default for NightClock {
    fn default() -> Self {
        Self {
            now: GameTime::hm(18, 0),
            start: GameTime::hm(18, 0),
            end: GameTime::hm(24, 0),
            scale: 20.0,
            haste: 1.0,
            started: false,
        }
    }
}

/// Whether the shared pause holds the night clock; nothing else stops it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct NightPaused(pub bool);

/// One named moment of the night.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedEvent {
    pub at: GameTime,
    pub name: Name,
}

/// The named moments of the night, kept in time order; each fires once, when the clock passes it, and those at or before the start fire on the first advance.
#[derive(Resource, Clone, Debug, Default)]
pub struct NightSchedule {
    events: Vec<TimedEvent>,
}

impl NightSchedule {
    /// Adds the moment `name` at `at`.
    pub fn at(&mut self, at: GameTime, name: Name) -> &mut Self {
        let index = self.events.partition_point(|event| event.at <= at);
        self.events.insert(index, TimedEvent { at, name });
        self
    }

    /// The moments in time order.
    pub fn events(&self) -> &[TimedEvent] {
        &self.events
    }
}

/// The clock passed the moment `name` of the `NightSchedule`.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct ClockReached {
    pub name: Name,
    pub at: GameTime,
}

/// The clock passed a whole hour; midnight strikes 0.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct HourStruck {
    pub hour: u32,
}

/// The night clock reached its end, 00:00 unless `NightClock::with_end` moved it.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub struct Midnight;

/// What one advance of the night clock passed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NightStep {
    pub reached: Vec<ClockReached>,
    pub hours: Vec<HourStruck>,
    pub midnight: bool,
}

impl NightClock {
    /// Advances `scale` times faster than real time; 20 runs 18:00 to 00:00 in 18 real minutes.
    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = scale.max(0.0);
        self
    }

    /// The time the night starts at and the clock reads before it moves.
    pub fn with_start(mut self, start: GameTime) -> Self {
        self.start = start;
        self.now = start;
        self.started = false;
        self
    }

    /// The time the night ends and the clock stops at; it must come after the start, or the night is over before it begins.
    pub fn with_end(mut self, end: GameTime) -> Self {
        self.end = end;
        self
    }

    /// The time of the night now.
    pub fn now(&self) -> GameTime {
        self.now
    }

    /// How many times faster than real time the night runs.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Runs the clock `haste` times faster than its scale, for players who are waiting; 1 is the usual pace.
    pub fn set_haste(&mut self, haste: f64) {
        self.haste = haste.max(1.0);
    }

    /// How many times faster than its scale the clock is running.
    pub fn haste(&self) -> f64 {
        self.haste
    }

    /// Puts the clock back at the night's start, as a new night begins.
    pub fn restart(&mut self) {
        self.now = self.start;
        self.started = false;
    }

    /// Moves the clock to `time`, kept within the night; nothing between fires.
    pub fn jump_to(&mut self, time: GameTime) {
        self.now = GameTime::from_seconds(
            time.seconds()
                .clamp(self.start.seconds(), self.end.seconds()),
        );
    }

    /// Whether the clock reached the end of the night.
    pub fn is_over(&self) -> bool {
        self.now >= self.end
    }

    /// Advances by `real` time unless `paused`, stopping at the end of the night, and returns what it passed.
    pub fn advance(&mut self, real: Duration, paused: bool, schedule: &NightSchedule) -> NightStep {
        let mut step = NightStep::default();
        if paused || self.is_over() {
            return step;
        }
        let from = self.now;
        let first = !self.started;
        self.started = true;
        let passed = real.as_secs_f64() * self.scale * self.haste;
        let to = GameTime((from.0 + passed).min(self.end.0.max(from.0)));
        self.now = to;
        step.reached = schedule
            .events
            .iter()
            .filter(|event| (first || from < event.at) && event.at <= to)
            .map(|event| ClockReached {
                name: event.name,
                at: event.at,
            })
            .collect();
        let first_hour = (from.0 / SECONDS_PER_HOUR).floor() as u32 + 1;
        step.hours = (first_hour..)
            .map(|hour| GameTime(f64::from(hour) * SECONDS_PER_HOUR))
            .take_while(|hour| *hour <= to)
            .map(|hour| HourStruck { hour: hour.hour() })
            .collect();
        step.midnight = from < self.end && self.end <= to;
        step
    }
}

/// Runs `clock` as the night clock, fires its schedule and its hours, and shows the time on screen.
#[derive(Clone, Debug, Default)]
pub struct NightClockPlugin {
    pub clock: NightClock,
    /// Shows the time in the window's corner, for a build with no pocket watch.
    pub readout: bool,
}

impl Plugin for NightClockPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.clock.clone())
            .insert_resource(NightSchedule::default())
            .insert_resource(NightPaused::default())
            .register_event::<ClockReached>()
            .register_event::<HourStruck>()
            .register_event::<Midnight>()
            .add_system(Update, advance_night_clock);
        if self.readout {
            app.add_system(Startup, spawn_clock_readout)
                .add_system(Update, update_clock_readout.after(advance_night_clock));
        }
    }
}

/// Advances the night clock by the frame's real time and sends what it passed.
pub fn advance_night_clock(
    mut clock: ResMut<NightClock>,
    paused: Res<NightPaused>,
    schedule: Res<NightSchedule>,
    time: Res<Time>,
    mut reached: EventWriter<ClockReached>,
    mut hours: EventWriter<HourStruck>,
    mut midnight: EventWriter<Midnight>,
) {
    let step = clock.advance(time.real_delta(), paused.0, &schedule);
    for event in step.reached {
        reached.write(event);
    }
    for event in step.hours {
        hours.write(event);
    }
    if step.midnight {
        midnight.write(Midnight);
    }
}

#[derive(Component)]
struct ClockReadout;

fn readout(time: GameTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn spawn_clock_readout(clock: Res<NightClock>, mut cmd: CommandQueue) {
    cmd.spawn((
        UINode::default().with_size(UIValue::Px(80.0), UIValue::Px(24.0)),
        UIText {
            text: readout(clock.now()),
            font_size: 18.0,
            line_height: 24.0,
            font_family: FontFamily::Monospace,
            ..Default::default()
        },
        ClockReadout,
    ));
}

fn update_clock_readout(clock: Res<NightClock>, texts: Query<&mut UIText, With<ClockReadout>>) {
    let shown = readout(clock.now());
    for mut text in texts.iter() {
        if text.text != shown {
            text.text = shown.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use concerto::{
        ecs::{
            Schedule, World,
            events::{event_channel::EventChannel, event_cursor::EventCursor},
            system::executor::single_thread::SingleThreadedExecutor,
        },
        name::name,
    };

    use super::*;

    fn minutes(real: u64) -> Duration {
        Duration::from_secs(real * 60)
    }

    fn hours(step: &NightStep) -> Vec<u32> {
        step.hours.iter().map(|struck| struck.hour).collect()
    }

    #[test]
    fn the_night_runs_twenty_times_faster_than_real_time_to_midnight_firing_each_moment_once() {
        let mut clock = NightClock::default();
        let mut schedule = NightSchedule::default();
        schedule
            .at(GameTime::hm(20, 0), name!("later"))
            .at(GameTime::hm(17, 0), name!("early"))
            .at(GameTime::hm(19, 30), name!("sooner"));
        let names = |step: &NightStep| {
            step.reached
                .iter()
                .map(|reached| reached.name)
                .collect::<Vec<_>>()
        };

        let first = clock.advance(Duration::from_secs(3), false, &schedule);
        assert_eq!(clock.now(), GameTime::hm(18, 1));
        assert_eq!(names(&first), [name!("early")]);

        let second = clock.advance(minutes(6), false, &schedule);
        assert_eq!(clock.now(), GameTime::hm(20, 1));
        assert_eq!(names(&second), [name!("sooner"), name!("later")]);
        assert_eq!(hours(&second), [19, 20]);

        let paused = clock.advance(minutes(10), true, &schedule);
        assert_eq!(clock.now(), GameTime::hm(20, 1));
        assert!(paused.reached.is_empty() && paused.hours.is_empty());

        clock.set_haste(12.0);
        let hasted = clock.advance(Duration::from_secs(20), false, &schedule);
        assert_eq!(clock.now(), GameTime::hm(21, 21));
        assert_eq!(hours(&hasted), [21]);
        clock.set_haste(1.0);

        let last = clock.advance(minutes(18), false, &schedule);
        assert!(last.midnight && clock.is_over());
        assert_eq!(clock.now(), GameTime::hm(24, 0));
        assert_eq!(hours(&last), [22, 23, 0]);
        assert!(names(&last).is_empty());
        assert!(!clock.advance(minutes(5), false, &schedule).midnight);
    }

    fn world_after_one_frame(paused: bool) -> World {
        let mut world = World::new();
        world.insert_resource(NightClock::default().with_scale(1.0e7));
        world.insert_resource(NightSchedule::default());
        world.insert_resource(NightPaused(paused));
        world.insert_resource(EventChannel::<ClockReached>::new());
        world.insert_resource(EventChannel::<HourStruck>::new());
        world.insert_resource(EventChannel::<Midnight>::new());
        let mut time = Time::new();
        std::thread::sleep(Duration::from_millis(20));
        time.update();
        world.insert_resource(time);

        let mut schedule = Schedule::new();
        schedule.add_system(advance_night_clock);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);
        world
    }

    fn sent<T: Event + Copy + 'static>(world: &World) -> Vec<T> {
        let channel = world.get_resource::<EventChannel<T>>().unwrap();
        channel
            .read_from(&mut EventCursor::default())
            .copied()
            .collect()
    }

    #[test]
    fn the_clock_system_sends_the_hours_and_midnight_it_passed_unless_the_shared_pause_is_on() {
        let held = world_after_one_frame(true);
        assert_eq!(
            held.get_resource::<NightClock>().unwrap().now(),
            GameTime::hm(18, 0)
        );
        assert!(sent::<Midnight>(&held).is_empty());

        let world = world_after_one_frame(false);
        assert!(world.get_resource::<NightClock>().unwrap().is_over());
        assert_eq!(
            sent::<HourStruck>(&world)
                .iter()
                .map(|struck| struck.hour)
                .collect::<Vec<_>>(),
            vec![19, 20, 21, 22, 23, 0]
        );
        assert_eq!(sent::<Midnight>(&world), vec![Midnight]);
    }
}
