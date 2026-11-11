use concerto::{
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    director::CameraDirector,
    ecs::{
        CommandQueue, Component, Entity, Query, Res, Without,
        query::change_detection::DetectChanges,
        reflect::{ReflectComponent, ReflectSceneComponent},
    },
    foundation::transform::Transform,
    gameplay::first_person::{FirstPersonView, FirstPersonWalker, spawn_first_person_walker},
    reflect::{Reflect, ReflectDefault},
    render::components::{
        camera::{Camera, Viewport},
        camera_environment::CameraEnvironment,
        visibility::RenderLayers,
    },
    ui::{
        elements::prelude::*,
        material::UIMaterial,
        node::{AlignContent, UINode, UITargetCamera},
        text::UIText,
        theme::UITheme,
        transform::UIValue,
    },
    window::input::{JoinSettings, PlayerIndex, Players},
};
use glam::{EulerRot, Vec2};

const DIVIDER_WIDTH: f32 = 3.0;
const NIGHT_SKY: Color = Color::srgba(0.01, 0.012, 0.03, 1.0);
const NIGHT_AMBIENT: Color = Color::rgba(0.07, 0.085, 0.13, 1.0);

/// How many players share the screen.
pub const PLAYER_COUNT: u8 = 2;

/// Where a player starts: an entity placed in the scene, facing the way the player first looks.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct SpawnPoint {
    /// The player who starts here, from 0.
    pub player: u32,
}

/// A spawn point whose player has been seated.
#[derive(Component)]
pub struct Seated;

/// An entity of a player's seat: their walker, camera or HUD. A new night despawns them and seats the players again.
#[derive(Component)]
pub struct SeatEntity;

#[derive(Component)]
struct SeatLabel {
    player: PlayerIndex,
}

/// The layer `player`'s own body is drawn on, which their camera leaves out.
pub fn body_layer(player: PlayerIndex) -> RenderLayers {
    RenderLayers::layer(1 + player.index())
}

/// What `player`'s camera draws: the world and every body but their own.
pub fn eyes_of(player: PlayerIndex) -> RenderLayers {
    (0..PLAYER_COUNT)
        .filter(|other| *other != player.index())
        .fold(RenderLayers::DEFAULT, |layers, other| {
            layers.with(1 + other)
        })
}

/// The part of the window `player` sees: side by side, the first player on the left.
pub fn viewport_of(player: PlayerIndex) -> Viewport {
    let width = 1.0 / PLAYER_COUNT as f32;
    let left = width * player.index() as f32;
    Viewport::new(Vec2::new(left, 0.0), Vec2::new(left + width, 1.0))
}

/// What a player's half says about them.
pub fn seat_text(player: PlayerIndex, players: &Players) -> String {
    let name = format!("Player {}", player.index() + 1);
    if players.is_waiting(player) {
        format!("{name}: reconnect your controller")
    } else if players.devices(player).is_some() {
        name
    } else {
        format!("{name}: press A, or Enter on the keyboard, to join")
    }
}

/// Split screen for two local players: each spawn point in the scene gets a walker, a camera on its half of the window, and a label.
pub fn install(app: &mut App) {
    app.insert_resource(CameraDirector::default().with_main_camera(false))
        .register_type::<SpawnPoint>()
        .add_system(Startup, divide_the_window)
        .add_system(Update, seat_players)
        .add_system(Update, update_seat_labels);
}

/// Turns joining on; call it after the default plugins, which insert `Players`.
pub fn enable_joining(app: &mut App) {
    app.get_resource_mut::<Players>()
        .expect("DefaultPlugins inserts Players")
        .set_settings(
            JoinSettings::new()
                .with_max_players(PLAYER_COUNT)
                .with_join_on_press(true)
                .with_keyboard_mouse_joins(true),
        );
}

fn divide_the_window(mut cmd: CommandQueue) {
    let root = cmd
        .spawn(
            UINode::default()
                .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                .with_justify_content(AlignContent::SpaceEvenly),
        )
        .entity();
    for _ in 1..PLAYER_COUNT {
        cmd.entity(root).add_child((
            UINode::default().with_size(UIValue::Px(DIVIDER_WIDTH), UIValue::Percent(100.0)),
            UIMaterial::flat(Color::srgba(0.0, 0.0, 0.0, 1.0)),
        ));
    }
}

fn seat_players(
    points: Query<(Entity, &SpawnPoint, &Transform), Without<Seated>>,
    players: Res<Players>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (point, spawn, transform) in points.iter() {
        cmd.insert(Seated, point);
        let Ok(index) = u8::try_from(spawn.player) else {
            continue;
        };
        if index >= PLAYER_COUNT {
            continue;
        }
        let player = PlayerIndex::new(index);
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        let walker = spawn_first_person_walker(
            &mut cmd,
            player,
            Transform::from_translation(transform.translation),
            FirstPersonWalker::default().with_yaw(yaw),
        );
        cmd.insert(SeatEntity, walker);
        let camera = cmd
            .spawn((
                Camera {
                    clear_color: NIGHT_SKY,
                    ..Camera::default().with_viewport(viewport_of(player))
                },
                CameraEnvironment::default().with_ambient(NIGHT_AMBIENT),
                eyes_of(player),
                Transform::from_translation(transform.translation),
                FirstPersonView { walker },
                player,
                SeatEntity,
            ))
            .entity();
        cmd.spawn((
            UINode::default().with_size(UIValue::Percent(100.0), UIValue::Percent(100.0)),
            UITargetCamera(camera),
            SeatEntity,
        ))
        .add_child((
            theme
                .label(seat_text(player, &players))
                .large()
                .margin(theme.spacing_md),
            SeatLabel { player },
        ));
    }
}

fn update_seat_labels(players: Res<Players>, labels: Query<(&SeatLabel, &mut UIText)>) {
    if !players.is_changed() {
        return;
    }
    for (label, mut text) in labels.iter() {
        let wanted = seat_text(label.player, &players);
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto::{
        app::{App, StartupScene},
        asset_id,
        ecs::Entity,
        foundation::transform::Transform,
        gameplay::first_person::{FirstPersonView, FirstPersonWalker},
        render::components::camera::{Camera, Viewport},
        ui::{node::UITargetCamera, text::UIText},
        window::input::{GamepadId, PlayerDevice, PlayerIndex, Players},
    };
    use glam::{Quat, Vec3};

    use super::{SeatLabel, SpawnPoint, viewport_of};

    struct Seat {
        player: u8,
        walker: Entity,
        yaw: f32,
        at: Vec3,
    }

    struct Eye {
        camera: Entity,
        viewport: Option<Viewport>,
        walker: Entity,
    }

    fn seats(app: &mut App) -> Vec<Seat> {
        let world = app.main_mut().world_mut();
        let mut walkers =
            world.query::<(Entity, &PlayerIndex, &FirstPersonWalker, &Transform), ()>();
        let mut seats: Vec<Seat> = walkers
            .iter(world)
            .map(|(walker, player, settings, transform)| Seat {
                player: player.index(),
                walker,
                yaw: settings.yaw(),
                at: transform.translation,
            })
            .collect();
        seats.sort_by_key(|seat| seat.player);
        seats
    }

    fn eyes(app: &mut App) -> Vec<Eye> {
        let world = app.main_mut().world_mut();
        let mut cameras = world.query::<(Entity, &Camera, &FirstPersonView), ()>();
        cameras
            .iter(world)
            .map(|(camera, settings, view)| Eye {
                camera,
                viewport: settings.viewport,
                walker: view.walker,
            })
            .collect()
    }

    fn labels(app: &mut App) -> Vec<String> {
        let world = app.main_mut().world_mut();
        let mut labels = world.query::<(&SeatLabel, &UIText), ()>();
        let mut found: Vec<&SeatLabel> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for (label, text) in labels.iter(world) {
            let before = found
                .iter()
                .filter(|other| other.player < label.player)
                .count();
            found.insert(before, label);
            texts.insert(before, text.text.clone());
        }
        texts
    }

    fn seats_alone(app: &mut App) {
        super::install(app);
        app.register_plugin(concerto::DefaultPlugins);
        super::enable_joining(app);
    }

    #[test]
    fn the_game_starts_in_the_test_level() {
        let mut app = App::new();
        crate::game(&mut app);

        assert_eq!(
            app.get_resource::<StartupScene>(),
            Some(&StartupScene(asset_id!("content/level/test_level.gasset")))
        );
    }

    #[test]
    fn each_spawn_point_seats_its_player_in_their_own_half_with_a_label_that_follows_joining() {
        let Some(mut app) = concerto::headless(seats_alone) else {
            return;
        };
        let world = app.main_mut().world_mut();
        let starts = [Vec3::new(-2.0, 0.0, 1.0), Vec3::new(2.0, 0.0, 1.0)];
        for (player, at) in starts.into_iter().enumerate() {
            world.spawn((
                SpawnPoint {
                    player: player as u32,
                },
                Transform::from_translation_rotation(at, Quat::from_rotation_y(1.0)),
            ));
        }
        world.spawn((SpawnPoint { player: 7 }, Transform::default()));

        app.update();
        app.update();

        let seats = seats(&mut app);
        assert_eq!(
            seats.iter().map(|seat| seat.player).collect::<Vec<_>>(),
            [0, 1],
            "one walker per player, and none for a spawn point past the last player"
        );
        let eyes = eyes(&mut app);
        assert_eq!(eyes.len(), 2);
        let world = app.main_mut().world_mut();
        let mut roots = world.query::<&UITargetCamera, ()>();
        let roots: Vec<Entity> = roots.iter(world).map(|target| target.0).collect();
        for seat in &seats {
            assert!((seat.yaw - 1.0).abs() < 1e-4, "it faces as its spawn point");
            assert!(
                (seat.at.with_y(0.0) - starts[seat.player as usize]).length() < 0.05,
                "player {} starts at their spawn point, not {}",
                seat.player,
                seat.at
            );
            let eye = eyes
                .iter()
                .find(|eye| eye.walker == seat.walker)
                .expect("each walker has a camera at its eyes");
            assert_eq!(
                eye.viewport,
                Some(viewport_of(PlayerIndex::new(seat.player)))
            );
            assert!(
                roots.contains(&eye.camera),
                "each camera has a UI root in its half"
            );
        }
        assert_eq!(
            labels(&mut app),
            [
                "Player 1: press A, or Enter on the keyboard, to join",
                "Player 2: press A, or Enter on the keyboard, to join"
            ]
        );

        app.get_resource_mut::<Players>()
            .unwrap()
            .assign(
                PlayerIndex::new(1),
                PlayerDevice::Gamepad(GamepadId::new(4)),
            )
            .unwrap();
        app.update();

        assert_eq!(
            labels(&mut app),
            [
                "Player 1: press A, or Enter on the keyboard, to join",
                "Player 2"
            ]
        );
    }
}
