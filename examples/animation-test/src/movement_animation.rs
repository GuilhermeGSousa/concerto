use concerto::{
    animation::{
        clip::AnimationClip,
        graph::AnimationGraph,
        player::{AnimationHandleComponent, AnimationPlayer},
    },
    ecs::{
        command::CommandQueue,
        component::Component,
        entity::Entity,
        query::{filter::Without, Query},
        resource::Res,
    },
    foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        time::Time,
        transform::Transform,
    },
    render::components::{light::LightType, Light},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    window::input::Input,
};
use concerto::{asset_id, foundation::assets::AssetId};
use concerto_color::Color;
use glam::{Quat, Vec2, Vec3};
use winit::keyboard::{KeyCode, PhysicalKey};

const NINJA_SCENE: AssetId = asset_id!("content/ninja/scene.gasset");
const IDLE_ANIM: AssetId = asset_id!("content/idle/animation_0.gasset");
const WALK_ANIM: AssetId = asset_id!("content/walk/animation_0.gasset");
const STRAFE_LEFT_ANIM: AssetId = asset_id!("content/strafe_left/animation_0.gasset");
const STRAFE_RIGHT_ANIM: AssetId = asset_id!("content/strafe_right/animation_0.gasset");

/// Marks the character entity spawned at startup (the scene spawner / eventual
/// `AnimationStore` holder), so the overlay and gizmo systems can find it.
#[derive(Component)]
pub(crate) struct AnimatedCharacter;

#[derive(Component)]
pub(crate) struct AnimationStore {
    pub(crate) idle: AssetHandle<AnimationClip>,
    pub(crate) walk: AssetHandle<AnimationClip>,
    pub(crate) strafe_left: AssetHandle<AnimationClip>,
    pub(crate) strafe_right: AssetHandle<AnimationClip>,
}

pub(crate) fn spawn_character(mut cmd: CommandQueue, asset_server: Res<AssetServer>) {
    let scene = asset_server.load::<Scene>(NINJA_SCENE);
    let anim_store = AnimationStore {
        idle: asset_server.load::<AnimationClip>(IDLE_ANIM),
        walk: asset_server.load::<AnimationClip>(WALK_ANIM),
        strafe_left: asset_server.load::<AnimationClip>(STRAFE_LEFT_ANIM),
        strafe_right: asset_server.load::<AnimationClip>(STRAFE_RIGHT_ANIM),
    };

    cmd.spawn((
        AnimatedCharacter,
        SceneSpawnerComponent(scene),
        anim_store,
        Transform::from_translation_rotation(Vec3::new(0.0, 0.0, -4.0), Quat::IDENTITY),
    ))
    .add_child((
        Light {
            color: Color::WHITE,
            intensity: 100.0,
            light_type: LightType::Point,
            range: 0.0,
            shadowmaps_enabled: false,
        },
        Transform::from_translation(Vec3::Y * 10.0),
    ));
}

pub(crate) fn setup_animations(
    players: Query<(Entity, &AnimationPlayer), Without<AnimationHandleComponent>>,
    animation_stores: Query<&AnimationStore>,
    asset_server: Res<AssetServer>,
    mut cmd: CommandQueue,
) {
    let Some(anim_store) = animation_stores.iter().next() else {
        return;
    };

    for (player_entity, _player) in players.iter() {
        let mut movement_graph = AnimationGraph::new();

        movement_graph
            .result_node()
            .with_blend_space_2d_input("movement", |context| {
                context
                    .animation_clip_input(anim_store.idle.clone(), Vec2::ZERO)
                    .animation_clip_input(anim_store.strafe_left.clone(), Vec2::new(-1.0, 0.0))
                    .animation_clip_input(anim_store.strafe_right.clone(), Vec2::new(1.0, 0.0))
                    .animation_clip_input(anim_store.walk.clone(), Vec2::new(0.0, 1.0));
            });

        cmd.insert(
            AnimationHandleComponent {
                handle: asset_server.add(movement_graph),
            },
            player_entity,
        );
    }
}

pub(crate) fn update_movement(
    anim_players: Query<&mut AnimationPlayer>,
    input: Res<Input>,
    time: Res<Time>,
) {
    for mut anim_player in anim_players.iter() {
        let mut input_vec = anim_player.get_vec2_param("movement").unwrap_or(Vec2::ZERO);

        let mut added_input = Vec2::ZERO;
        if input.is_held(PhysicalKey::Code(KeyCode::ArrowUp)) {
            added_input += Vec2::Y;
        }

        if input.is_held(PhysicalKey::Code(KeyCode::ArrowDown)) {
            added_input -= Vec2::Y;
        }

        if input.is_held(PhysicalKey::Code(KeyCode::ArrowRight)) {
            added_input += Vec2::X;
        }

        if input.is_held(PhysicalKey::Code(KeyCode::ArrowLeft)) {
            added_input -= Vec2::X;
        }

        let dt = time.delta().as_secs_f32();
        input_vec += added_input * 5.0 * dt;

        if added_input.x == 0.0 {
            input_vec.x *= (1.0 - 5.0 * dt).max(0.0);
        }
        if added_input.y == 0.0 {
            input_vec.y *= (1.0 - 5.0 * dt).max(0.0);
        }

        input_vec = input_vec.clamp(Vec2::NEG_ONE, Vec2::ONE);
        anim_player.set_vec2_param("movement", input_vec);
    }
}
