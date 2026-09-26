//! The pose library. A pose is a single frame of a clip, held with play rate
//! 0: mannequins never visibly animate, they are simply *different* each time
//! you look. Poses are grouped by how threatening they read.
use concerto::{
    animation::{
        clip::AnimationClip,
        graph::AnimationGraph,
        node::{AnimationClipNode, AnimationNodeKind, AnimationPlayMode},
    },
    ecs::{Res, ResMut, Resource},
    foundation::assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
};

use crate::{content, game::Rand};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Menace {
    /// Plausibly just a shop display.
    Display,
    /// Wrong, somehow. Head turned, crouched, weeping, mid-stride.
    Uneasy,
    /// Coming for you: reaching, lunging, crawling.
    Hunting,
}

/// (menace, clip, seconds into the clip).
const POSES: &[(Menace, AssetId, f32)] = &[
    (Menace::Display, content::IDLE, 0.4),
    (Menace::Display, content::IDLE_TALKING, 1.1),
    (Menace::Display, content::COUNTER_SHOW, 2.2),
    (Menace::Display, content::IDLE_PAPER, 1.6),
    (Menace::Display, content::IDLE_SCISSORS, 1.6),
    (Menace::Display, content::WALK_FORMAL, 0.35),
    (Menace::Display, content::DRINK, 1.5),
    (Menace::Display, content::INTERACT, 0.9),
    (Menace::Uneasy, content::IDLE_LOOK_AROUND, 1.9),
    (Menace::Uneasy, content::IDLE_LOOK_AROUND, 3.4),
    (Menace::Uneasy, content::CRYING, 2.0),
    (Menace::Uneasy, content::IDLE_TIRED, 1.0),
    (Menace::Uneasy, content::WALK, 0.35),
    (Menace::Uneasy, content::IDLE_TORCH, 0.6),
    (Menace::Uneasy, content::GROUNDSIT_IDLE, 0.6),
    (Menace::Uneasy, content::CROUCH_IDLE, 1.2),
    (Menace::Uneasy, content::PICKUP_KNEELING, 0.9),
    (Menace::Uneasy, content::DANCE, 0.3),
    (Menace::Hunting, content::PUSH_LOOP, 0.6),
    (Menace::Hunting, content::PUNCH_CROSS, 0.35),
    (Menace::Hunting, content::SWORD_ATTACK, 0.7),
    (Menace::Hunting, content::CRAWL_FWD, 0.5),
    (Menace::Hunting, content::SPRINT, 0.2),
    (Menace::Hunting, content::SPELL_DOUBLE_ENTER, 0.45),
    (Menace::Hunting, content::HIT_HEAD, 0.2),
    (Menace::Hunting, content::COUNTER_ANGRY, 0.6),
    (Menace::Hunting, content::JOG_FWD, 0.25),
];

pub struct Pose {
    pub menace: Menace,
    pub graph: AssetHandle<AnimationGraph>,
}

#[derive(Resource, Default)]
pub struct PoseLibrary {
    pub poses: Vec<Pose>,
    /// The jumpscare: an animated lunge, arms first.
    pub lunge: Option<AssetHandle<AnimationGraph>>,
}

impl PoseLibrary {
    /// A random pose of the given menace, never `avoid` (so a glance back
    /// always finds something changed).
    pub fn pick(&self, menace: Menace, avoid: Option<usize>, rand: &mut Rand) -> Option<usize> {
        let options: Vec<usize> = self
            .poses
            .iter()
            .enumerate()
            .filter(|(i, p)| p.menace == menace && Some(*i) != avoid)
            .map(|(i, _)| i)
            .collect();
        (!options.is_empty()).then(|| options[rand.index(options.len())])
    }

    pub fn graph(&self, index: usize) -> Option<AssetHandle<AnimationGraph>> {
        self.poses.get(index).map(|p| p.graph.clone())
    }
}

pub fn build_pose_library(server: Res<AssetServer>, mut library: ResMut<PoseLibrary>) {
    let still = |clip: AssetId, time: f32| {
        let node = AnimationClipNode::new(server.load::<AnimationClip>(clip))
            .with_play_mode(AnimationPlayMode::PlayOnce)
            .with_start_time(time)
            .with_play_rate(0.0);
        server.add(AnimationGraph::from_node(AnimationNodeKind::Clip(node)))
    };
    library.poses = POSES
        .iter()
        .map(|&(menace, clip, time)| Pose {
            menace,
            graph: still(clip, time),
        })
        .collect();
    let lunge = AnimationClipNode::new(server.load::<AnimationClip>(content::PUSH_ENTER))
        .with_play_mode(AnimationPlayMode::PlayOnce)
        .with_play_rate(1.6);
    library.lunge = Some(server.add(AnimationGraph::from_node(AnimationNodeKind::Clip(lunge))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tier_has_several_poses() {
        for menace in [Menace::Display, Menace::Uneasy, Menace::Hunting] {
            let count = POSES.iter().filter(|p| p.0 == menace).count();
            assert!(count >= 5, "{menace:?} has only {count} poses");
        }
    }
}
