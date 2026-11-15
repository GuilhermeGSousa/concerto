use concerto::animation::graph::AnimationGraph;
use concerto::animation::player::AnimationPlayer;
use concerto::app::{
    App,
    schedule_groups::{Startup, Update},
};
use concerto::asset_id;
use concerto::audio::{AudioClip, AudioPlayer};
use concerto::color::Color;
use concerto::ecs::events::{Event, event_reader::EventReader, event_writer::EventWriter};
use concerto::ecs::{CommandQueue, Component, Query, Res, ResMut, Resource};
use concerto::foundation::assets::{
    AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
};
use concerto::foundation::time::Time;
use concerto::procedural_animation::{FootLockNode, LockPhase};
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use glam::Vec3;

const MARK_LIFETIME: f32 = 1.5;
const MARK_RADIUS: f32 = 0.12;
const MIN_STRIDE: f32 = 0.1;
const MARK_COLOR: Color = Color::rgba(1.0, 0.85, 0.2, 1.0);
const STEP_VOLUME: f32 = 0.6;
const LEFT_STEP: AssetId =
    asset_id!("content/audio/kenney_impact-sounds/Audio/footstep_concrete_000/main.gasset");
const RIGHT_STEP: AssetId =
    asset_id!("content/audio/kenney_impact-sounds/Audio/footstep_concrete_001/main.gasset");

/// Which foot came down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Foot {
    Left,
    Right,
}

impl Foot {
    fn of_leg(leg: usize) -> Self {
        if leg == 0 { Foot::Left } else { Foot::Right }
    }

    /// The letter the overlay shows for the foot.
    pub(crate) fn letter(self) -> &'static str {
        match self {
            Foot::Left => "L",
            Foot::Right => "R",
        }
    }
}

/// A foot of the player came down at a world position: sent when a leg of its foot lock plants.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub(crate) struct Footstep {
    pub(crate) foot: Foot,
    pub(crate) position: Vec3,
}

/// Each leg's lock phase as of the last look, to tell a plant from a held lock, and where it last stepped, to tell a step from a foot that shuffles in place; goes on the skeleton whose footsteps are reported.
#[derive(Component, Default, Debug)]
pub(crate) struct FootContacts {
    phases: Vec<LockPhase>,
    stepped: Vec<Option<Vec3>>,
}

impl FootContacts {
    fn strides(&mut self, leg: usize, position: Vec3) -> bool {
        if self.stepped.len() <= leg {
            self.stepped.resize(leg + 1, None);
        }
        let far = self.stepped[leg].is_none_or(|last| last.distance(position) >= MIN_STRIDE);
        if far {
            self.stepped[leg] = Some(position);
        }
        far
    }
}

/// A footstep still shown, and how long ago it landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FootstepMark {
    pub(crate) step: Footstep,
    pub(crate) age: f32,
}

/// How many footsteps were heard, the last one, and the ones still drawn.
#[derive(Resource, Default, Debug)]
pub(crate) struct FootstepLog {
    pub(crate) count: u32,
    pub(crate) last: Option<Foot>,
    pub(crate) marks: Vec<FootstepMark>,
}

fn planted(before: &[LockPhase], after: &[LockPhase]) -> Vec<usize> {
    after
        .iter()
        .enumerate()
        .filter(|(leg, phase)| {
            **phase == LockPhase::Locked && before.get(*leg) != Some(&LockPhase::Locked)
        })
        .map(|(leg, _)| leg)
        .collect()
}

/// Sends a `Footstep` for each leg of a tracked skeleton's foot lock that planted since the last look, ten centimetres or more from its last step.
pub(crate) fn detect_footsteps(
    skeletons: Query<(&AnimationPlayer, &mut FootContacts)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    mut steps: EventWriter<Footstep>,
) {
    for (player, mut contacts) in skeletons.iter() {
        player.for_each_state::<FootLockNode>(&graphs, |_, state, _| {
            let phases: Vec<LockPhase> = state.legs().iter().map(|leg| leg.phase).collect();
            for leg in planted(&contacts.phases, &phases) {
                let position = state.legs()[leg].lock;
                if contacts.strides(leg, position) {
                    steps.write(Footstep {
                        foot: Foot::of_leg(leg),
                        position,
                    });
                }
            }
            if contacts.phases != phases {
                contacts.phases = phases;
            }
        });
    }
}

/// Counts each `Footstep`, keeps it as a mark, and drops the marks older than a second and a half.
pub(crate) fn log_footsteps(
    mut steps: EventReader<Footstep>,
    mut log: ResMut<FootstepLog>,
    time: Res<Time>,
) {
    let delta = time.delta().as_secs_f32();
    if log.marks.is_empty() && steps.is_empty() {
        return;
    }
    for mark in &mut log.marks {
        mark.age += delta;
    }
    log.marks.retain(|mark| mark.age < MARK_LIFETIME);
    for step in steps.read() {
        log.count += 1;
        log.last = Some(step.foot);
        log.marks.push(FootstepMark {
            step: *step,
            age: 0.0,
        });
    }
}

/// Draws each footstep mark as a ring on the ground that shrinks as it ages.
pub(crate) fn draw_footsteps(log: Res<FootstepLog>, mut gizmos: DebugGizmos) {
    for mark in &log.marks {
        let left = 1.0 - mark.age / MARK_LIFETIME;
        gizmos.circle(mark.step.position, Vec3::Y, MARK_RADIUS * left, MARK_COLOR);
        gizmos.circle(mark.step.position, Vec3::Y, MARK_RADIUS, MARK_COLOR);
    }
}

/// The clip each foot plays when it comes down.
#[derive(Resource)]
pub(crate) struct FootstepSounds {
    left: AssetHandle<AudioClip>,
    right: AssetHandle<AudioClip>,
}

fn load_footstep_sounds(server: Res<AssetServer>, mut cmd: CommandQueue) {
    cmd.insert_resource(FootstepSounds {
        left: server.load(LEFT_STEP),
        right: server.load(RIGHT_STEP),
    });
}

/// Plays the foot's clip for each `Footstep`, on an entity that despawns when the clip ends.
pub(crate) fn play_footsteps(
    mut steps: EventReader<Footstep>,
    sounds: Option<Res<FootstepSounds>>,
    mut cmd: CommandQueue,
) {
    let Some(sounds) = sounds else {
        return;
    };
    for step in steps.read() {
        let clip = match step.foot {
            Foot::Left => sounds.left.clone(),
            Foot::Right => sounds.right.clone(),
        };
        cmd.spawn(AudioPlayer::new(clip).with_volume(STEP_VOLUME));
    }
}

/// Registers the `Footstep` event and the systems that send, log, play and draw it; the drawing only where gizmos are.
pub(crate) fn install_footsteps(app: &mut App) {
    app.register_event::<Footstep>()
        .insert_resource(FootstepLog::default())
        .add_system(Startup, load_footstep_sounds)
        .add_system(Update, detect_footsteps)
        .add_system(Update, log_footsteps)
        .add_system(Update, play_footsteps);
    if app.is_plugin_added::<DebugGizmosPlugin>() {
        app.add_system(Update, draw_footsteps);
    }
}

#[cfg(test)]
mod tests {
    use concerto::procedural_animation::LockPhase::{Free, Locked, Releasing};

    use super::planted;

    #[test]
    fn a_leg_plants_when_its_lock_goes_from_free_or_releasing_to_locked() {
        struct Case {
            before: Vec<super::LockPhase>,
            after: Vec<super::LockPhase>,
            planted: Vec<usize>,
        }
        let cases = [
            Case {
                before: vec![],
                after: vec![Free, Locked],
                planted: vec![1],
            },
            Case {
                before: vec![Free, Locked],
                after: vec![Locked, Locked],
                planted: vec![0],
            },
            Case {
                before: vec![Releasing, Locked],
                after: vec![Locked, Releasing],
                planted: vec![0],
            },
            Case {
                before: vec![Locked, Locked],
                after: vec![Locked, Free],
                planted: vec![],
            },
        ];

        for case in cases {
            assert_eq!(planted(&case.before, &case.after), case.planted);
        }
    }
}
