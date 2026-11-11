use std::path::PathBuf;

use concerto::{
    animation::{
        clip::AnimationClip,
        graph::AnimationGraph,
        player::{AnimationHandleComponent, AnimationPlayer},
    },
    app::{
        App,
        schedule_groups::{FixedUpdate, Startup, Update},
    },
    asset_id,
    color::Color,
    ecs::{
        CommandQueue, Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, With,
        Without,
        entity::hierarchy::Children,
        events::event_reader::EventReader,
        reflect::{ReflectComponent, ReflectSceneComponent},
    },
    foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        time::Time,
        transform::Transform,
    },
    gameplay::first_person::{FirstPersonView, FirstPersonWalker},
    mesh::mesh::{Mesh, MeshComponent},
    name::name,
    physics::{
        body::BodyId,
        collider::{BoxCollider, Collider, ColliderOffset},
        physics_state::PhysicsState,
        query::QueryFilter,
        rigid_body::{AllowedDofs, MotionType, RigidBody},
    },
    reflect::{Reflect, ReflectDefault},
    render::{
        assets::material::StandardMaterial,
        components::{camera::Camera, material::MaterialComponent},
    },
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    ui::{
        material::UIMaterial,
        node::{FlexDirection, Position, UIInset, UINode, UIRect, UITargetCamera},
        text::UIText,
        transform::UIValue,
    },
    window::input::{Input, PlayerIndex, Players, actions::ActionMap},
};
use glam::{Quat, Vec2, Vec3};

use crate::book::Casebook;
use crate::case::{Id, NpcId, Phase, PlacementId, Role};
use crate::casefile::CaseData;
use crate::clock::{GameTime, Midnight, NightClock, NightPaused};
use crate::controls::{
    Back, Confirm, Interact, Listen, MenuDown, MenuLeft, MenuRight, MenuUp, OpenBook, Watch,
    bind_controls,
};
use crate::house::{House, Interactable, ItemPlace, end_night};
use crate::players::{PLAYER_COUNT, SeatEntity, Seated};
use crate::rules::name_of;
use crate::save::{SaveFile, save_dir};
use crate::schedule::{NpcPose, WALK_SPEED, case_time, entries_of, line_at, pose_at, talks_at};
use crate::session::{Effect, Press, Session};

const SEATS: usize = PLAYER_COUNT as usize;
const REACH: f32 = 2.6;
const AIM_RADIUS: f32 = 0.12;
const EARSHOT: f32 = 6.0;
const DOOR_LISTEN_REACH: f32 = 1.6;
const NPC_CATCH_UP: f32 = 3.2;
const NPC_SNAP: f32 = 5.0;
const SCREEN_ROWS: usize = 12;
const MODEL_TURN: f32 = std::f32::consts::PI;
const GAIT_IDLE: Vec2 = Vec2::ZERO;
const GAIT_WALK: Vec2 = Vec2::new(0.0, 1.0);
const GAIT_SIT: Vec2 = Vec2::new(1.0, 0.0);
const CHARACTER_SCENE: concerto::foundation::assets::AssetId =
    asset_id!("content/characters/UAL1/scene.gasset");
const IDLE_CLIP: concerto::foundation::assets::AssetId =
    asset_id!("content/characters/UAL1/animation_Idle_Loop.gasset");
const WALK_CLIP: concerto::foundation::assets::AssetId =
    asset_id!("content/characters/UAL1/animation_Walk_Loop.gasset");
const SIT_CLIP: concerto::foundation::assets::AssetId =
    asset_id!("content/characters/UAL1/animation_Sitting_Idle_Loop.gasset");
const CUBE_MESH: concerto::foundation::assets::AssetId =
    asset_id!("content/graybox/cube/mesh_0.gasset");
const CUBE_MATERIAL: concerto::foundation::assets::AssetId =
    asset_id!("content/graybox/cube/material_cube.gasset");

/// What a scene object is to the game.
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixtureKind {
    #[default]
    Door,
    BellPull,
    BellBoard,
    Gong,
    Bed,
    Lift,
}

/// Marks a scene entity as a door, a bell pull, the bell board, the gong, a bed or a dumbwaiter stop; `name` is the door or room id, the bed's player or the stop's number.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct Fixture {
    pub kind: FixtureKind,
    pub name: String,
}

#[derive(Component)]
struct Adopted;

#[derive(Component)]
struct DoorPanel {
    door: crate::case::DoorId,
    closed: Transform,
    shown_open: bool,
}

#[derive(Component)]
struct NpcBody {
    npc: NpcId,
    yaw: f32,
    walking: bool,
    sitting: bool,
}

#[derive(Component)]
struct NpcVisual {
    body: Entity,
    animator: Option<Entity>,
}

#[derive(Resource)]
struct NpcAnimations {
    graph: AssetHandle<AnimationGraph>,
}

#[derive(Component)]
struct ItemBody {
    placement: PlacementId,
    shown: Option<ItemPlace>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HudPart {
    Info,
    Screen,
    Prompt,
    Subtitle,
}

#[derive(Component)]
struct HudText {
    player: u8,
    part: HudPart,
}

#[derive(Component)]
struct HudBuilt;

/// What each player looks at, what each overhears, and whether the night is to end this frame.
#[derive(Resource, Default)]
pub struct Sight {
    pub targets: [Option<Interactable>; SEATS],
    pub heard: [Option<String>; SEATS],
    pub night_over: bool,
    was_present: [bool; SEATS],
}

/// Where the game saves at each loop boundary; nowhere when `None`.
#[derive(Resource, Default)]
pub struct SaveSlot(pub Option<PathBuf>);

/// Holds the night clock still from the debug keys.
#[derive(Resource, Default)]
pub struct DebugPause(pub bool);

/// Adds the night's mechanics over the test case: the house, its characters and props, each player's screen and HUD, and the loop.
pub fn install(app: &mut App, saves: Option<PathBuf>) {
    let data = CaseData::test();
    let mut session = Session::default();
    let mut book = Casebook::default();
    if let Some(save) = saves.as_deref().and_then(SaveFile::read) {
        let restored = save.restore(&data);
        book = restored.book;
        session.roles = restored.roles;
    }
    session.show_card(book.loops() + 1, Vec::new());
    bind_controls(
        app.get_resource_mut::<ActionMap>()
            .expect("DefaultPlugins inserts the ActionMap"),
    );
    app.insert_resource(House::new(&data))
        .insert_resource(data)
        .insert_resource(book)
        .insert_resource(session)
        .insert_resource(Sight::default())
        .insert_resource(SaveSlot(saves))
        .insert_resource(DebugPause::default())
        .register_type::<Fixture>()
        .add_system(Startup, spawn_case)
        .add_system(Update, adopt_fixtures)
        .add_system(Update, build_huds)
        .add_system(Update, find_targets)
        .add_system(Update, play.after(find_targets))
        .add_system(Update, overhear.after(play))
        .add_system(Update, turn_night.after(overhear))
        .add_system(Update, show_hud.after(turn_night))
        .add_system(Update, swing_doors.after(play))
        .add_system(Update, place_items.after(play))
        .add_system(Update, face_npcs)
        .add_system(Update, pause_for_screens.after(play))
        .add_system(FixedUpdate, move_npcs);
    #[cfg(feature = "debug")]
    app.add_system(Update, debug_keys.before(turn_night));
}

/// The folder the standalone game saves in.
pub fn default_saves() -> Option<PathBuf> {
    Some(save_dir())
}

fn now_seconds(clock: &NightClock) -> f64 {
    clock.now().seconds()
}

fn npc_pose(data: &CaseData, house: &House, npc: NpcId, now: f64) -> Option<NpcPose> {
    if house.gathered
        && let Some(staging) = data
            .case
            .finale
            .staging
            .iter()
            .find(|staging| staging.npc == npc)
    {
        let anchor = data.level.anchor(staging.anchor);
        return Some(NpcPose {
            position: anchor.position.with_y(0.0),
            yaw: anchor.yaw,
            entry: 0,
            walking: false,
        });
    }
    let entries = entries_of(&data.case, npc, Phase::Before);
    pose_at(&entries, &data.level, now, WALK_SPEED)
}

fn npc_is_busy(data: &CaseData, house: &House, npc: NpcId, now: f64) -> bool {
    let entries = entries_of(&data.case, npc, Phase::Before);
    npc_pose(data, house, npc, now)
        .and_then(|pose| entries.get(pose.entry).map(|entry| entry.critical))
        .unwrap_or(false)
}

fn prop_size(data: &CaseData, placement: PlacementId) -> Vec3 {
    match data
        .catalog
        .props
        .name(data.case.placements.get(placement).prop)
    {
        "prop_case" => Vec3::new(0.5, 0.35, 0.3),
        "prop_note" => Vec3::new(0.22, 0.02, 0.3),
        "prop_basket" => Vec3::new(0.6, 0.45, 0.45),
        "prop_key" => Vec3::new(0.06, 0.14, 0.03),
        _ => Vec3::new(0.12, 0.12, 0.12),
    }
}

fn item_home(data: &CaseData, placement: PlacementId) -> Vec3 {
    let anchor = data
        .level
        .anchor(data.case.placements.get(placement).anchor);
    anchor.position + Vec3::Y * prop_size(data, placement).y / 2.0
}

fn spawn_case(
    data: Res<CaseData>,
    house: Res<House>,
    clock: Res<NightClock>,
    assets: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let mesh = assets.load::<Mesh>(CUBE_MESH);
    let material = assets.load::<StandardMaterial>(CUBE_MATERIAL);
    let character = assets.load::<Scene>(CHARACTER_SCENE);
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .with_blend_space_2d_input(name!("gait"), |space| {
            space
                .animation_clip_input(assets.load::<AnimationClip>(IDLE_CLIP), GAIT_IDLE)
                .animation_clip_input(assets.load::<AnimationClip>(WALK_CLIP), GAIT_WALK)
                .animation_clip_input(assets.load::<AnimationClip>(SIT_CLIP), GAIT_SIT);
        });
    cmd.insert_resource(NpcAnimations {
        graph: assets.add(graph),
    });
    for npc in data.case.characters.ids() {
        let Some(pose) = npc_pose(&data, &house, npc, now_seconds(&clock)) else {
            continue;
        };
        let collider = Collider::capsule(0.55, 0.3);
        let offset = ColliderOffset::bottom_origin(&collider);
        let body = cmd
            .spawn((
                NpcBody {
                    npc,
                    yaw: pose.yaw,
                    walking: false,
                    sitting: false,
                },
                Interactable::Npc(npc),
                RigidBody {
                    density: 1000.0,
                    allowed_dofs: AllowedDofs::TRANSLATION,
                    motion_type: MotionType::Kinematic,
                },
                collider,
                offset,
                Transform::from_translation(pose.position),
            ))
            .entity();
        cmd.spawn((
            NpcVisual {
                body,
                animator: None,
            },
            SceneSpawnerComponent(character.clone()),
            Transform::from_translation_rotation(
                pose.position,
                Quat::from_rotation_y(pose.yaw + MODEL_TURN),
            ),
        ));
    }
    for placement in data.case.placements.ids() {
        cmd.spawn((
            ItemBody {
                placement,
                shown: None,
            },
            Interactable::Placement(placement),
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent {
                handle: material.clone(),
            },
            Transform::from_translation_rotation_scale(
                item_home(&data, placement),
                Quat::IDENTITY,
                prop_size(&data, placement),
            ),
            BoxCollider::default(),
        ));
    }
}

fn adopt_fixtures(
    fixtures: Query<(Entity, &Fixture, &Transform), Without<Adopted>>,
    mut data: ResMut<CaseData>,
    mut cmd: CommandQueue,
) {
    for (entity, fixture, transform) in fixtures.iter() {
        cmd.insert(Adopted, entity);
        let number = fixture.name.parse::<u8>().ok();
        let target = match fixture.kind {
            FixtureKind::Door => data.catalog.doors.id(&fixture.name).map(|door| {
                data.level.place_door(door, transform.translation);
                cmd.insert(
                    DoorPanel {
                        door,
                        closed: transform.clone(),
                        shown_open: false,
                    },
                    entity,
                );
                Interactable::Door(door)
            }),
            FixtureKind::BellPull => data
                .catalog
                .rooms
                .id(&fixture.name)
                .map(Interactable::BellPull),
            FixtureKind::BellBoard => Some(Interactable::BellBoard),
            FixtureKind::Gong => Some(Interactable::Gong),
            FixtureKind::Bed => number.map(Interactable::Bed),
            FixtureKind::Lift => number.map(Interactable::Lift),
        };
        match target {
            Some(target) => cmd.insert(target, entity),
            None => log::warn!(
                "the scene's {:?} fixture names `{}`, which the level does not have",
                fixture.kind,
                fixture.name
            ),
        }
    }
}

fn hud_text(size: f32) -> UIText {
    UIText {
        font_size: size,
        line_height: size * 1.35,
        ..Default::default()
    }
}

fn build_huds(
    cameras: Query<(Entity, &PlayerIndex), (With<Camera>, Without<HudBuilt>)>,
    mut cmd: CommandQueue,
) {
    for (camera, player) in cameras.iter() {
        let player = player.index();
        cmd.insert(HudBuilt, camera);
        let text = |part: HudPart, size: f32| {
            (UINode::default(), hud_text(size), HudText { player, part })
        };
        let root = cmd
            .spawn((
                UINode::default()
                    .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                    .with_flex_direction(FlexDirection::Column)
                    .with_padding(UIRect::all(12.0)),
                UITargetCamera(camera),
                SeatEntity,
            ))
            .entity();
        cmd.entity(root)
            .add_child(UINode::default().with_size(UIValue::Percent(100.0), UIValue::Px(34.0)));
        cmd.entity(root).add_child(text(HudPart::Info, 16.0));
        cmd.entity(root).add_child((
            UINode::default()
                .with_padding(UIRect::all(14.0))
                .with_visible(false),
            UIMaterial {
                corner_radius: 8.0,
                ..UIMaterial::flat(Color::srgba(0.04, 0.04, 0.07, 0.88))
            },
            hud_text(19.0),
            HudText {
                player,
                part: HudPart::Screen,
            },
        ));
        cmd.entity(root)
            .add_child(UINode::default().with_flex_grow(1.0));
        cmd.entity(root).add_child(text(HudPart::Prompt, 18.0));
        cmd.entity(root).add_child(text(HudPart::Subtitle, 20.0));
        cmd.entity(root).add_child((
            UINode::default()
                .with_position(Position::Absolute)
                .with_inset(UIInset {
                    top: UIValue::Percent(50.0),
                    left: UIValue::Percent(50.0),
                    right: UIValue::Auto,
                    bottom: UIValue::Auto,
                }),
            UIText {
                text: "+".into(),
                ..hud_text(22.0)
            },
        ));
    }
}

fn find_targets(
    cameras: Query<(&PlayerIndex, &Transform, &FirstPersonView), With<Camera>>,
    bodies: Query<&BodyId>,
    targets: Query<&Interactable>,
    physics: Res<PhysicsState>,
    house: Res<House>,
    mut sight: ResMut<Sight>,
) {
    let mut seen = [None; SEATS];
    for (player, transform, view) in cameras.iter() {
        let own = bodies.get_entity(view.walker).copied();
        let filter = QueryFilter {
            ignore: own.as_slice(),
        };
        let reach = transform.rotation * Vec3::NEG_Z * REACH;
        seen[player.index() as usize] = physics
            .cast_sphere(transform.translation, AIM_RADIUS, reach, filter)
            .and_then(|hit| hit.entity)
            .and_then(|entity| targets.get_entity(entity))
            .copied()
            .filter(|target| match target {
                Interactable::Placement(placement) => house.in_the_open(*placement),
                _ => true,
            });
    }
    if sight.targets != seen {
        sight.targets = seen;
    }
}

fn play(
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    mut session: ResMut<Session>,
    mut book: ResMut<Casebook>,
    mut house: ResMut<House>,
    data: Res<CaseData>,
    clock: Res<NightClock>,
    mut sight: ResMut<Sight>,
    cameras: Query<(&PlayerIndex, &Transform), With<Camera>>,
    walkers: Query<(&PlayerIndex, &mut FirstPersonWalker)>,
    physics: Res<PhysicsState>,
) {
    let now = now_seconds(&clock);
    let time = case_time(now);
    let mut effects = Vec::new();
    for player in 0..PLAYER_COUNT {
        let seat = player as usize;
        let index = PlayerIndex::new(player);
        let present = players.devices(index).is_some();
        let joined_now = present && !sight.was_present[seat];
        sight.was_present[seat] = present;
        if session.seats[seat].present != present {
            session.seats[seat].present = present;
        }
        if !present || joined_now {
            continue;
        }
        let pad = actions.player(index, &input, &players);
        if session.is_walking(player) {
            if pad.is_just_pressed(OpenBook) {
                session.open_book(player);
            } else if pad.is_just_pressed(Interact)
                && let Some(target) = sight.targets[seat]
            {
                let busy = match target {
                    Interactable::Npc(npc) => npc_is_busy(&data, &house, npc, now),
                    _ => false,
                };
                effects.extend(house.use_target(
                    &mut session,
                    &mut book,
                    &data,
                    player,
                    target,
                    time,
                    busy,
                ));
            } else if pad.is_just_pressed(Back)
                && session.seats[seat].carried.is_some()
                && let Some((_, eye)) = cameras.iter().find(|(camera, _)| camera.index() == player)
            {
                let ahead = eye.translation + (eye.rotation * Vec3::NEG_Z).with_y(0.0) * 0.9;
                let floor = physics
                    .cast_ray(ahead, Vec3::NEG_Y * 3.0, QueryFilter::default())
                    .map_or(ahead.with_y(0.0), |hit| hit.point);
                house.put_down(&mut session, player, floor + Vec3::Y * 0.08);
            }
            continue;
        }
        let presses = [
            (pad.is_just_pressed(MenuUp), Press::Up),
            (pad.is_just_pressed(MenuDown), Press::Down),
            (pad.is_just_pressed(MenuLeft), Press::Left),
            (pad.is_just_pressed(MenuRight), Press::Right),
            (pad.is_just_pressed(Confirm), Press::Confirm),
            (pad.is_just_pressed(Back), Press::Back),
            (pad.is_just_pressed(OpenBook), Press::Book),
        ];
        for (pressed, press) in presses {
            if pressed {
                effects.extend(session.press(player, press, &mut book, &data, time, Phase::Before));
            }
        }
    }
    if session.finale.is_none() && house.gathered {
        house.gathered = false;
    }
    if effects.contains(&Effect::EndNight) {
        sight.night_over = true;
    }
    for (player, mut walker) in walkers.iter() {
        let free =
            session.is_walking(player.index()) && !session.seats[player.index() as usize].in_bed;
        if walker.controlled != free {
            walker.controlled = free;
        }
    }
}

fn overhear(
    data: Res<CaseData>,
    clock: Res<NightClock>,
    house: Res<House>,
    mut session: ResMut<Session>,
    mut book: ResMut<Casebook>,
    mut sight: ResMut<Sight>,
    npcs: Query<(&NpcBody, &Transform)>,
    walkers: Query<(&PlayerIndex, &Transform), With<FirstPersonWalker>>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    time: Res<Time>,
) {
    session.age_notices(time.real_delta().as_secs_f32());
    let mut heard: [Option<String>; SEATS] = Default::default();
    let listeners: Vec<(u8, Vec3)> = walkers
        .iter()
        .map(|(player, transform)| (player.index(), transform.translation))
        .filter(|(player, _)| session.seats[*player as usize].present)
        .collect();
    if !house.gathered {
        for talk in talks_at(&data.case, Phase::Before, now_seconds(&clock)) {
            let conversation = data.case.conversations.get(talk.conversation);
            let Some(place) = npcs
                .iter()
                .find(|(body, _)| conversation.participants.contains(&body.npc))
                .map(|(_, transform)| transform.translation)
            else {
                continue;
            };
            let room = data.level.room_at(place);
            let earshot = conversation.earshot.unwrap_or(EARSHOT);
            let guest_in_sight = listeners.iter().any(|(player, at)| {
                session.role(*player) == Role::Guest && data.level.room_at(*at) == room
            });
            let covered = conversation.private && guest_in_sight;
            let lines = if covered {
                &conversation.cover
            } else {
                &conversation.lines
            };
            let seconds: Vec<f32> = lines.iter().map(|line| line.seconds).collect();
            let Some(line) = line_at(&seconds, talk.elapsed, covered).map(|line| &lines[line])
            else {
                continue;
            };
            for (player, at) in &listeners {
                let near = data.level.room_at(*at) == room && at.distance(place) <= earshot;
                let at_door = session.role(*player) == Role::Staff
                    && actions
                        .player(PlayerIndex::new(*player), &input, &players)
                        .is_held(Listen)
                    && data.level.doors().any(|door| {
                        let rule = data.level.door(door);
                        data.level.door_position(door).is_some_and(|position| {
                            (Some(rule.a) == room || Some(rule.b) == room)
                                && position.distance(*at) <= DOOR_LISTEN_REACH
                                && position.distance(place) <= earshot * 1.5
                        })
                    });
                if near || at_door {
                    heard[*player as usize] = Some(format!(
                        "{}: {}",
                        name_of(&data.case, line.speaker),
                        data.text(line.text)
                    ));
                    session.grant(&mut book, &data, &line.grants);
                }
            }
        }
    }
    if sight.heard != heard {
        sight.heard = heard;
    }
}

fn turn_night(
    mut midnight: EventReader<Midnight>,
    mut sight: ResMut<Sight>,
    mut house: ResMut<House>,
    mut session: ResMut<Session>,
    mut book: ResMut<Casebook>,
    mut clock: ResMut<NightClock>,
    data: Res<CaseData>,
    slot: Res<SaveSlot>,
    seated: Query<Entity, With<SeatEntity>>,
    points: Query<Entity, With<Seated>>,
    mut cmd: CommandQueue,
) {
    let struck = midnight.read().count() > 0;
    if !struck && !sight.night_over {
        return;
    }
    sight.night_over = false;
    if session.finished {
        return;
    }
    end_night(&mut house, &mut session, &mut book, &data);
    clock.restart();
    if let Some(dir) = &slot.0
        && let Err(error) = SaveFile::of(&book, &session, &data).write(dir)
    {
        log::warn!("could not save to {}: {error}", dir.display());
    }
    for entity in seated.iter() {
        cmd.despawn(entity);
    }
    for point in points.iter() {
        cmd.remove::<Seated>(point);
    }
}

fn pause_for_screens(
    session: Res<Session>,
    debug: Res<DebugPause>,
    mut paused: ResMut<NightPaused>,
) {
    let hold = session.card.is_some() || session.finale.is_some() || debug.0;
    if paused.0 != hold {
        paused.0 = hold;
    }
}

fn readout(time: GameTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn show_hud(
    session: Res<Session>,
    book: Res<Casebook>,
    data: Res<CaseData>,
    house: Res<House>,
    clock: Res<NightClock>,
    sight: Res<Sight>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    texts: Query<(&HudText, &mut UIText, &mut UINode)>,
) {
    let time = case_time(now_seconds(&clock));
    let screens: [Option<String>; SEATS] = std::array::from_fn(|seat| {
        session
            .view(seat as u8, &book, &data, time, Phase::Before)
            .map(|screen| screen.laid_out(SCREEN_ROWS))
    });
    for (hud, mut text, mut node) in texts.iter() {
        if hud.part == HudPart::Screen {
            let shown = screens[hud.player as usize].is_some();
            if node.visible != shown {
                node.visible = shown;
            }
        }
        let seat = &session.seats[hud.player as usize];
        let wanted = match hud.part {
            HudPart::Screen => screens[hud.player as usize].clone().unwrap_or_default(),
            HudPart::Info => {
                if !seat.present {
                    String::new()
                } else {
                    let mut info = format!("You are {}.", session.role_title(hud.player));
                    if let Some(carried) = seat.carried {
                        info.push_str(&format!(
                            "  Carrying: {} (B to put down).",
                            data.text(data.case.placements.get(carried).name)
                        ));
                    }
                    if seat.in_bed {
                        info.push_str("  In bed.");
                    }
                    let watch = actions
                        .player(PlayerIndex::new(hud.player), &input, &players)
                        .is_held(Watch);
                    if watch {
                        info.push_str(&format!("  Pocket watch: {}", readout(clock.now())));
                    }
                    info
                }
            }
            HudPart::Prompt => match sight.targets[hud.player as usize] {
                Some(target) if seat.present && session.is_walking(hud.player) => {
                    format!("X: {}", house.prompt(&session, &data, hud.player, target))
                }
                _ => String::new(),
            },
            HudPart::Subtitle => {
                let mut lines: Vec<&str> = Vec::new();
                if let Some(heard) = &sight.heard[hud.player as usize] {
                    lines.push(heard);
                }
                if let Some(told) = &seat.told {
                    lines.push(&told.text);
                }
                if let Some(notice) = session.notices.last() {
                    lines.push(&notice.text);
                }
                lines.join("\n")
            }
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

fn swing_doors(
    panels: Query<(Entity, &mut DoorPanel, &mut Transform)>,
    house: Res<House>,
    mut cmd: CommandQueue,
) {
    for (entity, mut panel, mut transform) in panels.iter() {
        let open = house.doors[panel.door.index()].open;
        if panel.shown_open == open {
            continue;
        }
        panel.shown_open = open;
        let closed = panel.closed.clone();
        let half = Vec3::X * closed.scale.x / 2.0;
        let (rotation, translation) = if open {
            let hinge = closed.translation - closed.rotation * half;
            let rotation = closed.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
            (rotation, hinge + rotation * half)
        } else {
            (closed.rotation, closed.translation)
        };
        transform.rotation = rotation;
        transform.translation = translation;
        cmd.insert(BoxCollider::default(), entity);
    }
}

fn place_items(
    items: Query<(Entity, &mut ItemBody, &mut Transform)>,
    lifts: Query<(&Interactable, &Transform), Without<ItemBody>>,
    house: Res<House>,
    data: Res<CaseData>,
    mut cmd: CommandQueue,
) {
    for (entity, mut item, mut transform) in items.iter() {
        let place = house.items[item.placement.index()];
        let lift_at = house.lift_at;
        let shown = (place, lift_at);
        if item.shown == Some(place) && place != ItemPlace::Lift {
            continue;
        }
        let translation = match place {
            ItemPlace::Home => item_home(&data, item.placement),
            ItemPlace::Dropped(at) => at + Vec3::Y * prop_size(&data, item.placement).y / 2.0,
            ItemPlace::Carried(_) => Vec3::new(0.0, -40.0, 0.0),
            ItemPlace::Lift => lifts
                .iter()
                .find(|(target, _)| **target == Interactable::Lift(shown.1))
                .map_or(Vec3::new(0.0, -40.0, 0.0), |(_, lift)| {
                    lift.translation + Vec3::Y * (lift.scale.y / 2.0 + 0.1)
                }),
        };
        let moved = transform.translation.distance(translation) > 1e-4;
        if item.shown != Some(place) {
            item.shown = Some(place);
        }
        if moved {
            transform.translation = translation;
            cmd.insert(BoxCollider::default(), entity);
        }
    }
}

fn move_npcs(
    npcs: Query<(&mut NpcBody, &BodyId)>,
    data: Res<CaseData>,
    house: Res<House>,
    session: Res<Session>,
    clock: Res<NightClock>,
    mut physics: ResMut<PhysicsState>,
) {
    let dt = Time::fixed_delta_time().as_secs_f32();
    let now = now_seconds(&clock);
    for (mut npc, body) in npcs.iter() {
        let held = (0..PLAYER_COUNT).any(|player| session.talking_to(player) == Some(npc.npc));
        let Some(pose) = npc_pose(&data, &house, npc.npc, now).filter(|_| !held) else {
            physics.set_linear_velocity(*body, Vec3::ZERO);
            continue;
        };
        let at = physics.body_transform(*body).translation;
        let step = (pose.position - at).with_y(0.0);
        let velocity = if step.length() > NPC_SNAP {
            step / dt
        } else {
            (step / dt).clamp_length_max(NPC_CATCH_UP)
        };
        physics.set_linear_velocity(*body, velocity);
        if (npc.yaw - pose.yaw).abs() > 1e-4 {
            npc.yaw = pose.yaw;
        }
        let moving = pose.walking || step.length() > 0.05;
        let sitting = !moving && !house.gathered && {
            let entries = entries_of(&data.case, npc.npc, Phase::Before);
            entries
                .get(pose.entry)
                .is_some_and(|entry| data.catalog.activities.name(entry.activity) == "sit")
        };
        if npc.walking != moving {
            npc.walking = moving;
        }
        if npc.sitting != sitting {
            npc.sitting = sitting;
        }
    }
}

fn animator_under(
    root: Entity,
    children: &Query<&Children>,
    players: &Query<&mut AnimationPlayer>,
) -> Option<Entity> {
    let mut open = vec![root];
    while let Some(entity) = open.pop() {
        if players.contains_entity(entity) {
            return Some(entity);
        }
        if let Some(below) = children.get_entity(entity) {
            open.extend(below.iter().copied());
        }
    }
    None
}

fn face_npcs(
    visuals: Query<(Entity, &mut NpcVisual, &mut Transform)>,
    bodies: Query<(&NpcBody, &Transform), Without<NpcVisual>>,
    children: Query<&Children>,
    players: Query<&mut AnimationPlayer>,
    animations: Option<Res<NpcAnimations>>,
    mut cmd: CommandQueue,
) {
    for (entity, mut visual, mut transform) in visuals.iter() {
        let Some((npc, body)) = bodies.get_entity(visual.body) else {
            continue;
        };
        transform.translation = body.translation;
        transform.rotation = Quat::from_rotation_y(npc.yaw + MODEL_TURN);
        if visual.animator.is_none()
            && let Some(animations) = &animations
            && let Some(animator) = animator_under(entity, &children, &players)
        {
            visual.animator = Some(animator);
            cmd.insert(
                AnimationHandleComponent::new(animations.graph.clone()),
                animator,
            );
        }
        if let Some(mut player) = visual
            .animator
            .and_then(|player| players.get_entity(player))
        {
            let gait = if npc.walking {
                GAIT_WALK
            } else if npc.sitting {
                GAIT_SIT
            } else {
                GAIT_IDLE
            };
            player.set_vec2_param(name!("gait"), gait);
        }
    }
}

#[cfg(feature = "debug")]
fn debug_keys(
    input: Res<Input>,
    data: Res<CaseData>,
    mut clock: ResMut<NightClock>,
    mut pause: ResMut<DebugPause>,
    mut session: ResMut<Session>,
    mut book: ResMut<Casebook>,
    mut sight: ResMut<Sight>,
) {
    use concerto::window::input::{KeyCode, PhysicalKey};
    let pressed = |key: KeyCode| input.is_just_pressed(PhysicalKey::Code(key));
    if pressed(KeyCode::F5) {
        let later = GameTime::from_seconds(clock.now().seconds() + 1800.0);
        clock.jump_to(later);
        session.notice(format!(
            "Debug: the clock jumps to {}",
            readout(clock.now())
        ));
    }
    if pressed(KeyCode::F6) {
        pause.0 = !pause.0;
        session.notice(if pause.0 {
            "Debug: the clock is held"
        } else {
            "Debug: the clock runs"
        });
    }
    if pressed(KeyCode::F7)
        && let Some(fact) = data.case.facts.ids().find(|fact| !book.knows(*fact))
    {
        session.grant(&mut book, &data, &[fact]);
    }
    if pressed(KeyCode::F8) {
        sight.night_over = true;
    }
    if pressed(KeyCode::F9)
        && let Some(deduction) = book
            .visible_deductions(&data.case)
            .into_iter()
            .find(|deduction| !book.is_confirmed(*deduction))
    {
        book.force_confirm(deduction);
        session.notice("Debug: a deduction is confirmed");
    }
}

#[cfg(test)]
mod tests {
    use concerto::{
        app::App,
        ecs::Entity,
        foundation::transform::Transform,
        gameplay::first_person::FirstPersonWalker,
        physics::collider::BoxCollider,
        window::input::{GamepadButton, GamepadId, Input, PlayerDevice, PlayerIndex, Players},
    };
    use glam::Vec3;

    use super::Sight;
    use crate::book::Casebook;
    use crate::clock::{GameTime, NightClock};
    use crate::players::SpawnPoint;
    use crate::session::{Mode, Session};

    const PADS: [GamepadId; 2] = [GamepadId::new(0), GamepadId::new(1)];

    fn frames(app: &mut App, count: u32) {
        for _ in 0..count {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
    }

    fn tap(app: &mut App, pad: usize, button: GamepadButton) {
        for value in [1.0, 0.0] {
            app.get_resource_mut::<Input>()
                .unwrap()
                .update_gamepad_button(PADS[pad], button, value);
            frames(app, 1);
        }
    }

    fn session(app: &App) -> &Session {
        app.get_resource::<Session>().unwrap()
    }

    fn walkers(app: &mut App) -> Vec<Entity> {
        let world = app.main_mut().world_mut();
        let mut walkers = world.query::<(Entity, &FirstPersonWalker), ()>();
        walkers.iter(world).map(|(entity, _)| entity).collect()
    }

    #[test]
    fn two_pads_play_a_night_talk_to_a_character_and_start_the_next() {
        let Some(mut app) = concerto::headless(crate::game_without_saves) else {
            return;
        };
        let world = app.main_mut().world_mut();
        world.spawn((
            BoxCollider::default(),
            Transform::from_translation_rotation_scale(
                Vec3::new(6.0, -0.5, -4.5),
                Default::default(),
                Vec3::new(30.0, 1.0, 30.0),
            ),
        ));
        world.spawn((
            SpawnPoint { player: 0 },
            Transform::from_translation(Vec3::new(3.0, 0.0, -0.8)),
        ));
        world.spawn((
            SpawnPoint { player: 1 },
            Transform::from_translation(Vec3::new(7.0, 0.0, -6.3)),
        ));
        for (index, pad) in PADS.into_iter().enumerate() {
            app.get_resource_mut::<Input>()
                .unwrap()
                .connect_gamepad(pad, "Test pad");
            app.get_resource_mut::<Players>()
                .unwrap()
                .assign(PlayerIndex::new(index as u8), PlayerDevice::Gamepad(pad))
                .unwrap();
        }
        frames(&mut app, 30);

        assert_eq!(
            *session(&app).mode(0),
            Mode::Card,
            "the first night opens on its card"
        );
        tap(&mut app, 0, GamepadButton::South);
        assert!(session(&app).is_walking(0) && session(&app).is_walking(1));
        let before = walkers(&mut app);
        assert_eq!(before.len(), 2);

        frames(&mut app, 20);
        let hobbs = app.get_resource::<crate::casefile::CaseData>().unwrap();
        let hobbs = hobbs.case.characters.id("npc_03").unwrap();
        assert_eq!(
            app.get_resource::<Sight>().unwrap().targets[1],
            Some(super::Interactable::Npc(hobbs)),
            "the Staff stands in front of Hobbs and looks at him"
        );
        tap(&mut app, 1, GamepadButton::West);
        assert_eq!(session(&app).talking_to(1), Some(hobbs));
        app.get_resource_mut::<NightClock>()
            .unwrap()
            .jump_to(GameTime::hm(20, 0));
        tap(&mut app, 1, GamepadButton::South);
        assert_eq!(
            app.get_resource::<Casebook>().unwrap().facts().len(),
            1,
            "his answer after 19:40 grants a fact"
        );
        tap(&mut app, 1, GamepadButton::East);
        tap(&mut app, 1, GamepadButton::East);
        assert!(session(&app).is_walking(1));

        app.get_resource_mut::<Sight>().unwrap().night_over = true;
        frames(&mut app, 5);
        let card = session(&app)
            .card
            .clone()
            .expect("the next night's card shows");
        assert_eq!(card.night, 2);
        assert_eq!(card.new_facts.len(), 1);
        assert_eq!(
            app.get_resource::<NightClock>().unwrap().now(),
            GameTime::hm(18, 0)
        );
        let after = walkers(&mut app);
        assert_eq!(after.len(), 2, "both players are seated again");
        assert!(after.iter().all(|walker| !before.contains(walker)));
    }
}
