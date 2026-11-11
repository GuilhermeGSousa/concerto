use glam::Vec3;

use crate::case::{Case, ConvId, GameTime, NpcId, Phase, ScheduleEntry};
use crate::level::{Level, path_length, point_along};

/// Metres an NPC walks in one second of the night clock, at the default 20 to 1.
pub const WALK_SPEED: f32 = 1.3 / 20.0;

/// In-game seconds since midnight of a case time.
pub fn seconds(time: GameTime) -> f64 {
    f64::from(time.minutes()) * 60.0
}

/// The case time of in-game seconds since midnight, to the minute.
pub fn case_time(seconds: f64) -> GameTime {
    let minutes = (seconds / 60.0).floor().clamp(0.0, 24.0 * 60.0) as u16;
    GameTime::from_hm(minutes / 60, minutes % 60).expect("a clamped time is within the day")
}

/// Where a schedule puts its character at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NpcPose {
    pub position: Vec3,
    /// Radians about +Y; 0 looks along -Z.
    pub yaw: f32,
    /// The entry the character is at, or has just left for the next.
    pub entry: usize,
    pub walking: bool,
}

/// The entries of `npc`'s schedule in `phase`, in order.
pub fn entries_of(case: &Case, npc: NpcId, phase: Phase) -> Vec<&ScheduleEntry> {
    case.schedule(npc)
        .map(|schedule| {
            schedule
                .entries
                .iter()
                .filter(|entry| entry.phase == phase)
                .collect()
        })
        .unwrap_or_default()
}

fn yaw_along(from: Vec3, to: Vec3) -> Option<f32> {
    let direction = (to - from).with_y(0.0);
    (direction.length_squared() > 1e-8).then(|| (-direction.x).atan2(-direction.z))
}

/// Where `entries` put their character at `now`, in-game seconds since midnight: at an entry's anchor, or on the way to the next, having left early enough to arrive at its start.
pub fn pose_at(entries: &[&ScheduleEntry], level: &Level, now: f64, speed: f32) -> Option<NpcPose> {
    let current = entries
        .iter()
        .rposition(|entry| seconds(entry.start) <= now)
        .unwrap_or(0);
    let entry = entries.get(current)?;
    let anchor = level.anchor(entry.anchor);
    let standing = NpcPose {
        position: anchor.position.with_y(0.0),
        yaw: entry
            .facing
            .map_or(anchor.yaw, crate::level::yaw_from_facing),
        entry: current,
        walking: false,
    };
    let Some(next) = entries.get(current + 1) else {
        return Some(standing);
    };
    let route = level.route(entry.anchor, next.anchor);
    let length = path_length(&route);
    if length <= 1e-3 || speed <= 0.0 {
        return Some(standing);
    }
    let depart = seconds(next.start) - f64::from(length / speed);
    if now < depart {
        return Some(standing);
    }
    let walked = ((now - depart) as f32 * speed).min(length);
    let position = point_along(&route, walked);
    let ahead = point_along(&route, (walked + 0.3).min(length));
    Some(NpcPose {
        position,
        yaw: yaw_along(position, ahead).unwrap_or(standing.yaw),
        entry: current,
        walking: true,
    })
}

/// A conversation under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Talk {
    pub conversation: ConvId,
    /// In-game seconds since it began.
    pub elapsed: f64,
}

/// When `conversation` begins: the earliest start of a `phase` entry that names it.
pub fn conversation_start(case: &Case, conversation: ConvId, phase: Phase) -> Option<f64> {
    case.schedules
        .iter()
        .flat_map(|schedule| &schedule.entries)
        .filter(|entry| entry.phase == phase && entry.conversation == Some(conversation))
        .map(|entry| seconds(entry.start))
        .min_by(f64::total_cmp)
}

/// The conversations being spoken at `now`.
pub fn talks_at(case: &Case, phase: Phase, now: f64) -> Vec<Talk> {
    case.conversations
        .ids()
        .filter_map(|conversation| {
            let start = conversation_start(case, conversation, phase)?;
            let length: f64 = case
                .conversations
                .get(conversation)
                .lines
                .iter()
                .map(|line| f64::from(line.seconds))
                .sum();
            (now >= start && now < start + length).then_some(Talk {
                conversation,
                elapsed: now - start,
            })
        })
        .collect()
}

/// Which of `seconds`-long lines is being said `elapsed` seconds in; `looping` starts again after the last.
pub fn line_at(seconds: &[f32], elapsed: f64, looping: bool) -> Option<usize> {
    let total: f64 = seconds.iter().map(|seconds| f64::from(*seconds)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut left = if looping { elapsed % total } else { elapsed };
    for (index, seconds) in seconds.iter().enumerate() {
        if left < f64::from(*seconds) {
            return Some(index);
        }
        left -= f64::from(*seconds);
    }
    None
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{WALK_SPEED, case_time, entries_of, line_at, pose_at, seconds, talks_at};
    use crate::case::{GameTime, Phase};
    use crate::casefile::CaseData;

    fn at(hour: u16, minute: u16) -> f64 {
        seconds(GameTime::from_hm(hour, minute).unwrap())
    }

    #[test]
    fn a_schedule_places_its_character_as_a_function_of_time() {
        let data = CaseData::test();
        let mut level = data.level.clone();
        let door = data.catalog.doors.id("d_ab").unwrap();
        level.place_door(door, Vec3::new(6.0, 0.0, -3.0));
        let npc = data.case.characters.id("npc_01").unwrap();
        let entries = entries_of(&data.case, npc, Phase::Before);
        let seat = Vec3::new(1.5, 0.0, -1.5);
        let stand = Vec3::new(8.4, 0.0, -3.0);

        let early = pose_at(&entries, &level, at(18, 30), WALK_SPEED).unwrap();
        assert_eq!(early.position, seat);
        assert!(!early.walking);
        assert_eq!(early.entry, 0);

        let arrived = pose_at(&entries, &level, at(19, 0), WALK_SPEED).unwrap();
        assert!((arrived.position - stand).length() < 1e-3, "{arrived:?}");
        assert_eq!(arrived.entry, 1);

        let length = seat.distance(Vec3::new(6.0, 0.0, -3.0)) + 2.4;
        let halfway = at(19, 0) - f64::from(length / WALK_SPEED) / 2.0;
        let walking = pose_at(&entries, &level, halfway, WALK_SPEED).unwrap();
        assert!(walking.walking);
        assert_eq!(walking.entry, 0);
        let walked = seat.distance(walking.position.min(Vec3::new(6.0, 0.0, 0.0)))
            + (walking.position.x - 6.0).max(0.0);
        assert!((walked - length / 2.0).abs() < 0.05, "{walking:?}");

        let before = pose_at(&entries, &level, at(12, 0), WALK_SPEED).unwrap();
        assert_eq!(
            before.position, seat,
            "before its first entry it waits there"
        );
        let last = pose_at(&entries, &level, at(23, 59), WALK_SPEED).unwrap();
        assert_eq!(last.position, seat);
        assert_eq!(last.entry, 2);
    }

    #[test]
    fn a_conversation_runs_from_the_entries_that_name_it_line_by_line() {
        let data = CaseData::test();
        let conversation = data.case.conversations.id("conv_01").unwrap();

        assert!(talks_at(&data.case, Phase::Before, at(18, 59)).is_empty());
        let talks = talks_at(&data.case, Phase::Before, at(19, 0) + 150.0);
        assert_eq!(talks.len(), 1);
        assert_eq!(talks[0].conversation, conversation);
        assert_eq!(talks[0].elapsed, 150.0);
        assert!(talks_at(&data.case, Phase::Before, at(19, 0) + 300.0).is_empty());

        let lines = [100.0, 100.0, 100.0];
        assert_eq!(line_at(&lines, 0.0, false), Some(0));
        assert_eq!(line_at(&lines, 150.0, false), Some(1));
        assert_eq!(line_at(&lines, 300.0, false), None);
        assert_eq!(line_at(&lines, 350.0, true), Some(0));
        assert_eq!(line_at(&[], 1.0, true), None);
        assert_eq!(
            case_time(at(19, 40) + 59.0),
            GameTime::from_hm(19, 40).unwrap()
        );
    }
}
