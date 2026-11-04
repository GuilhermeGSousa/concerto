use std::{fmt::Write as _, path::PathBuf};

use concerto::{
    app::{App, schedule_groups::Update},
    ecs::{Query, Res, ResMut, Resource, With},
    foundation::{time::Time, transform::Transform},
    motion_matching::DesiredMotion,
    physics::ground::GroundProbe,
    window::plugin::{Window, WindowCommand},
};
use glam::Vec3;

use crate::character::{Player, facing};

const WALK_SPEED: f32 = 1.4;
const ARRIVE_RADIUS: f32 = 0.35;
const PROGRESS_STEP: f32 = 0.2;
const STUCK_AFTER: f32 = 3.0;
const FALL_DEPTH: f32 = 1.0;
const TIME_LIMIT: f32 = 900.0;
const HITCH_MS: f32 = 1000.0 / 60.0;

/// The critical path in Blender coordinates (x, y, z up), from the funicular to the boiler room and back to the lodge.
const CRITICAL_PATH: &[[f32; 3]] = &[
    [0.0, -32.7, -6.24],
    [2.0, -32.4, -6.2],
    [2.0, -3.6, -0.1],
    [2.0, 0.5, 0.0],
    [0.0, 5.0, 0.2],
    [0.0, 26.6, 2.5],
    [0.0, 30.6, 2.5],
    [0.0, 32.8, 3.0],
    [-20.0, 32.8, 3.0],
    [0.0, 32.8, 3.0],
    [0.0, 35.0, 3.0],
    [0.0, 40.0, 3.0],
    [3.6, 43.5, 3.0],
    [6.0, 43.5, 3.0],
    [16.0, 43.5, 3.0],
    [26.0, 43.5, 3.0],
    [28.5, 43.5, 3.0],
    [29.2, 43.2, 3.0],
    [29.2, 41.3, 3.0],
    [29.6, 41.3, 3.0],
    [33.3, 41.3, 5.0],
    [33.7, 42.1, 5.0],
    [33.3, 42.8, 5.0],
    [29.6, 42.8, 7.0],
    [29.2, 43.3, 7.0],
    [28.5, 43.5, 7.0],
    [20.0, 43.5, 7.0],
    [9.0, 43.5, 7.0],
    [9.0, 42.0, 7.0],
    [8.0, 39.0, 7.0],
    [9.0, 42.0, 7.0],
    [9.0, 43.5, 7.0],
    [28.5, 43.5, 7.0],
    [30.5, 44.5, 7.0],
    [33.0, 45.8, 7.0],
    [33.0, 46.5, 7.0],
    [33.0, 47.2, 7.0],
    [33.0, 50.0, 7.0],
    [33.0, 55.0, 7.0],
    [33.0, 56.2, 7.0],
    [32.0, 58.0, 7.0],
    [33.0, 56.2, 7.0],
    [33.0, 54.0, 7.0],
    [31.5, 54.0, 7.0],
    [28.5, 54.0, 7.0],
    [31.5, 54.0, 7.0],
    [33.0, 54.0, 7.0],
    [33.0, 47.2, 7.0],
    [33.0, 46.5, 7.0],
    [33.0, 45.8, 7.0],
    [31.0, 45.0, 7.0],
    [29.6, 44.6, 7.0],
    [29.2, 43.3, 7.0],
    [29.6, 42.8, 7.0],
    [33.3, 42.8, 5.0],
    [33.7, 42.1, 5.0],
    [33.3, 41.3, 5.0],
    [29.6, 41.3, 3.0],
    [29.2, 41.3, 3.0],
    [29.2, 43.6, 3.0],
    [31.0, 45.0, 3.0],
    [33.0, 45.8, 3.0],
    [33.0, 46.5, 3.0],
    [33.6, 47.4, 3.0],
    [33.725, 47.8, 3.0],
    [33.725, 50.6, 1.33],
    [33.3, 51.5, 1.33],
    [32.3, 51.3, 1.33],
    [32.275, 50.3, 1.33],
    [32.275, 47.5, -0.5],
    [31.2, 47.1, -0.5],
    [27.0, 49.0, -0.5],
    [27.0, 52.0, -0.5],
    [27.0, 53.2, -0.5],
    [28.6, 55.4, -0.5],
    [30.0, 58.9, -0.5],
    [28.6, 55.4, -0.5],
    [27.0, 53.2, -0.5],
    [27.0, 52.0, -0.5],
    [27.0, 47.0, -0.5],
    [27.0, 45.7, -0.5],
    [27.0, 43.5, -0.5],
    [-3.725, 43.5, -0.5],
    [-3.725, 41.5, -0.5],
    [-3.725, 38.3, 1.33],
    [-3.0, 37.3, 1.33],
    [-2.275, 38.3, 1.33],
    [-2.275, 41.2, 3.0],
    [-2.27, 41.6, 3.0],
    [-2.27, 42.4, 3.0],
    [-2.0, 43.2, 3.0],
    [1.5, 38.0, 3.0],
    [2.4, 36.7, 3.0],
];

/// Whether the walk was asked for, with `SANATORIUM_AUTOWALK`.
pub(crate) fn requested() -> bool {
    std::env::var_os("SANATORIUM_AUTOWALK").is_some()
}

/// Steers the player along the critical path and closes the window once it is walked.
pub(crate) fn install(app: &mut App) {
    let out = std::env::var_os("SANATORIUM_AUTOWALK_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("sanatorium_autowalk"));
    app.insert_resource(Autowalk::new(critical_path(), out));
    app.add_system(Update, walk);
}

/// A Blender position (Z up) as a glTF one (Y up, -Z forward).
fn from_blender([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x, z, -y)
}

fn critical_path() -> Vec<Vec3> {
    CRITICAL_PATH.iter().copied().map(from_blender).collect()
}

struct Frame {
    time: f32,
    dt_ms: f32,
    position: Vec3,
    waypoint: usize,
}

/// The walk's progress along its waypoints, and every frame it took.
#[derive(Resource)]
pub(crate) struct Autowalk {
    waypoints: Vec<Vec3>,
    next: usize,
    best: f32,
    since_progress: f32,
    fell_on: Option<usize>,
    elapsed: f32,
    frames: Vec<Frame>,
    events: Vec<String>,
    out: PathBuf,
    written: bool,
}

impl Autowalk {
    fn new(waypoints: Vec<Vec3>, out: PathBuf) -> Self {
        Self {
            waypoints,
            next: 0,
            best: f32::INFINITY,
            since_progress: 0.0,
            fell_on: None,
            elapsed: 0.0,
            frames: Vec::new(),
            events: Vec::new(),
            out,
            written: false,
        }
    }

    fn finished(&self) -> bool {
        self.next >= self.waypoints.len() || self.elapsed > TIME_LIMIT
    }

    fn advance(&mut self) {
        self.next += 1;
        self.best = f32::INFINITY;
        self.since_progress = 0.0;
    }

    /// The velocity that heads for the next waypoint from `position`, after `dt` seconds of walking.
    fn step(&mut self, position: Vec3, dt: f32) -> Vec3 {
        self.elapsed += dt;
        while !self.finished() {
            let target = self.waypoints[self.next];
            let offset = (target - position) * Vec3::new(1.0, 0.0, 1.0);
            let distance = offset.length();
            if distance < ARRIVE_RADIUS {
                self.advance();
                continue;
            }
            let floor = match self.next.checked_sub(1) {
                Some(previous) => self.waypoints[previous].y.min(target.y),
                None => target.y,
            };
            if position.y < floor - FALL_DEPTH && self.fell_on != Some(self.next) {
                self.fell_on = Some(self.next);
                self.events.push(format!(
                    "fell: at {position:.2} heading for waypoint {} {target:.2}",
                    self.next
                ));
            }
            if distance < self.best - PROGRESS_STEP {
                self.best = distance;
                self.since_progress = 0.0;
            } else {
                self.since_progress += dt;
            }
            if self.since_progress > STUCK_AFTER {
                self.events.push(format!(
                    "stuck: at {position:.2} heading for waypoint {} {target:.2}",
                    self.next
                ));
                self.advance();
                continue;
            }
            return offset / distance * WALK_SPEED;
        }
        Vec3::ZERO
    }

    fn report(&self) -> (String, String) {
        let mut csv = String::from("time_s,frame_ms,x,y,z,waypoint\n");
        for frame in &self.frames {
            let p = frame.position;
            let _ = writeln!(
                csv,
                "{:.3},{:.3},{:.2},{:.2},{:.2},{}",
                frame.time, frame.dt_ms, p.x, p.y, p.z, frame.waypoint
            );
        }
        let walking = &self.frames[self.frames.len().min(1)..];
        let mut times: Vec<f32> = walking.iter().map(|frame| frame.dt_ms).collect();
        times.sort_by(f32::total_cmp);
        let percentile = |p: f32| {
            times
                .get(((times.len() as f32 * p) as usize).min(times.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0.0)
        };
        let hitches: Vec<&Frame> = walking.iter().filter(|f| f.dt_ms > HITCH_MS).collect();
        let mut summary = format!(
            "waypoints reached: {}/{}\nwalk time: {:.1} s\nframes: {}\nframe ms p50 {:.2} p99 {:.2} max {:.2}\nframes over {HITCH_MS:.1} ms: {}\n",
            self.next.min(self.waypoints.len()),
            self.waypoints.len(),
            self.elapsed,
            walking.len(),
            percentile(0.5),
            percentile(0.99),
            times.last().copied().unwrap_or(0.0),
            hitches.len(),
        );
        for event in &self.events {
            let _ = writeln!(summary, "{event}");
        }
        for frame in hitches {
            let _ = writeln!(
                summary,
                "hitch: {:.2} ms at t={:.2}s pos {:.2} waypoint {}",
                frame.dt_ms, frame.time, frame.position, frame.waypoint
            );
        }
        (csv, summary)
    }
}

fn walk(
    players: Query<(&Transform, &mut DesiredMotion, &GroundProbe), With<Player>>,
    time: Res<Time>,
    mut walk: ResMut<Autowalk>,
    window: Option<ResMut<Window>>,
) {
    let Some((transform, mut desired, ground)) = players.iter().next() else {
        return;
    };
    let dt = time.real_delta().as_secs_f32();
    if walk.written {
        return;
    }
    if walk.finished() {
        let (csv, summary) = walk.report();
        let _ = std::fs::write(walk.out.with_extension("csv"), csv);
        let _ = std::fs::write(walk.out.with_extension("txt"), &summary);
        eprintln!("AUTOWALK DONE\n{summary}");
        walk.written = true;
        desired.velocity = Vec3::ZERO;
        if let Some(mut window) = window {
            window.send(WindowCommand::Close);
        }
        return;
    }
    let position = transform.translation;
    let velocity = walk.step(position, dt);
    let frame = Frame {
        time: walk.elapsed,
        dt_ms: dt * 1000.0,
        position,
        waypoint: walk.next,
    };
    walk.frames.push(frame);
    if ground.is_grounded() {
        desired.velocity = velocity;
    }
    desired.rotation = facing(velocity, desired.rotation);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glam::Vec3;

    use super::{Autowalk, WALK_SPEED, critical_path, from_blender};

    fn walker(waypoints: &[Vec3]) -> Autowalk {
        Autowalk::new(waypoints.to_vec(), PathBuf::new())
    }

    #[test]
    fn a_blender_position_lands_on_the_gltf_axes() {
        assert_eq!(from_blender([1.0, 2.0, 3.0]), Vec3::new(1.0, 3.0, -2.0));
    }

    #[test]
    fn the_walk_heads_level_for_the_next_waypoint_at_walking_speed() {
        let mut walk = walker(&[Vec3::new(0.0, 5.0, -10.0)]);

        let velocity = walk.step(Vec3::ZERO, 0.016);

        assert!((velocity - Vec3::NEG_Z * WALK_SPEED).length() < 1e-5);
    }

    #[test]
    fn reaching_a_waypoint_heads_for_the_next_one() {
        let mut walk = walker(&[Vec3::new(0.0, 0.0, -1.0), Vec3::new(4.0, 0.0, -0.9)]);

        let velocity = walk.step(Vec3::new(0.0, 0.0, -0.9), 0.016);

        assert_eq!(walk.next, 1);
        assert!((velocity - Vec3::X * WALK_SPEED).length() < 1e-5);
    }

    #[test]
    fn three_seconds_without_progress_is_reported_and_the_waypoint_skipped() {
        let mut walk = walker(&[Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -20.0)]);
        for _ in 0..200 {
            walk.step(Vec3::ZERO, 0.016);
        }

        assert_eq!(walk.next, 1);
        assert_eq!(walk.events.len(), 1);
        assert!(walk.events[0].starts_with("stuck"));
    }

    #[test]
    fn dropping_a_metre_below_the_path_is_reported_once() {
        let mut walk = walker(&[Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, 3.0, -10.0)]);
        walk.step(Vec3::new(0.0, 3.0, 0.0), 0.016);

        walk.step(Vec3::new(0.0, 1.5, -2.0), 0.016);
        walk.step(Vec3::new(0.0, 1.4, -3.0), 0.016);

        assert_eq!(walk.events.len(), 1);
        assert!(walk.events[0].starts_with("fell"));
    }

    #[test]
    fn the_walk_is_finished_after_its_last_waypoint() {
        let mut walk = walker(&[Vec3::ZERO]);

        let velocity = walk.step(Vec3::ZERO, 0.016);

        assert!(walk.finished());
        assert_eq!(velocity, Vec3::ZERO);
    }

    #[test]
    fn the_critical_path_starts_at_the_player_start() {
        let path = critical_path();

        assert_eq!(path[0], Vec3::new(0.0, -6.24, 32.7));
        assert!(path.len() > 80);
    }
}
