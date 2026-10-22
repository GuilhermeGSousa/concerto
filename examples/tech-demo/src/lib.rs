use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    asset_id,
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
};

use crate::{
    character::{
        PlayerSpawner, face_camera_direction, setup_character_animations, spawn_character,
        update_movement,
    },
    ui::{spawn_grounded_overlay, update_grounded_overlay},
};

mod character;
mod ui;

/// Builds tech-demo: the forest, the character, and its overlay.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(FrameStatsOverlayPlugin);

    app.set_startup_scene(asset_id!("content/forest/scene.gasset"))
        .register_type::<PlayerSpawner>();

    app.add_system(Update, spawn_character)
        .add_system(Startup, spawn_grounded_overlay)
        .add_system(Update, setup_character_animations)
        .add_system(Update, update_movement)
        .add_system(Update, face_camera_direction)
        .add_system(Update, update_grounded_overlay);
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use concerto::{
        app::StartupScene,
        asset_id,
        ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
        reflect::ReflectDefault,
    };

    use crate::{character::PlayerSpawner, game};

    #[test]
    fn the_recipe_declares_the_forest_and_registers_the_spawner_for_reflection() {
        let Some(app) = concerto::headless(game) else {
            return;
        };

        assert_eq!(
            app.get_resource::<StartupScene>(),
            Some(&StartupScene(asset_id!("content/forest/scene.gasset")))
        );
        let spawner = app
            .get_resource::<AppTypeRegistry>()
            .and_then(|registry| registry.get(TypeId::of::<PlayerSpawner>()))
            .unwrap();
        assert!(spawner.data::<ReflectComponent>().is_some());
        assert!(spawner.data::<ReflectDefault>().is_some());
        assert!(spawner.data::<ReflectSceneComponent>().is_some());
    }
}

#[cfg(test)]
mod derives {
    use concerto::{
        ecs::{Component, Event, Resource, SystemSet, SystemTag, system::schedule::ScheduleLabel},
        foundation::assets::{Asset, handle::AssetHandle},
        reflect::Reflect,
        render::{AsBindGroup, assets::texture::Texture},
    };

    #[derive(Component, Reflect, Default)]
    struct Health {
        value: f32,
    }

    #[derive(Resource)]
    struct Score(u32);

    #[derive(Event)]
    struct Hit;

    #[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
    struct Gameplay;

    #[derive(SystemTag, Clone, Debug, PartialEq, Eq, Hash)]
    struct Tagged;

    #[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
    struct Phase;

    #[derive(Asset, AsBindGroup, serde::Serialize, serde::Deserialize)]
    #[material(cull_mode = "none", blend = "alpha")]
    struct Glow {
        #[uniform(0)]
        color: [f32; 4],
        #[texture(1)]
        #[sampler(2)]
        texture: Option<AssetHandle<Texture>>,
    }

    #[test]
    fn engine_derives_compile_from_a_crate_that_depends_on_concerto_alone() {
        let glow = Glow {
            color: [1.0; 4],
            texture: None,
        };
        let _ = (
            Health::default().value,
            Score(0).0,
            Hit,
            Gameplay,
            Tagged,
            Phase,
        );
        assert!(glow.texture.is_none() && glow.color[0] == 1.0);
    }
}
