use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    asset_id,
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
};
use concerto_motion_matching_gizmos::MotionMatchingGizmosPlugin;

use crate::{
    character::{
        CharacterConfig, PlayerSpawner, cycle_displacement, debug_keys, setup_character_animations,
        spawn_character, spawn_forest_walls, update_movement,
    },
    ui::{spawn_grounded_overlay, update_grounded_overlay},
};

mod character;
mod grid;
mod sanatorium;
mod ui;

pub use grid::grid;
pub use sanatorium::sanatorium;

/// Builds tech-demo: the forest, the character, and its overlay.
pub fn game(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(MotionMatchingGizmosPlugin)
        .register_plugin(FrameStatsOverlayPlugin);

    app.set_startup_scene(asset_id!("content/forest/scene.gasset"))
        .register_type::<PlayerSpawner>()
        .insert_resource(CharacterConfig::forest());

    app.add_system(Startup, spawn_forest_walls)
        .add_system(Update, spawn_character)
        .add_system(Startup, spawn_grounded_overlay)
        .add_system(Update, setup_character_animations)
        .add_system(Update, update_movement)
        .add_system(Update, debug_keys)
        .add_system(Update, cycle_displacement)
        .add_system(Update, update_grounded_overlay);
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use concerto::{
        app::{App, StartupScene},
        asset_id,
        ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
        foundation::assets::asset_server::AssetServer,
        reflect::ReflectDefault,
        render::gpu::Gpu,
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
            .and_then(|registry| registry.read().get(TypeId::of::<PlayerSpawner>()))
            .unwrap();
        assert!(spawner.data::<ReflectComponent>().is_some());
        assert!(spawner.data::<ReflectDefault>().is_some());
        assert!(spawner.data::<ReflectSceneComponent>().is_some());
    }

    fn builds_with_no_gpu_and_can_cook_the_motion_database(recipe: fn(&mut App)) {
        let mut app = App::new();
        recipe(&mut app);

        assert!(app.get_resource::<Gpu>().is_none());
        assert!(
            app.get_resource::<AssetServer>()
                .unwrap()
                .can_cook("MotionDatabase")
        );
    }

    #[test]
    fn the_grid_builds_with_no_gpu_and_can_cook_the_motion_database() {
        builds_with_no_gpu_and_can_cook_the_motion_database(crate::grid);
    }

    #[test]
    fn the_forest_builds_with_no_gpu_and_can_cook_the_motion_database() {
        builds_with_no_gpu_and_can_cook_the_motion_database(game);
    }

    #[test]
    fn the_sanatorium_builds_with_no_gpu_and_can_cook_the_motion_database() {
        builds_with_no_gpu_and_can_cook_the_motion_database(crate::sanatorium);
    }
}

#[cfg(test)]
mod derives {
    use concerto::{
        ecs::{Component, Event, Resource, SystemSet, SystemTag, system::schedule::ScheduleLabel},
        foundation::assets::{Asset, LoadableAsset, handle::AssetHandle},
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

    impl LoadableAsset for Glow {}

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
