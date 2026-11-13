mod body;
mod character;
mod footsteps;
mod grid;
mod retarget;
mod sanatorium;
mod ui;

pub use grid::grid;
pub use retarget::{retarget, retargeted_sanatorium};
pub use sanatorium::sanatorium;

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use concerto::{
        app::App,
        ecs::reflect::{AppTypeRegistry, ReflectComponent, ReflectSceneComponent},
        foundation::assets::asset_server::AssetServer,
        reflect::ReflectDefault,
        render::gpu::Gpu,
    };

    use concerto_procedural_animation_gizmos::ProceduralAnimationGizmosPlugin;

    use crate::{character::PlayerSpawner, retarget::RetargetedPlayer};

    #[test]
    fn the_sanatorium_registers_the_spawner_for_reflection() {
        let Some(app) = concerto::headless(crate::sanatorium) else {
            return;
        };

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
    fn the_sanatorium_builds_with_no_gpu_and_can_cook_the_motion_database() {
        builds_with_no_gpu_and_can_cook_the_motion_database(crate::sanatorium);
    }

    #[test]
    fn the_retargeted_grid_builds_with_no_gpu_and_can_cook_the_motion_database() {
        builds_with_no_gpu_and_can_cook_the_motion_database(crate::retarget);
    }

    #[test]
    fn only_the_retargeted_levels_play_the_retargeted_player() {
        let recipes: [fn(&mut App); 4] = [
            crate::grid,
            crate::sanatorium,
            crate::retarget,
            crate::retargeted_sanatorium,
        ];
        let retargeted: Vec<bool> = recipes
            .into_iter()
            .map(|recipe| {
                let mut app = App::new();
                recipe(&mut app);
                app.get_resource::<RetargetedPlayer>().is_some()
            })
            .collect();
        assert_eq!(retargeted, vec![false, false, true, true]);
    }

    #[test]
    fn every_level_draws_the_foot_locks() {
        let recipes: [fn(&mut App); 3] = [crate::grid, crate::sanatorium, crate::retarget];
        for recipe in recipes {
            let mut app = App::new();
            recipe(&mut app);

            assert!(app.is_plugin_added::<ProceduralAnimationGizmosPlugin>());
        }
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
