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
        component::name::Name,
        entity::hierarchy::Children,
        events::event_reader::EventReader,
        reflect::{ReflectComponent, ReflectSceneComponent},
    },
    foundation::{
        assets::{
            AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle,
        },
        time::Time,
        transform::Transform,
    },
    gameplay::first_person::{FirstPersonView, FirstPersonWalker},
    mesh::{
        mesh::{Aabb, Mesh, MeshComponent},
        skeleton::JointName,
    },
    name::name,
    physics::{
        body::BodyId,
        collider::{BoxCollider, Collider, ColliderOffset},
        physics_state::PhysicsState,
        query::QueryFilter,
        rigid_body::{AllowedDofs, MotionType, RigidBody},
        shape::MeshCollider,
    },
    procedural_animation::{LookAtBone, LookAtNode},
    reflect::{Reflect, ReflectDefault},
    render::components::{camera::Camera, light::Light},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    ui::{
        material::UIMaterial,
        node::{FlexDirection, Position, UIInset, UINode, UIRect, UITargetCamera},
        text::UIText,
        transform::UIValue,
    },
    window::{
        input::{Input, PlayerIndex, Players, actions::ActionMap},
        plugin::{CursorMode, Window},
    },
};
use glam::{Quat, Vec2, Vec3};

use crate::book::Casebook;
use crate::book_ui::{
    BookPanel, PANEL, ScreenPanel, book_panel_node, chip, show_book, show_screens,
};
use crate::case::{DoorId, Id, NpcId, PlacementId, Role, RoomId, Zone};
use crate::casefile::CaseData;
use crate::clock::{GameTime, Midnight, NightClock, NightPaused};
use crate::controls::{
    Back, Confirm, Interact, Listen, MenuDown, MenuLeft, MenuRight, MenuUp, OpenBook, PageNext,
    PagePrev, Pause, Watch, bind_controls,
};
use crate::house::{House, Interactable, ItemPlace, end_night};
use crate::players::{PLAYER_COUNT, SeatEntity, Seated, SpawnPoint, body_layer};
use crate::rules::{Privacy, name_of};
use crate::save::{SaveFile, save_dir};
use crate::schedule::{NpcPose, WALK_SPEED, case_time, entries_of, line_at, pose_at, talks_at};
use crate::session::{Effect, Mode, Press, Session};
use concerto::gameplay::actions::{Look, Move};
use concerto::render::components::visibility::RenderLayers;

const SEATS: usize = PLAYER_COUNT as usize;
const REACH: f32 = 2.6;
const EYE_DROP: f32 = 1.5;
const AIM_RADIUS: f32 = 0.12;
const EARSHOT: f32 = 6.0;
const DOOR_LISTEN_REACH: f32 = 1.6;
const NPC_CATCH_UP: f32 = 3.2;
const NPC_SNAP: f32 = 5.0;
const NPC_DOOR_REACH: f32 = 1.1;
const NPC_DOOR_STOREY: f32 = 1.5;
const MOONLIGHT: f32 = 0.18;
const MOON_COLOR: Color = Color::rgba(0.62, 0.72, 1.0, 1.0);
const LAMP_GLOW: f32 = 5.0;
const SERVICE_LAMP_GLOW: f32 = 3.6;
const LAMP_COLOR: Color = Color::rgba(1.0, 0.74, 0.46, 1.0);
const SERVICE_LAMP_COLOR: Color = Color::rgba(1.0, 0.88, 0.7, 1.0);
const DEVICE_HALF_EXTENTS: Vec3 = Vec3::new(0.35, 0.5, 0.3);
const PASSING_HASTE: f64 = 12.0;
const HELD_SIZE: f32 = 0.15;
const HELD_NEAR: f32 = 0.32;
const HELD_FAR: f32 = 0.8;
const HELD_TURN_RATE: f32 = 2.4;
const HELD_ZOOM_RATE: f32 = 0.5;
const MODEL_TURN: f32 = std::f32::consts::PI;
const GAIT_IDLE: Vec2 = Vec2::ZERO;
const GAIT_WALK: Vec2 = Vec2::new(0.0, 1.0);
const GAIT_SIT: Vec2 = Vec2::new(1.0, 0.0);
const GAIT_BLEND_RATE: f32 = 2.5;
const LEVEL_ART: AssetId = asset_id!("content/art/export/level_test/scene.gasset");

fn fixture_art(kind: FixtureKind) -> AssetId {
    match kind {
        FixtureKind::Door => asset_id!("content/art/export/kit_door/scene.gasset"),
        FixtureKind::BellPull => asset_id!("content/art/export/kit_bell_pull/scene.gasset"),
        FixtureKind::BellBoard => asset_id!("content/art/export/kit_bell_board/scene.gasset"),
        FixtureKind::Gong => asset_id!("content/art/export/kit_gong/scene.gasset"),
        FixtureKind::Bed => asset_id!("content/art/export/kit_bed/scene.gasset"),
        FixtureKind::Lift => asset_id!("content/art/export/kit_lift/scene.gasset"),
    }
}

struct PropArt {
    scene: AssetId,
    size: Vec3,
}

fn prop_art(data: &CaseData, placement: PlacementId) -> PropArt {
    match data
        .catalog
        .props
        .name(data.case.placements.get(placement).prop)
    {
        "prop_case" => PropArt {
            scene: asset_id!("content/art/export/kit_prop_case/scene.gasset"),
            size: Vec3::new(0.5, 0.35, 0.3),
        },
        "prop_note" => PropArt {
            scene: asset_id!("content/art/export/kit_prop_note/scene.gasset"),
            size: Vec3::new(0.22, 0.02, 0.3),
        },
        "prop_basket" => PropArt {
            scene: asset_id!("content/art/export/kit_prop_basket/scene.gasset"),
            size: Vec3::new(0.6, 0.45, 0.45),
        },
        "prop_cufflink" => PropArt {
            scene: asset_id!("content/art/export/kit_prop_key/scene.gasset"),
            size: Vec3::new(0.14, 0.02, 0.06),
        },
        "prop_key" => PropArt {
            scene: asset_id!("content/art/export/kit_prop_key/scene.gasset"),
            size: Vec3::new(0.14, 0.02, 0.06),
        },
        _ => PropArt {
            scene: asset_id!("content/art/export/kit_prop_cup/scene.gasset"),
            size: Vec3::new(0.14, 0.07, 0.14),
        },
    }
}

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
struct LevelScene;

#[derive(Component)]
struct DoorPanel {
    door: crate::case::DoorId,
    closed: Transform,
    shown_open: bool,
}

#[derive(Component)]
struct LeafPanel {
    door: crate::case::DoorId,
    closed: Transform,
    shown_open: bool,
    bounds: Option<Aabb>,
}

fn ray_to_box(origin: Vec3, direction: Vec3, bounds: Aabb) -> Option<f32> {
    let mut near = 0.0f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let (from, along) = (origin[axis], direction[axis]);
        let (low, high) = (bounds.min[axis], bounds.max[axis]);
        if along.abs() < 1e-6 {
            if from < low || from > high {
                return None;
            }
            continue;
        }
        let (a, b) = ((low - from) / along, (high - from) / along);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far).then_some(near)
}

fn aim_at_leaf(eye: Vec3, aim: Vec3, leaf: &Transform, bounds: Aabb) -> Option<f32> {
    let into = leaf.rotation.inverse();
    let reach = Aabb {
        min: bounds.min * leaf.scale - Vec3::splat(AIM_RADIUS),
        max: bounds.max * leaf.scale + Vec3::splat(AIM_RADIUS),
    };
    ray_to_box(into * (eye - leaf.translation), into * aim, reach)
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
    gait: Vec2,
    graph: AssetHandle<AnimationGraph>,
}

struct CharacterArt {
    scene: AssetId,
    idle: AssetId,
    walk: AssetId,
    sit: AssetId,
}

fn character_art(data: &CaseData, npc: NpcId) -> CharacterArt {
    figure_of(
        data.catalog
            .portraits
            .name(data.case.characters.get(npc).portrait),
    )
}

fn figure_of(portrait: &str) -> CharacterArt {
    match portrait {
        "portrait_02" => CharacterArt {
            scene: asset_id!("content/characters/export/gentleman/scene.gasset"),
            idle: asset_id!("content/characters/export/gentleman/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/gentleman/animation_Walk_Loop.gasset"),
            sit: asset_id!(
                "content/characters/export/gentleman/animation_Sitting_Idle_Loop.gasset"
            ),
        },
        "portrait_03" => CharacterArt {
            scene: asset_id!("content/characters/export/footman/scene.gasset"),
            idle: asset_id!("content/characters/export/footman/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/footman/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/footman/animation_Sitting_Idle_Loop.gasset"),
        },
        "portrait_04" => CharacterArt {
            scene: asset_id!("content/characters/export/colonel/scene.gasset"),
            idle: asset_id!("content/characters/export/colonel/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/colonel/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/colonel/animation_Sitting_Idle_Loop.gasset"),
        },
        "portrait_05" => CharacterArt {
            scene: asset_id!("content/characters/export/debutante/scene.gasset"),
            idle: asset_id!("content/characters/export/debutante/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/debutante/animation_Walk_Loop.gasset"),
            sit: asset_id!(
                "content/characters/export/debutante/animation_Sitting_Idle_Loop.gasset"
            ),
        },
        "portrait_06" => CharacterArt {
            scene: asset_id!("content/characters/export/doctor/scene.gasset"),
            idle: asset_id!("content/characters/export/doctor/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/doctor/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/doctor/animation_Sitting_Idle_Loop.gasset"),
        },
        "portrait_07" => CharacterArt {
            scene: asset_id!("content/characters/export/butler/scene.gasset"),
            idle: asset_id!("content/characters/export/butler/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/butler/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/butler/animation_Sitting_Idle_Loop.gasset"),
        },
        "portrait_08" => CharacterArt {
            scene: asset_id!("content/characters/export/cook/scene.gasset"),
            idle: asset_id!("content/characters/export/cook/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/cook/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/cook/animation_Sitting_Idle_Loop.gasset"),
        },
        "portrait_09" => CharacterArt {
            scene: asset_id!("content/characters/export/housekeeper/scene.gasset"),
            idle: asset_id!("content/characters/export/housekeeper/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/housekeeper/animation_Walk_Loop.gasset"),
            sit: asset_id!(
                "content/characters/export/housekeeper/animation_Sitting_Idle_Loop.gasset"
            ),
        },
        "portrait_10" => CharacterArt {
            scene: asset_id!("content/characters/export/chauffeur/scene.gasset"),
            idle: asset_id!("content/characters/export/chauffeur/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/chauffeur/animation_Walk_Loop.gasset"),
            sit: asset_id!(
                "content/characters/export/chauffeur/animation_Sitting_Idle_Loop.gasset"
            ),
        },
        _ => CharacterArt {
            scene: asset_id!("content/characters/export/lady/scene.gasset"),
            idle: asset_id!("content/characters/export/lady/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/lady/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/lady/animation_Sitting_Idle_Loop.gasset"),
        },
    }
}

fn player_art(role: Role) -> CharacterArt {
    match role {
        Role::Guest => CharacterArt {
            scene: asset_id!("content/characters/export/guest/scene.gasset"),
            idle: asset_id!("content/characters/export/guest/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/guest/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/guest/animation_Sitting_Idle_Loop.gasset"),
        },
        Role::Staff => CharacterArt {
            scene: asset_id!("content/characters/export/maid/scene.gasset"),
            idle: asset_id!("content/characters/export/maid/animation_Idle_Loop.gasset"),
            walk: asset_id!("content/characters/export/maid/animation_Walk_Loop.gasset"),
            sit: asset_id!("content/characters/export/maid/animation_Sitting_Idle_Loop.gasset"),
        },
    }
}

fn character_graph(assets: &AssetServer, art: &CharacterArt) -> AnimationGraph {
    let look = LookAtNode {
        chain: vec![
            LookAtBone {
                bone: JointName(name!("neck_01")),
                share: 0.35,
                halflife: 0.25,
                ..Default::default()
            },
            LookAtBone {
                bone: JointName(name!("Head")),
                share: 1.0,
                halflife: 0.2,
                ..Default::default()
            },
        ],
        target_param: name!("look_target"),
        weight_param: Some(name!("look_weight")),
        ..Default::default()
    };
    let mut graph = AnimationGraph::new();
    graph.result_node().with_input(look, |mut body| {
        body.with_blend_space_2d_input(name!("gait"), |space| {
            space
                .animation_clip_input(assets.load::<AnimationClip>(art.idle), GAIT_IDLE)
                .animation_clip_input(assets.load::<AnimationClip>(art.walk), GAIT_WALK)
                .animation_clip_input(assets.load::<AnimationClip>(art.sit), GAIT_SIT);
        });
    });
    graph
}

#[derive(Component)]
struct PlayerVisual {
    walker: Entity,
    role: Role,
    animator: Option<Entity>,
    gait: Vec2,
    graph: AssetHandle<AnimationGraph>,
}

#[derive(Component)]
struct ItemBody {
    placement: PlacementId,
    shown: Option<ItemPlace>,
}

#[derive(Component)]
struct Held {
    player: u8,
    placement: PlacementId,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HudPart {
    Info,
    Guide,
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
    /// The Staff stands at a door with a conversation behind it and is not listening.
    pub could_listen: [bool; SEATS],
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
    install_stage(app, saves, Stage::test());
}

/// What is played: a case over its level, the scenes that draw the level, and whether the level's doors and devices are scene nodes found by name.
pub struct Stage {
    pub data: CaseData,
    pub scenes: Vec<AssetId>,
    pub named_nodes: bool,
}

impl Stage {
    /// The open test case on its two-room level, whose fixtures are placed in the level scene.
    pub fn test() -> Self {
        Self {
            data: CaseData::test(),
            scenes: vec![LEVEL_ART],
            named_nodes: false,
        }
    }

    /// The manor's greybox with an empty case, its doors and devices read from node names.
    pub fn manor_walk() -> Self {
        Self {
            data: CaseData::manor_walk(),
            scenes: vec![
                asset_id!("content/manor/export/manor_ground/scene.gasset"),
                asset_id!("content/manor/export/manor_first/scene.gasset"),
                asset_id!("content/manor/export/manor_attic/scene.gasset"),
                asset_id!("content/manor/export/manor_cellar/scene.gasset"),
                asset_id!("content/manor/export/manor_grounds/scene.gasset"),
            ],
            named_nodes: true,
        }
    }
}

#[derive(Resource)]
struct StageArt {
    scenes: Vec<AssetId>,
    named_nodes: bool,
}

/// Adds the night's mechanics over `stage`.
pub fn install_stage(app: &mut App, saves: Option<PathBuf>, stage: Stage) {
    app.insert_resource(StageArt {
        scenes: stage.scenes,
        named_nodes: stage.named_nodes,
    });
    let data = stage.data;
    let mut session = Session::default();
    let mut book = Casebook::default();
    if let Some(save) = saves.as_deref().and_then(SaveFile::read) {
        let restored = save.restore(&data);
        book = restored.book;
        session.roles = restored.roles;
    }
    session.show_card(book.loops() + 1, Vec::new());
    #[cfg(feature = "debug")]
    if std::env::var_os("ONE_MORE_NIGHT_NO_CARD").is_some() {
        session.card = None;
        for seat in &mut session.seats {
            seat.mode = Mode::Walking;
        }
    }
    #[cfg(feature = "debug")]
    if let Some(start) = std::env::var("ONE_MORE_NIGHT_START")
        .ok()
        .and_then(|start| {
            let (hour, minute) = start.split_once(':')?;
            Some(GameTime::hm(hour.parse().ok()?, minute.parse().ok()?))
        })
        && let Some(mut clock) = app.get_resource_mut::<NightClock>()
    {
        clock.jump_to(start);
    }
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
        .add_system(Update, adopt_named_nodes)
        .add_system(Update, swing_leaves.after(play))
        .add_system(Update, build_huds)
        .add_system(Update, find_targets)
        .add_system(Update, play.after(find_targets))
        .add_system(Update, overhear.after(play))
        .add_system(Update, turn_night.after(overhear))
        .add_system(Update, show_hud.after(turn_night))
        .add_system(Update, show_book.after(turn_night))
        .add_system(Update, show_screens.after(turn_night))
        .add_system(Update, swing_doors.after(play))
        .add_system(Update, place_items.after(play))
        .add_system(Update, face_npcs)
        .add_system(Update, dress_players)
        .add_system(Update, hold_doors_for_npcs.before(swing_doors))
        .add_system(Update, capture_mouse.after(play))
        .add_system(
            Update,
            hold_up_props
                .after(play)
                .after(concerto::gameplay::first_person::place_first_person_views),
        )
        .add_system(Update, pause_for_screens.after(play))
        .add_system(Update, pass_time.after(overhear))
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
            position: data.level.footing(staging.anchor),
            yaw: anchor.yaw,
            entry: 0,
            walking: false,
        });
    }
    let entries = entries_of(&data.case, npc, data.case.phase_at(case_time(now)));
    pose_at(&entries, &data.level, now, WALK_SPEED)
}

fn npc_room(data: &CaseData, house: &House, npc: NpcId, now: f64) -> Option<RoomId> {
    if house.gathered
        && let Some(staging) = data
            .case
            .finale
            .staging
            .iter()
            .find(|staging| staging.npc == npc)
    {
        return Some(data.level.anchor(staging.anchor).room);
    }
    let entries = entries_of(&data.case, npc, data.case.phase_at(case_time(now)));
    let pose = pose_at(&entries, &data.level, now, WALK_SPEED)?;
    (!pose.walking).then(|| data.level.anchor(entries[pose.entry].anchor).room)
}

fn privacy_at(
    data: &CaseData,
    house: &House,
    door: DoorId,
    player: u8,
    from: Option<RoomId>,
    now: f64,
) -> Privacy {
    let sides = data.level.door(door);
    let beyond: Vec<RoomId> = [sides.a, sides.b]
        .into_iter()
        .filter(|room| Some(*room) != from && data.level.room(*room).zone == Zone::Private)
        .collect();
    Privacy {
        entering: !beyond.is_empty(),
        own: data
            .level
            .home(player)
            .is_some_and(|home| beyond.contains(&home)),
        someone_in: data
            .case
            .characters
            .ids()
            .any(|npc| npc_room(data, house, npc, now).is_some_and(|room| beyond.contains(&room))),
        rung: beyond.iter().any(|room| house.bells[room.index()]),
    }
}

fn npc_is_busy(data: &CaseData, house: &House, npc: NpcId, now: f64) -> bool {
    let entries = entries_of(&data.case, npc, data.case.phase_at(case_time(now)));
    npc_pose(data, house, npc, now)
        .and_then(|pose| entries.get(pose.entry).map(|entry| entry.critical))
        .unwrap_or(false)
}

fn prop_size(data: &CaseData, placement: PlacementId) -> Vec3 {
    prop_art(data, placement).size
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
    stage: Res<StageArt>,
    mut cmd: CommandQueue,
) {
    for scene in &stage.scenes {
        cmd.spawn((
            SceneSpawnerComponent(assets.load::<Scene>(*scene)),
            Transform::default(),
            LevelScene,
        ));
    }
    if stage.named_nodes {
        cmd.spawn((
            Light::directional_light()
                .with_intensity(MOONLIGHT)
                .with_color(MOON_COLOR),
            Transform::from_translation_rotation(
                Vec3::ZERO,
                Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9),
            ),
        ));
        for lamp in data.level.lamps() {
            let service = data.level.room(lamp.room).zone == Zone::Service;
            let light = if service {
                Light::point_light()
                    .with_intensity(SERVICE_LAMP_GLOW)
                    .with_color(SERVICE_LAMP_COLOR)
            } else {
                Light::point_light()
                    .with_intensity(LAMP_GLOW)
                    .with_color(LAMP_COLOR)
            };
            cmd.spawn((
                Light {
                    intensity: light.intensity * lamp.glow,
                    ..light.with_range(lamp.range)
                },
                Transform::from_translation(lamp.position),
            ));
        }
    }
    for npc in data.case.characters.ids() {
        let Some(pose) = npc_pose(&data, &house, npc, now_seconds(&clock)) else {
            continue;
        };
        let art = character_art(&data, npc);
        let collider = Collider::capsule(0.5, 0.3);
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
                gait: GAIT_IDLE,
                graph: assets.add(character_graph(&assets, &art)),
            },
            SceneSpawnerComponent(assets.load::<Scene>(art.scene)),
            Transform::from_translation_rotation(
                pose.position,
                Quat::from_rotation_y(pose.yaw + MODEL_TURN),
            ),
        ));
    }
    for placement in data.case.placements.ids() {
        let art = prop_art(&data, placement);
        cmd.spawn((
            ItemBody {
                placement,
                shown: None,
            },
            Interactable::Placement(placement),
            Transform::from_translation(item_home(&data, placement)),
            BoxCollider {
                half_extents: art.size / 2.0,
            },
        ))
        .add_child((
            SceneSpawnerComponent(assets.load::<Scene>(art.scene)),
            Transform::default(),
        ));
    }
}

fn adopt_named_nodes(
    scenes: Query<(Entity, &Children), (With<LevelScene>, Without<Adopted>)>,
    children: Query<&Children>,
    nodes: Query<(&Name, &Transform, Option<&MeshComponent>)>,
    stage: Res<StageArt>,
    mut data: ResMut<CaseData>,
    mut cmd: CommandQueue,
) {
    if !stage.named_nodes {
        return;
    }
    let mut below = Vec::new();
    for (scene, roots) in scenes.iter() {
        cmd.insert(Adopted, scene);
        below.extend(roots.iter().copied());
    }
    while let Some(entity) = below.pop() {
        if let Some(parts) = children.get_entity(entity) {
            below.extend(parts.iter().copied());
        }
        let Some((name, transform, mesh)) = nodes.get_entity(entity) else {
            continue;
        };
        let name = name.as_str();
        let (kind, rest) = name.split_once('.').unwrap_or((name, ""));
        let features = &data.case.features;
        let hidden = data
            .level
            .hidden_nodes(|feature| features.is_on(feature))
            .contains(&name);
        let helper = matches!(kind, "room" | "nav" | "acoustic") || hidden;
        if helper {
            if mesh.is_some() {
                cmd.remove::<MeshComponent>(entity);
            }
            continue;
        }
        let solid = |cmd: &mut CommandQueue| {
            if mesh.is_some() {
                cmd.insert(MeshCollider, entity);
            } else {
                cmd.insert(
                    BoxCollider {
                        half_extents: DEVICE_HALF_EXTENTS,
                    },
                    entity,
                );
            }
        };
        let target = match kind {
            "anchor" => continue,
            "door" => {
                if let Some(door) = data.catalog.doors.id(rest) {
                    data.level.place_door(door, transform.translation);
                }
                continue;
            }
            "doorleaf" => {
                let door = data.catalog.doors.id(rest).or_else(|| {
                    rest.rsplit_once('.')
                        .and_then(|(door, _)| data.catalog.doors.id(door))
                });
                door.map(|door| {
                    cmd.insert(
                        LeafPanel {
                            door,
                            closed: transform.clone(),
                            shown_open: false,
                            bounds: None,
                        },
                        entity,
                    );
                    Interactable::Door(door)
                })
            }
            "bell" => data.catalog.rooms.id(rest).map(Interactable::BellPull),
            "bellboard" => Some(Interactable::BellBoard),
            "dumbwaiter" => match rest {
                "kitchen" => Some(Interactable::Lift(0)),
                "pantry" => Some(Interactable::Lift(1)),
                "gallery" => Some(Interactable::Lift(2)),
                _ => None,
            },
            "bed" => match rest {
                "guest" => Some(Interactable::Bed(0)),
                "staff" => Some(Interactable::Bed(1)),
                _ => None,
            },
            "spawn" => {
                let player = u32::from(rest == "staff");
                cmd.insert(SpawnPoint { player }, entity);
                if let Some(room) = data.level.room_at(transform.translation) {
                    data.level.place_home(player as u8, room);
                }
                continue;
            }
            _ => None,
        };
        if let Some(target) = target {
            cmd.insert(target, entity);
        }
        solid(&mut cmd);
    }
}

fn swing_leaves(
    leaves: Query<(Entity, &mut LeafPanel, &mut Transform, &MeshComponent)>,
    meshes: Res<AssetStore<Mesh>>,
    house: Res<House>,
    mut cmd: CommandQueue,
) {
    for (entity, mut leaf, mut transform, mesh) in leaves.iter() {
        if leaf.bounds.is_none()
            && let Some(bounds) = meshes.get(&mesh.handle).and_then(Mesh::local_aabb)
        {
            leaf.bounds = Some(bounds);
        }
        let open = house.stands_open(leaf.door);
        if leaf.shown_open == open {
            continue;
        }
        leaf.shown_open = open;
        let turn = if open {
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        transform.rotation = leaf.closed.rotation * turn;
        cmd.remove::<Collider>(entity);
        if open {
            cmd.remove::<MeshCollider>(entity);
        } else {
            cmd.insert(MeshCollider, entity);
        }
    }
}

fn adopt_fixtures(
    fixtures: Query<(Entity, &Fixture, &Transform), Without<Adopted>>,
    mut data: ResMut<CaseData>,
    assets: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    for (entity, fixture, transform) in fixtures.iter() {
        cmd.insert(Adopted, entity);
        cmd.entity(entity).add_child((
            SceneSpawnerComponent(assets.load::<Scene>(fixture_art(fixture.kind))),
            Transform::default(),
        ));
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
            let (node, fill) = chip();
            (node, fill, hud_text(size), HudText { player, part })
        };
        let root = cmd
            .spawn((
                UINode::default()
                    .with_size(UIValue::Percent(100.0), UIValue::Percent(100.0))
                    .with_flex_direction(FlexDirection::Column)
                    .with_gap(Vec2::splat(6.0))
                    .with_padding(UIRect::all(12.0)),
                UITargetCamera(camera),
                SeatEntity,
            ))
            .entity();
        cmd.entity(root)
            .add_child(UINode::default().with_size(UIValue::Percent(100.0), UIValue::Px(34.0)));
        cmd.entity(root).add_child(text(HudPart::Info, 16.0));
        cmd.entity(root).add_child(text(HudPart::Guide, 16.0));
        cmd.entity(root).add_child((
            book_panel_node(),
            UIMaterial {
                corner_radius: 8.0,
                ..UIMaterial::flat(PANEL)
            },
            BookPanel::new(player),
        ));
        cmd.entity(root).add_child((
            book_panel_node(),
            UIMaterial {
                corner_radius: 8.0,
                ..UIMaterial::flat(PANEL)
            },
            ScreenPanel::new(player),
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

struct OpenLeaf {
    door: crate::case::DoorId,
    distance: f32,
}

fn find_targets(
    cameras: Query<(&PlayerIndex, &Transform, &FirstPersonView), With<Camera>>,
    bodies: Query<&BodyId>,
    targets: Query<&Interactable>,
    leaves: Query<(&LeafPanel, &Transform)>,
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
        let aim = transform.rotation * Vec3::NEG_Z;
        let hit = physics.cast_sphere(transform.translation, AIM_RADIUS, aim * REACH, filter);
        let solid_at = hit.as_ref().map_or(REACH, |hit| hit.fraction * REACH);
        let open_leaf = leaves
            .iter()
            .filter(|(leaf, _)| leaf.shown_open)
            .filter_map(|(leaf, at)| {
                let distance = aim_at_leaf(transform.translation, aim, at, leaf.bounds?)?;
                (distance < solid_at).then_some(OpenLeaf {
                    door: leaf.door,
                    distance,
                })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance));
        seen[player.index() as usize] = match open_leaf {
            Some(leaf) => Some(Interactable::Door(leaf.door)),
            None => hit
                .and_then(|hit| hit.entity)
                .and_then(|entity| targets.get_entity(entity))
                .copied()
                .filter(|target| match target {
                    Interactable::Placement(placement) => house.in_the_open(*placement),
                    _ => true,
                }),
        };
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
    if session.card.is_none() {
        house.ring_scheduled(&mut session, &data, time);
    }
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
        if pad.is_just_pressed(Pause) {
            session.paused = !session.paused;
        }
        if session.paused {
            continue;
        }
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
                let at_the_door = match target {
                    Interactable::Door(door) => {
                        let from = cameras
                            .iter()
                            .find(|(camera, _)| camera.index() == player)
                            .and_then(|(_, eye)| {
                                data.level.room_at(eye.translation - Vec3::Y * EYE_DROP)
                            });
                        privacy_at(&data, &house, door, player, from, now)
                    }
                    _ => Privacy::default(),
                };
                effects.extend(house.use_target(
                    &mut session,
                    &mut book,
                    &data,
                    player,
                    target,
                    time,
                    busy,
                    at_the_door,
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
            (pad.is_just_pressed(PagePrev), Press::PagePrev),
            (pad.is_just_pressed(PageNext), Press::PageNext),
        ];
        for (pressed, press) in presses {
            if pressed {
                effects.extend(session.press(
                    player,
                    press,
                    &mut book,
                    &data,
                    time,
                    data.case.phase_at(time),
                ));
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
        let free = session.is_walking(player.index())
            && !session.paused
            && !session.seats[player.index() as usize].in_bed;
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
    let mut could_listen = [false; SEATS];
    let listeners: Vec<(u8, Vec3)> = walkers
        .iter()
        .map(|(player, transform)| (player.index(), transform.translation))
        .filter(|(player, _)| session.seats[*player as usize].present)
        .collect();
    if !house.gathered {
        let phase = data.case.phase_at(case_time(now_seconds(&clock)));
        for talk in talks_at(&data.case, phase, now_seconds(&clock)) {
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
                let listening = actions
                    .player(PlayerIndex::new(*player), &input, &players)
                    .is_held(Listen);
                let by_door = session.role(*player) == Role::Staff
                    && data.level.doors().any(|door| {
                        let rule = data.level.door(door);
                        data.level.door_position(door).is_some_and(|position| {
                            (Some(rule.a) == room || Some(rule.b) == room)
                                && position.distance(*at) <= DOOR_LISTEN_REACH
                                && position.distance(place) <= earshot * 1.5
                        })
                    });
                let at_door = by_door && listening;
                if by_door && !listening && !near {
                    could_listen[*player as usize] = true;
                }
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
    if sight.could_listen != could_listen {
        sight.could_listen = could_listen;
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

fn pass_time(
    session: Res<Session>,
    sight: Res<Sight>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    mut clock: ResMut<NightClock>,
) {
    let mut present = session.present_players().peekable();
    let waiting = present.peek().is_some()
        && !session.paused
        && sight.heard.iter().all(Option::is_none)
        && present.all(|player| {
            session.is_walking(player)
                && actions
                    .player(PlayerIndex::new(player), &input, &players)
                    .is_held(Watch)
        });
    let haste = if waiting { PASSING_HASTE } else { 1.0 };
    if clock.haste() != haste {
        clock.set_haste(haste);
    }
}

fn pause_for_screens(
    session: Res<Session>,
    debug: Res<DebugPause>,
    mut paused: ResMut<NightPaused>,
) {
    let hold = session.card.is_some() || session.finale.is_some() || session.paused || debug.0;
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
    for (hud, mut text, mut node) in texts.iter() {
        let seat = &session.seats[hud.player as usize];
        let wanted = match hud.part {
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
                        info.push_str(&format!("  Pocket watch: {}.", readout(clock.now())));
                        info.push_str(if clock.haste() > 1.0 {
                            "  Time flies."
                        } else {
                            "  Hold both watches to pass the time."
                        });
                    }
                    info
                }
            }
            HudPart::Guide => {
                if sight.could_listen[hud.player as usize] {
                    "Someone is talking behind this door. Hold RB to listen.".to_string()
                } else {
                    session.guide(hud.player, &book, &data).unwrap_or_default()
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
                if session.paused {
                    lines.push("Paused. Start, or Escape, to go on.");
                }
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
        if node.visible == wanted.is_empty() {
            node.visible = !wanted.is_empty();
        }
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

fn swing_doors(
    panels: Query<(Entity, &mut DoorPanel, &mut Transform, &BoxCollider)>,
    house: Res<House>,
    mut cmd: CommandQueue,
) {
    for (entity, mut panel, mut transform, collider) in panels.iter() {
        let open = house.stands_open(panel.door);
        if panel.shown_open == open {
            continue;
        }
        panel.shown_open = open;
        let closed = panel.closed.clone();
        let half = Vec3::X * collider.half_extents.x * closed.scale.x;
        let (rotation, translation) = if open {
            let hinge = closed.translation - closed.rotation * half;
            let rotation = closed.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
            (rotation, hinge + rotation * half)
        } else {
            (closed.rotation, closed.translation)
        };
        transform.rotation = rotation;
        transform.translation = translation;
        cmd.insert(*collider, entity);
    }
}

fn place_items(
    items: Query<(Entity, &mut ItemBody, &mut Transform, &BoxCollider)>,
    lifts: Query<(&Interactable, &Transform, &BoxCollider), Without<ItemBody>>,
    house: Res<House>,
    data: Res<CaseData>,
    mut cmd: CommandQueue,
) {
    for (entity, mut item, mut transform, collider) in items.iter() {
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
                .find(|(target, _, _)| **target == Interactable::Lift(shown.1))
                .map_or(Vec3::new(0.0, -40.0, 0.0), |(_, lift, cabinet)| {
                    lift.translation + Vec3::Y * (cabinet.half_extents.y + collider.half_extents.y)
                }),
        };
        let moved = transform.translation.distance(translation) > 1e-4;
        if item.shown != Some(place) {
            item.shown = Some(place);
        }
        if moved {
            transform.translation = translation;
            cmd.insert(*collider, entity);
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
        let step = pose.position - at;
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
            let entries = entries_of(&data.case, npc.npc, data.case.phase_at(case_time(now)));
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

fn hold_doors_for_npcs(
    npcs: Query<&Transform, With<NpcBody>>,
    data: Res<CaseData>,
    mut house: ResMut<House>,
) {
    for door in data.level.doors() {
        let held = data.level.door_position(door).is_some_and(|position| {
            npcs.iter().any(|npc| {
                npc.translation.distance(position.with_y(npc.translation.y)) <= NPC_DOOR_REACH
                    && (npc.translation.y - position.y).abs() < NPC_DOOR_STOREY
            })
        });
        if house.held_open[door.index()] != held {
            house.held_open[door.index()] = held;
        }
    }
}

fn capture_mouse(session: Res<Session>, players: Res<Players>, mut window: ResMut<Window>) {
    let played = (0..PLAYER_COUNT).any(|player| {
        players
            .devices(PlayerIndex::new(player))
            .is_some_and(|devices| devices.keyboard_mouse)
    });
    let captured = played && !session.paused;
    let cursor = CursorMode {
        visible: !captured,
        grabbed: captured,
    };
    if window.settings.cursor != cursor {
        window.settings.cursor = cursor;
    }
}

fn held_placement(session: &Session, player: u8) -> Option<PlacementId> {
    match session.mode(player) {
        Mode::Inspecting { placement, .. } | Mode::Reading { placement, .. } => Some(*placement),
        _ => None,
    }
}

fn hold_up_props(
    held: Query<(Entity, &mut Held, &mut Transform)>,
    eyes: Query<(&PlayerIndex, &Transform), (With<Camera>, Without<Held>)>,
    session: Res<Session>,
    data: Res<CaseData>,
    assets: Res<AssetServer>,
    input: Res<Input>,
    actions: Res<ActionMap>,
    players: Res<Players>,
    time: Res<Time>,
    mut cmd: CommandQueue,
) {
    let dt = time.real_delta().as_secs_f32();
    let mut shown = [false; SEATS];
    for (entity, mut prop, mut transform) in held.iter() {
        let eye = eyes
            .iter()
            .find(|(player, _)| player.index() == prop.player)
            .map(|(_, eye)| eye);
        let (Some(eye), true) = (
            eye,
            held_placement(&session, prop.player) == Some(prop.placement),
        ) else {
            cmd.despawn(entity);
            continue;
        };
        shown[prop.player as usize] = true;
        let pad = actions.player(PlayerIndex::new(prop.player), &input, &players);
        let turn = pad.axis2d(Look) * HELD_TURN_RATE * dt;
        let zoom = pad.axis2d(Move).y * HELD_ZOOM_RATE * dt;
        if turn != Vec2::ZERO || zoom != 0.0 {
            prop.yaw += turn.x;
            prop.pitch = (prop.pitch - turn.y).clamp(-1.4, 1.4);
            prop.distance = (prop.distance - zoom).clamp(HELD_NEAR, HELD_FAR);
        }
        transform.translation =
            eye.translation + eye.rotation * Vec3::new(0.0, -0.06, -prop.distance);
        transform.rotation =
            eye.rotation * Quat::from_rotation_x(prop.pitch) * Quat::from_rotation_y(prop.yaw);
    }
    for (player, eye) in eyes.iter() {
        let player = player.index();
        let Some(placement) = held_placement(&session, player).filter(|_| !shown[player as usize])
        else {
            continue;
        };
        let art = prop_art(&data, placement);
        let scale = HELD_SIZE / art.size.max_element();
        cmd.spawn((
            Held {
                player,
                placement,
                yaw: 0.5,
                pitch: 0.6,
                distance: 0.55,
            },
            Transform::from_translation(eye.translation),
        ))
        .add_child((
            SceneSpawnerComponent(assets.load::<Scene>(art.scene)),
            Transform::from_translation_rotation_scale(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(scale),
            ),
        ));
    }
}

fn dress_players(
    walkers: Query<(Entity, &PlayerIndex, &FirstPersonWalker, &Transform), Without<PlayerVisual>>,
    visuals: Query<(Entity, &mut PlayerVisual, &mut Transform)>,
    children: Query<&Children>,
    players: Query<&mut AnimationPlayer>,
    undrawn: Query<Entity, (With<MeshComponent>, Without<RenderLayers>)>,
    session: Res<Session>,
    assets: Res<AssetServer>,
    time: Res<Time>,
    mut cmd: CommandQueue,
) {
    let dt = time.delta().as_secs_f32();
    let mut dressed = Vec::new();
    for (entity, mut visual, mut transform) in visuals.iter() {
        let Some((_, player, walker, body)) = walkers.get_entity(visual.walker) else {
            cmd.despawn(entity);
            continue;
        };
        if session.role(player.index()) != visual.role {
            cmd.despawn(entity);
            continue;
        }
        dressed.push(visual.walker);
        let mut below = vec![entity];
        while let Some(part) = below.pop() {
            if undrawn.contains_entity(part) {
                cmd.insert(body_layer(*player), part);
            }
            if let Some(parts) = children.get_entity(part) {
                below.extend(parts.iter().copied());
            }
        }
        transform.translation = body.translation;
        transform.rotation = Quat::from_rotation_y(walker.yaw() + MODEL_TURN);
        if visual.animator.is_none()
            && let Some(animator) = animator_under(entity, &children, &players)
        {
            visual.animator = Some(animator);
            cmd.insert(
                AnimationHandleComponent::new(visual.graph.clone()),
                animator,
            );
        }
        if let Some(mut player) = visual
            .animator
            .and_then(|player| players.get_entity(player))
        {
            let gait = if walker.desired_velocity().length() > 0.1 {
                GAIT_WALK
            } else {
                GAIT_IDLE
            };
            let step = (gait - visual.gait).clamp_length_max(GAIT_BLEND_RATE * dt);
            if step != Vec2::ZERO {
                visual.gait += step;
            }
            player.set_vec2_param(name!("gait"), visual.gait);
            player.set_float_param(name!("look_weight"), 0.0);
        }
    }
    for (walker, player, _, body) in walkers.iter() {
        if dressed.contains(&walker) {
            continue;
        }
        let role = session.role(player.index());
        let art = player_art(role);
        cmd.spawn((
            PlayerVisual {
                walker,
                role,
                animator: None,
                gait: GAIT_IDLE,
                graph: assets.add(character_graph(&assets, &art)),
            },
            SceneSpawnerComponent(assets.load::<Scene>(art.scene)),
            Transform::from_translation(body.translation),
            SeatEntity,
        ));
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
    eyes: Query<(&PlayerIndex, &Transform), (With<Camera>, Without<NpcVisual>)>,
    session: Res<Session>,
    house: Res<House>,
    time: Res<Time>,
    mut cmd: CommandQueue,
) {
    let dt = time.delta().as_secs_f32();
    for (entity, mut visual, mut transform) in visuals.iter() {
        let Some((npc, body)) = bodies.get_entity(visual.body) else {
            continue;
        };
        let watched = eyes
            .iter()
            .filter(|(player, _)| {
                house.gathered || session.talking_to(player.index()) == Some(npc.npc)
            })
            .map(|(_, eye)| eye.translation)
            .min_by(|a, b| {
                a.distance(body.translation)
                    .total_cmp(&b.distance(body.translation))
            });
        transform.translation = body.translation;
        transform.rotation = Quat::from_rotation_y(npc.yaw + MODEL_TURN);
        if visual.animator.is_none()
            && let Some(animator) = animator_under(entity, &children, &players)
        {
            visual.animator = Some(animator);
            cmd.insert(
                AnimationHandleComponent::new(visual.graph.clone()),
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
            let step = (gait - visual.gait).clamp_length_max(GAIT_BLEND_RATE * dt);
            if step != Vec2::ZERO {
                visual.gait += step;
            }
            player.set_vec2_param(name!("gait"), visual.gait);
            if let Some(target) = watched {
                player.set_param(name!("look_target"), target);
            }
            player.set_float_param(name!("look_weight"), f32::from(watched.is_some()));
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
    let first = session.present_players().next();
    if pressed(KeyCode::F10)
        && let Some(player) = first
        && let Some(placement) = data.case.placements.ids().find(|placement| {
            matches!(
                data.case.placements.get(*placement).interaction,
                crate::case::Interaction::Inspect { .. }
            )
        })
    {
        session.examine(player, placement, &mut book, &data);
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
    use concerto::mesh::mesh::Aabb;
    use concerto::{
        app::App,
        ecs::Entity,
        foundation::{
            assets::{ContentAssetRoot, asset_server::AssetServer, registry::AssetRegistry},
            transform::Transform,
        },
        gameplay::first_person::FirstPersonWalker,
        physics::collider::BoxCollider,
        window::input::{
            GamepadAxis, GamepadButton, GamepadId, Input, PlayerDevice, PlayerIndex, Players,
        },
    };
    use glam::{Quat, Vec3};

    use super::Sight;
    use crate::book::Casebook;
    use crate::case::Id;
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
    fn the_staff_opens_a_service_door_and_its_body_swings_with_it() {
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
        let door = world.spawn((
            super::Fixture {
                kind: super::FixtureKind::Door,
                name: "d_a_service".into(),
            },
            BoxCollider {
                half_extents: Vec3::new(0.5, 1.1, 0.04),
            },
            Transform::from_translation(Vec3::new(3.0, 1.1, -6.0)),
        ));
        world.spawn((
            SpawnPoint { player: 1 },
            Transform::from_translation_rotation(
                Vec3::new(3.0, 0.0, -7.6),
                glam::Quat::from_rotation_y(std::f32::consts::PI),
            ),
        ));
        app.get_resource_mut::<Input>()
            .unwrap()
            .connect_gamepad(PADS[1], "Test pad");
        app.get_resource_mut::<Players>()
            .unwrap()
            .assign(PlayerIndex::new(1), PlayerDevice::Gamepad(PADS[1]))
            .unwrap();
        frames(&mut app, 30);
        tap(&mut app, 1, GamepadButton::South);
        frames(&mut app, 20);
        let through = |app: &App| {
            app.get_resource::<concerto::physics::physics_state::PhysicsState>()
                .unwrap()
                .cast_ray(
                    Vec3::new(3.0, 1.9, -6.6),
                    Vec3::new(0.0, 0.0, 1.5),
                    Default::default(),
                )
                .and_then(|hit| hit.entity)
        };
        assert_eq!(
            through(&app),
            Some(door),
            "the closed door fills its doorway"
        );

        tap(&mut app, 1, GamepadButton::West);
        frames(&mut app, 20);

        let panel = app
            .main_mut()
            .world_mut()
            .get_component_for_entity::<Transform>(door)
            .unwrap()
            .clone();
        assert!(
            (panel.translation - Vec3::new(2.5, 1.1, -6.5)).length() < 0.01,
            "the panel swung about its hinge, to {}",
            panel.translation
        );
        assert_eq!(through(&app), None, "and the doorway is clear");
    }

    #[test]
    fn a_private_conversation_covers_itself_for_the_guest_and_is_heard_by_the_staff() {
        for (staff, heard, facts) in [
            (
                false,
                "Mr Corwin: Lovely weather for the season, is it not?",
                0,
            ),
            (
                true,
                "Lady Ashby: Then you will answer for it, Mr Corwin.",
                1,
            ),
        ] {
            let Some(mut app) = concerto::headless(crate::game_without_saves) else {
                return;
            };
            if staff {
                app.get_resource_mut::<Session>().unwrap().roles.swap(0, 1);
            }
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
                Transform::from_translation(Vec3::new(9.0, 0.0, -4.4)),
            ));
            app.get_resource_mut::<Input>()
                .unwrap()
                .connect_gamepad(PADS[0], "Test pad");
            app.get_resource_mut::<Players>()
                .unwrap()
                .assign(PlayerIndex::new(0), PlayerDevice::Gamepad(PADS[0]))
                .unwrap();
            frames(&mut app, 30);
            tap(&mut app, 0, GamepadButton::South);
            app.get_resource_mut::<NightClock>()
                .unwrap()
                .jump_to(GameTime::from_seconds(
                    GameTime::hm(19, 0).seconds() + 210.0,
                ));
            frames(&mut app, 40);

            assert_eq!(
                app.get_resource::<Sight>().unwrap().heard[0].as_deref(),
                Some(heard),
                "staff: {staff}"
            );
            assert_eq!(
                app.get_resource::<Casebook>().unwrap().facts().len(),
                facts,
                "staff: {staff}"
            );
        }
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

        let case = app.get_resource::<crate::casefile::CaseData>().unwrap();
        let placement = case.case.placements.id("p_case").unwrap();
        let held_props = |app: &mut App| {
            let world = app.main_mut().world_mut();
            let mut held = world.query::<&super::Held, ()>();
            held.iter(world).count()
        };
        app.get_resource_mut::<Session>().unwrap().seats[0].mode =
            Mode::Inspecting { placement, spot: 0 };
        frames(&mut app, 3);
        assert_eq!(
            held_props(&mut app),
            1,
            "an inspected prop is held up to its player"
        );
        app.get_resource_mut::<Session>().unwrap().seats[0].mode = Mode::Walking;
        frames(&mut app, 3);
        assert_eq!(held_props(&mut app), 0, "and put away when they are done");

        for pad in PADS {
            app.get_resource_mut::<Input>()
                .unwrap()
                .update_gamepad_button(pad, GamepadButton::LeftBumper, 1.0);
        }
        frames(&mut app, 3);
        assert_eq!(
            app.get_resource::<NightClock>().unwrap().haste(),
            12.0,
            "both watches held pass the time"
        );
        app.get_resource_mut::<Input>()
            .unwrap()
            .update_gamepad_button(PADS[0], GamepadButton::LeftBumper, 0.0);
        frames(&mut app, 3);
        assert_eq!(app.get_resource::<NightClock>().unwrap().haste(), 1.0);
        app.get_resource_mut::<Input>()
            .unwrap()
            .update_gamepad_button(PADS[1], GamepadButton::LeftBumper, 0.0);
        frames(&mut app, 1);

        tap(&mut app, 1, GamepadButton::Start);
        let held = app.get_resource::<NightClock>().unwrap().now();
        frames(&mut app, 10);
        assert!(session(&app).paused);
        assert_eq!(
            app.get_resource::<NightClock>().unwrap().now(),
            held,
            "the shared pause holds the clock"
        );
        tap(&mut app, 0, GamepadButton::West);
        assert!(session(&app).is_walking(0), "and nothing else is read");
        tap(&mut app, 0, GamepadButton::Start);
        assert!(!session(&app).paused, "either player ends it");

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

    #[test]
    fn an_aim_reaches_a_leaf_that_stands_open_and_misses_one_behind_it() {
        let bounds = Aabb {
            min: Vec3::new(0.0, 0.0, -0.02),
            max: Vec3::new(0.9, 2.1, 0.02),
        };
        let leaf = Transform::from_translation_rotation(
            Vec3::new(4.0, 0.0, -2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        let eye = Vec3::new(3.0, 1.6, -2.5);

        let reached = super::aim_at_leaf(eye, Vec3::X, &leaf, bounds).unwrap();
        assert!((reached - (1.0 - 0.02 - super::AIM_RADIUS)).abs() < 1e-4);
        assert_eq!(super::aim_at_leaf(eye, Vec3::NEG_X, &leaf, bounds), None);
        assert_eq!(super::aim_at_leaf(eye, Vec3::Z, &leaf, bounds), None);
        assert_eq!(
            super::aim_at_leaf(eye.with_z(-4.0), Vec3::X, &leaf, bounds),
            None,
            "past the leaf's free edge"
        );
    }

    const EYE_HEIGHT: f32 = 1.62;

    struct Leg {
        x: f32,
        y: f32,
    }

    struct Walk {
        player: u8,
        legs: Vec<Leg>,
        ends_at_height: f32,
        ends_in: &'static str,
    }

    fn leg(x: f32, y: f32) -> Leg {
        Leg { x, y }
    }

    fn feet(app: &mut App, player: u8) -> Option<Vec3> {
        let world = app.main_mut().world_mut();
        let mut walkers = world.query::<(&PlayerIndex, &FirstPersonWalker, &Transform), ()>();
        walkers
            .iter(world)
            .find(|(index, ..)| index.index() == player)
            .map(|(_, _, transform)| transform.translation)
    }

    fn head_for(app: &mut App, player: u8, target: Vec3) {
        let world = app.main_mut().world_mut();
        let mut walkers = world.query::<(&PlayerIndex, &mut FirstPersonWalker, &Transform), ()>();
        for (index, mut walker, transform) in walkers.iter(world) {
            if index.index() != player {
                continue;
            }
            let to = target - transform.translation;
            let yaw = (-to.x).atan2(-to.z);
            let turn = yaw - walker.yaw();
            walker.look(turn, 0.0);
        }
    }

    fn aiming_at(app: &mut App, player: u8, point: Vec3) -> Option<super::Interactable> {
        let world = app.main_mut().world_mut();
        let mut walkers = world.query::<(&PlayerIndex, &mut FirstPersonWalker, &Transform), ()>();
        for (index, mut walker, transform) in walkers.iter(world) {
            if index.index() != player {
                continue;
            }
            let to = point - (transform.translation + Vec3::Y * EYE_HEIGHT);
            let yaw = (-to.x).atan2(-to.z);
            let pitch = to.y.atan2(to.x.hypot(to.z));
            let turn = yaw - walker.yaw();
            let tilt = pitch - walker.pitch();
            walker.look(turn, tilt);
        }
        frames(app, 3);
        app.get_resource::<Sight>().unwrap().targets[player as usize]
    }

    fn uses(app: &mut App, player: u8) {
        tap(app, player as usize, GamepadButton::West);
        frames(app, 5);
    }

    fn walk_all(app: &mut App, walks: &[Walk]) -> [usize; 2] {
        let mut reached = [0usize; 2];
        for _ in 0..12_000 {
            let mut walking = false;
            for walk in walks {
                let player = walk.player as usize;
                let at = feet(app, walk.player).expect("the player has a walker");
                while reached[player] < walk.legs.len() {
                    let leg = &walk.legs[reached[player]];
                    let target = Vec3::new(leg.x, at.y, -leg.y);
                    if at.distance(target) > 0.25 {
                        head_for(app, walk.player, target);
                        break;
                    }
                    reached[player] += 1;
                }
                let push = if reached[player] < walk.legs.len() {
                    walking = true;
                    1.0
                } else {
                    0.0
                };
                app.get_resource_mut::<Input>()
                    .unwrap()
                    .update_gamepad_axis(PADS[player], GamepadAxis::LeftStickY, push);
            }
            if !walking {
                break;
            }
            frames(app, 1);
        }
        reached
    }

    fn walk_to_the_end(app: &mut App, walks: &[Walk]) {
        let reached = walk_all(app, walks);
        frames(app, 30);
        for walk in walks {
            let at = feet(app, walk.player).unwrap();
            assert_eq!(
                reached[walk.player as usize],
                walk.legs.len(),
                "player {} stopped at {at} on the way",
                walk.player
            );
            assert!(
                (at.y - walk.ends_at_height).abs() < 0.3,
                "player {} ends at height {}",
                walk.player,
                at.y
            );
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            assert_eq!(
                data.level.room_at(at),
                data.catalog.rooms.id(walk.ends_in),
                "player {} ends in the {}",
                walk.player,
                walk.ends_in
            );
        }
    }

    #[test]
    fn every_portrait_a_case_can_name_has_its_own_figure_and_picture() {
        let figures: Vec<_> = crate::casefile::PORTRAITS
            .iter()
            .map(|portrait| super::figure_of(portrait).scene)
            .collect();
        assert_eq!(figures.len(), 10);
        for (index, figure) in figures.iter().enumerate() {
            assert!(
                !figures[..index].contains(figure),
                "{} wears a figure of its own",
                crate::casefile::PORTRAITS[index]
            );
        }
        assert_eq!(crate::book_ui::PORTRAITS.len(), figures.len());
    }

    #[test]
    fn the_manor_is_walked_and_its_doors_bells_beds_and_dumbwaiter_answer() {
        let Some(mut app) = concerto::headless(|app| {
            crate::manor(app);
            let content = concerto::content_registry!();
            app.get_resource::<AssetServer>()
                .unwrap()
                .publish_content_source(
                    ContentAssetRoot::Directory(env!("CARGO_MANIFEST_DIR").into()),
                    AssetRegistry::parse(content.registry).unwrap(),
                );
        }) else {
            return;
        };
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
        tap(&mut app, 0, GamepadButton::South);
        assert!(session(&app).is_walking(0) && session(&app).is_walking(1));
        for _ in 0..4000 {
            frames(&mut app, 5);
            if walkers(&mut app).len() == 2 {
                break;
            }
        }
        frames(&mut app, 60);
        let bedroom = {
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            data.catalog.doors.id("d_g5").unwrap()
        };
        let aimed_at = |app: &mut App, x: f32, y: f32| {
            let at = feet(app, 0).unwrap();
            aiming_at(app, 0, Vec3::new(x, at.y + EYE_HEIGHT, -y))
        };
        let house = |app: &App| app.get_resource::<crate::house::House>().unwrap().clone();
        let room = |app: &App, name: &str| {
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            data.catalog.rooms.id(name).unwrap()
        };
        let in_bed = |app: &App, player: usize| session(app).seats[player].in_bed;

        {
            let world = app.main_mut().world_mut();
            let mut bodies = world.query::<(&super::NpcBody, &Transform), ()>();
            let heights: Vec<f32> = bodies
                .iter(world)
                .map(|(_, transform)| transform.translation.y)
                .collect();
            assert_eq!(heights.len(), 10, "the house holds its ten characters");
            assert_eq!(
                heights
                    .iter()
                    .filter(|height| (**height - 3.8).abs() < 0.2)
                    .count(),
                1,
                "the one who is upstairs stands on that floor"
            );
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            let her_door = data.catalog.doors.id("d_g1").unwrap();
            let knock = |player: u8, hour: f64| {
                super::privacy_at(data, &house(&app), her_door, player, None, hour * 3600.0)
            };
            let lady = data.case.characters.id("npc_01").unwrap();
            let on_the_stair = (0..90).any(|step| {
                let now = 19.5 * 3600.0 + f64::from(step) * 20.0;
                super::npc_pose(data, &house(&app), lady, now)
                    .is_some_and(|pose| pose.walking && (0.5..3.3).contains(&pose.position.y))
            });
            assert!(on_the_stair, "she comes down the main stair to dinner");
            assert!(knock(0, 18.2).someone_in, "she is in until seven");
            assert!(!knock(1, 20.0).someone_in, "and out in the gallery after");
        }
        assert_eq!(
            aiming_at(&mut app, 0, Vec3::new(20.1, 4.35, -13.5)),
            Some(super::Interactable::Bed(0)),
            "the Guest starts beside their bed"
        );
        uses(&mut app, 0);
        assert!(in_bed(&app, 0));
        uses(&mut app, 0);
        assert!(!in_bed(&app, 0));
        walk_all(
            &mut app,
            &[Walk {
                player: 0,
                legs: vec![leg(23.2, 13.8)],
                ends_at_height: 3.8,
                ends_in: "g5",
            }],
        );
        let bedroom_bell = room(&app, "g5");
        assert_eq!(
            aiming_at(&mut app, 0, Vec3::new(24.35, 5.4, -14.1)),
            Some(super::Interactable::BellPull(bedroom_bell))
        );
        uses(&mut app, 0);
        assert!(house(&app).bells[bedroom_bell.index()]);
        assert_eq!(
            aiming_at(&mut app, 1, Vec3::new(37.14, 2.0, -6.2)),
            Some(super::Interactable::BellBoard),
            "the Staff starts by the bell board"
        );
        uses(&mut app, 1);
        assert!(
            !house(&app).bells[bedroom_bell.index()],
            "the Staff reading the board drops its flags"
        );
        let stands_open = |app: &App| {
            app.get_resource::<crate::house::House>()
                .unwrap()
                .stands_open(bedroom)
        };
        let to_the_door = [Walk {
            player: 0,
            legs: vec![leg(21.75, 11.2)],
            ends_at_height: 3.8,
            ends_in: "g5",
        }];
        walk_all(&mut app, &to_the_door);
        assert_eq!(
            aimed_at(&mut app, 21.75, 10.0),
            Some(super::Interactable::Door(bedroom)),
            "the Guest faces their closed bedroom door"
        );
        tap(&mut app, 0, GamepadButton::West);
        frames(&mut app, 5);
        assert!(stands_open(&app));
        assert_eq!(
            aimed_at(&mut app, 21.26, 10.6),
            Some(super::Interactable::Door(bedroom)),
            "the leaf standing open beside them is still theirs to close"
        );
        tap(&mut app, 0, GamepadButton::West);
        frames(&mut app, 5);
        assert!(!stands_open(&app));
        assert_eq!(
            aimed_at(&mut app, 21.75, 10.0),
            Some(super::Interactable::Door(bedroom))
        );
        tap(&mut app, 0, GamepadButton::West);
        frames(&mut app, 5);
        assert!(stands_open(&app));

        let walks = [
            Walk {
                player: 0,
                legs: vec![leg(21.75, 9.0), leg(26.0, 9.1)],
                ends_at_height: 3.8,
                ends_in: "gallery",
            },
            Walk {
                player: 1,
                legs: vec![
                    leg(37.9, 4.0),
                    leg(36.0, 4.0),
                    leg(35.5, 5.9),
                    leg(31.2, 6.6),
                ],
                ends_at_height: 0.0,
                ends_in: "kitchen",
            },
        ];
        walk_to_the_end(&mut app, &walks);
        let neighbour = {
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            assert_eq!(
                data.level.home(0),
                data.catalog.rooms.id("g5"),
                "the room the Guest starts in is their own"
            );
            data.catalog.doors.id("d_g2").unwrap()
        };
        assert_eq!(
            aimed_at(&mut app, 26.0, 8.0),
            Some(super::Interactable::Door(neighbour))
        );
        uses(&mut app, 0);
        assert!(
            !house(&app).stands_open(neighbour),
            "an empty bedroom that is not the Guest's stays shut to them"
        );
        walk_all(
            &mut app,
            &[Walk {
                player: 0,
                legs: vec![leg(28.6, 9.0)],
                ends_at_height: 3.8,
                ends_in: "gallery",
            }],
        );
        assert_eq!(
            aiming_at(&mut app, 0, Vec3::new(29.55, 4.7, -8.1)),
            Some(super::Interactable::Lift(2))
        );
        uses(&mut app, 0);
        assert_eq!(house(&app).lift_at, 2, "the gallery stop calls the car up");
        assert_eq!(
            aiming_at(&mut app, 1, Vec3::new(30.1, 0.9, -7.55)),
            Some(super::Interactable::Lift(0))
        );
        uses(&mut app, 1);
        assert_eq!(house(&app).lift_at, 0, "and the kitchen stop calls it back");

        let walks = [
            Walk {
                player: 0,
                legs: vec![
                    leg(18.2, 9.0),
                    leg(18.2, 11.8),
                    leg(18.2, 15.0),
                    leg(16.2, 15.0),
                    leg(16.2, 10.8),
                    leg(13.6, 6.5),
                ],
                ends_at_height: 0.0,
                ends_in: "hall",
            },
            Walk {
                player: 1,
                legs: vec![
                    leg(33.5, 7.0),
                    leg(33.5, 9.0),
                    leg(31.5, 9.5),
                    leg(31.5, 10.6),
                    leg(30.7, 10.6),
                    leg(30.7, 15.3),
                    leg(32.3, 15.3),
                    leg(32.3, 10.6),
                    leg(30.7, 10.6),
                    leg(30.7, 15.3),
                    leg(32.3, 15.3),
                    leg(32.3, 10.6),
                    leg(32.2, 9.0),
                ],
                ends_at_height: 7.2,
                ends_in: "backstairs",
            },
        ];
        walk_to_the_end(&mut app, &walks);

        let staff_room = {
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            data.catalog.doors.id("d_staffroom").unwrap()
        };
        walk_all(
            &mut app,
            &[Walk {
                player: 1,
                legs: vec![leg(21.75, 9.0)],
                ends_at_height: 7.2,
                ends_in: "attic_corr",
            }],
        );
        assert_eq!(
            aiming_at(&mut app, 1, Vec3::new(21.75, 8.3, -10.0)),
            Some(super::Interactable::Door(staff_room))
        );
        uses(&mut app, 1);
        walk_to_the_end(
            &mut app,
            &[Walk {
                player: 1,
                legs: vec![leg(21.75, 11.0), leg(22.0, 12.0)],
                ends_at_height: 7.2,
                ends_in: "attic_staff",
            }],
        );
        assert_eq!(
            aiming_at(&mut app, 1, Vec3::new(23.45, 7.7, -12.85)),
            Some(super::Interactable::Bed(1)),
            "the Staff's bed is under the attic roof"
        );
        uses(&mut app, 1);
        assert!(in_bed(&app, 1));
        uses(&mut app, 1);
        assert!(!in_bed(&app, 1));
        walk_all(
            &mut app,
            &[Walk {
                player: 1,
                legs: vec![leg(21.75, 11.0), leg(21.75, 9.0), leg(32.2, 9.0)],
                ends_at_height: 7.2,
                ends_in: "backstairs",
            }],
        );

        let garden_door = {
            let data = app.get_resource::<crate::casefile::CaseData>().unwrap();
            data.catalog.doors.id("d_hall_garden").unwrap()
        };
        walk_all(
            &mut app,
            &[Walk {
                player: 0,
                legs: vec![leg(13.0, 14.8)],
                ends_at_height: 0.0,
                ends_in: "hall",
            }],
        );
        assert_eq!(
            aimed_at(&mut app, 13.0, 16.0),
            Some(super::Interactable::Door(garden_door))
        );
        tap(&mut app, 0, GamepadButton::West);
        frames(&mut app, 5);
        let ways_out = [
            Walk {
                player: 0,
                legs: vec![leg(13.0, 18.0), leg(13.0, 23.0)],
                ends_at_height: -0.6,
                ends_in: "garden",
            },
            Walk {
                player: 1,
                legs: vec![
                    leg(32.3, 10.6),
                    leg(32.3, 15.3),
                    leg(30.7, 15.3),
                    leg(30.7, 10.6),
                    leg(32.3, 10.6),
                    leg(32.3, 15.3),
                    leg(30.7, 15.3),
                    leg(30.7, 10.6),
                    leg(32.3, 10.6),
                    leg(32.3, 15.3),
                    leg(30.7, 15.3),
                    leg(30.7, 10.6),
                    leg(31.5, 9.2),
                ],
                ends_at_height: -3.0,
                ends_in: "cellar_pass",
            },
        ];
        walk_to_the_end(&mut app, &ways_out);
    }
}
