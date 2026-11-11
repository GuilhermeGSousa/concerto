use concerto::animation::graph::{AnimationGraph, AnimationNodeIndex};
use concerto::animation::player::AnimationPlayer;
use concerto::app::{
    App,
    schedule_groups::{Startup, Update},
};
use concerto::director::VirtualCamera;
use concerto::ecs::{Component, Query, Res, With};
use concerto::foundation::assets::asset_store::AssetStore;
use concerto::foundation::time::Time;
use concerto::foundation::transform::GlobalTransform;
use concerto::motion_matching::CharacterLink;
use concerto::name::Name;
use concerto::window::define_action;
use concerto::window::input::actions::{ActionMap, Shortcut};
use concerto::window::input::{GamepadButton, Input, KeyCode, MouseButton};
use glam::Vec3;

use crate::character::Aim;
use crate::footsteps::install_footsteps;
use crate::retarget::{
    aim_target, aim_weight, fatigue, look_weight, pace, pistol_weight, sway, tremor,
};
use crate::ui::spawn_stat_sliders;

const PISTOL_BLEND: f32 = 0.25;
const AIM_DISTANCE: f32 = 30.0;

define_action!(
    /// Draws or holsters the pistol: G, or the north face button.
    TogglePistol
);
define_action!(
    /// Fires while aiming: the left mouse button, or the right trigger.
    Fire
);

/// Binds drawing the pistol and firing it globally.
pub(crate) fn bind_body_controls(actions: &mut ActionMap) {
    actions.bind_global(TogglePistol, Shortcut::key(KeyCode::KeyG));
    actions.bind_global(TogglePistol, GamepadButton::North);
    actions.bind_global(Fire, MouseButton::Left);
    actions.bind_global(Fire, GamepadButton::RightTrigger);
}

/// The span a stat's slider covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StatRange {
    pub(crate) min: f32,
    pub(crate) max: f32,
}

/// One of the numbers the sliders set on the player's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stat {
    Fatigue,
    Sway,
    Tremor,
    Look,
    Pace,
}

impl Stat {
    pub(crate) const ALL: [Stat; 5] = [
        Stat::Fatigue,
        Stat::Sway,
        Stat::Tremor,
        Stat::Look,
        Stat::Pace,
    ];

    /// The name shown beside the stat's slider.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Stat::Fatigue => "fatigue",
            Stat::Sway => "sway",
            Stat::Tremor => "tremor",
            Stat::Look => "look-at",
            Stat::Pace => "pace",
        }
    }

    /// The values the stat's slider runs between.
    pub(crate) fn range(self) -> StatRange {
        match self {
            Stat::Fatigue | Stat::Look => StatRange { min: 0.0, max: 1.0 },
            Stat::Sway | Stat::Tremor => StatRange { min: 0.0, max: 4.0 },
            Stat::Pace => StatRange { min: 0.5, max: 1.5 },
        }
    }
}

/// What the player's body does over its locomotion: whether the pistol is out and aimed, how far it has blended in, and the stats the sliders set.
#[derive(Component, Clone, Debug, PartialEq)]
pub(crate) struct Body {
    pub(crate) armed: bool,
    pub(crate) aiming: bool,
    pub(crate) pistol: f32,
    pub(crate) fatigue: f32,
    pub(crate) sway: f32,
    pub(crate) tremor: f32,
    pub(crate) look: f32,
    pub(crate) pace: f32,
}

impl Default for Body {
    fn default() -> Self {
        Self {
            armed: false,
            aiming: false,
            pistol: 0.0,
            fatigue: 0.0,
            sway: 1.0,
            tremor: 0.0,
            look: 1.0,
            pace: 1.0,
        }
    }
}

impl Body {
    /// The value of `stat`.
    pub(crate) fn stat(&self, stat: Stat) -> f32 {
        match stat {
            Stat::Fatigue => self.fatigue,
            Stat::Sway => self.sway,
            Stat::Tremor => self.tremor,
            Stat::Look => self.look,
            Stat::Pace => self.pace,
        }
    }

    /// Sets `stat`, held inside its range.
    pub(crate) fn set_stat(&mut self, stat: Stat, value: f32) {
        let range = stat.range();
        let value = value.clamp(range.min, range.max);
        match stat {
            Stat::Fatigue => self.fatigue = value,
            Stat::Sway => self.sway = value,
            Stat::Tremor => self.tremor = value,
            Stat::Look => self.look = value,
            Stat::Pace => self.pace = value,
        }
    }
}

/// The graph node that plays a shot's kick; goes on the skeleton whose graph has one.
#[derive(Component, Clone, Copy, PartialEq)]
pub(crate) struct Recoil {
    pub(crate) node: AnimationNodeIndex,
}

/// Draws or holsters the pistol on `TogglePistol`, blends it in or out over a quarter of a second, and aims it while it is out and `Aim` is held.
pub(crate) fn update_pistol(
    bodies: Query<&mut Body>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    time: Res<Time>,
) {
    let toggled = actions.is_just_pressed(TogglePistol, &input);
    let aim_held = actions.is_held(Aim, &input);
    let step = time.delta().as_secs_f32() / PISTOL_BLEND;
    for mut body in bodies.iter() {
        let armed = body.armed != toggled;
        let aiming = armed && aim_held;
        let goal = if armed { 1.0 } else { 0.0 };
        let pistol = body.pistol + (goal - body.pistol).clamp(-step, step);
        if armed != body.armed || aiming != body.aiming || pistol != body.pistol {
            body.armed = armed;
            body.aiming = aiming;
            body.pistol = pistol;
        }
    }
}

/// Plays the shot's kick on `Fire` while the player aims.
pub(crate) fn fire_pistol(
    links: Query<&CharacterLink>,
    bodies: Query<&Body>,
    skeletons: Query<(&mut AnimationPlayer, &Recoil)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    input: Res<Input>,
    actions: Res<ActionMap>,
) {
    if !actions.is_just_pressed(Fire, &input) {
        return;
    }
    for link in links.iter() {
        let (Some(capsule), Some(skeleton)) = (link.capsule.entity(), link.skeleton.entity())
        else {
            continue;
        };
        if !bodies.get_entity(capsule).is_some_and(|body| body.aiming) {
            continue;
        }
        if let Some((mut player, recoil)) = skeletons.get_entity(skeleton) {
            player.play(&recoil.node, &graphs);
        }
    }
}

fn set_float(player: &mut AnimationPlayer, param: Name, value: f32) {
    if player.get_float_param(param) != Some(value) {
        player.set_float_param(param, value);
    }
}

/// Writes each body's state to its skeleton's graph params: the pistol and aim weights, the stats, and the point thirty metres along the camera's view as the aim target.
pub(crate) fn feed_body(
    links: Query<&CharacterLink>,
    bodies: Query<&Body>,
    cameras: Query<&GlobalTransform, With<VirtualCamera>>,
    players: Query<&mut AnimationPlayer>,
) {
    let target = cameras
        .iter()
        .next()
        .map(|camera| camera.translation() + camera.rotation() * Vec3::NEG_Z * AIM_DISTANCE);
    for link in links.iter() {
        let (Some(capsule), Some(skeleton)) = (link.capsule.entity(), link.skeleton.entity())
        else {
            continue;
        };
        let Some(body) = bodies.get_entity(capsule) else {
            continue;
        };
        let Some(mut player) = players.get_entity(skeleton) else {
            continue;
        };
        set_float(&mut player, pistol_weight(), body.pistol);
        set_float(
            &mut player,
            aim_weight(),
            if body.aiming { 1.0 } else { 0.0 },
        );
        set_float(&mut player, fatigue(), body.fatigue);
        set_float(&mut player, sway(), body.sway * body.pistol);
        set_float(&mut player, tremor(), body.tremor);
        set_float(&mut player, look_weight(), body.look);
        set_float(&mut player, pace(), body.pace);
        if let Some(target) = target
            && player.get_param::<Vec3>(aim_target()) != Some(&target)
        {
            player.set_param(aim_target(), target);
        }
    }
}

/// Adds the systems that draw, aim and fire the pistol, feed the body's graph params and report footsteps, and the stat sliders.
pub(crate) fn install_body(app: &mut App) {
    app.add_system(Update, update_pistol)
        .add_system(Update, fire_pistol)
        .add_system(Update, feed_body);
    install_footsteps(app);
    app.add_system(Startup, spawn_stat_sliders);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use concerto::animation::player::AnimationPlayer;
    use concerto::director::VirtualCamera;
    use concerto::ecs::component::scene::SceneEntityRef;
    use concerto::ecs::{IntoSystem, System, World};
    use concerto::foundation::time::Time;
    use concerto::foundation::transform::{GlobalTransform, Transform};
    use concerto::motion_matching::CharacterLink;
    use concerto::name::name;
    use concerto::window::input::actions::ActionMap;
    use concerto::window::input::{Input, KeyCode, MouseButton, PhysicalKey};
    use glam::{Mat4, Vec3};

    use super::{Body, Stat, feed_body, update_pistol};
    use crate::character::bind_controls;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn after(world: &mut World, wait: Duration, input: Input) -> Body {
        std::thread::sleep(wait);
        world.get_resource_mut::<Time>().unwrap().update();
        world.insert_resource(input);
        run(world, update_pistol);
        world
            .query::<&Body, ()>()
            .iter(world)
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn g_draws_the_pistol_over_a_blend_and_the_aim_button_aims_it_only_while_it_is_out() {
        let mut world = World::new();
        let mut actions = ActionMap::default();
        bind_controls(&mut actions);
        world.insert_resource(actions);
        world.insert_resource(Time::new());
        world.spawn(Body::default());
        let aim = || {
            let mut input = Input::new();
            input.update_mouse_button(MouseButton::Right, true);
            input
        };

        let holstered = after(&mut world, Duration::from_millis(5), aim());
        assert!(!holstered.armed && !holstered.aiming);
        assert_eq!(holstered.pistol, 0.0);

        let mut draw = Input::new();
        draw.update_key_input(PhysicalKey::Code(KeyCode::KeyG), true);
        let drawing = after(&mut world, Duration::from_millis(5), draw);
        assert!(drawing.armed && !drawing.aiming);
        assert!(drawing.pistol > 0.0 && drawing.pistol < 1.0);

        let aiming = after(&mut world, Duration::from_millis(300), aim());
        assert!(aiming.armed && aiming.aiming);
        assert_eq!(aiming.pistol, 1.0);
    }

    #[test]
    fn the_body_is_fed_to_its_skeleton_with_the_aim_target_along_the_camera_view() {
        let mut world = World::new();
        let capsule = world.spawn(Body {
            armed: true,
            aiming: true,
            pistol: 0.5,
            fatigue: 0.25,
            sway: 2.0,
            tremor: 3.0,
            look: 0.75,
            pace: 1.25,
        });
        let skeleton = world.spawn(AnimationPlayer::new(1));
        world.spawn((
            Transform::default(),
            CharacterLink {
                capsule: SceneEntityRef::Entity(capsule),
                skeleton: SceneEntityRef::Entity(skeleton),
            },
        ));
        world.spawn((
            VirtualCamera::new(0),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0))),
        ));

        run(&mut world, feed_body);

        let player = world
            .get_component_for_entity::<AnimationPlayer>(skeleton)
            .unwrap();
        let float = |param| player.get_float_param(param);
        assert_eq!(float(name!("pistol_weight")), Some(0.5));
        assert_eq!(float(name!("aim_weight")), Some(1.0));
        assert_eq!(float(name!("fatigue")), Some(0.25));
        assert_eq!(float(name!("sway")), Some(1.0));
        assert_eq!(float(name!("tremor")), Some(3.0));
        assert_eq!(float(name!("look_weight")), Some(0.75));
        assert_eq!(float(name!("pace")), Some(1.25));
        assert_eq!(
            player.get_param::<Vec3>(name!("aim_target")),
            Some(&Vec3::new(1.0, 2.0, -27.0))
        );
    }

    #[test]
    fn a_stat_is_held_inside_its_slider_range() {
        let mut body = Body::default();
        for stat in Stat::ALL {
            let range = stat.range();
            body.set_stat(stat, range.max + 1.0);
            assert_eq!(body.stat(stat), range.max);
            body.set_stat(stat, range.min - 1.0);
            assert_eq!(body.stat(stat), range.min);
        }
    }
}
