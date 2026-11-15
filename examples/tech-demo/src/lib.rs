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
    use concerto::{app::App, foundation::assets::asset_server::AssetServer, render::gpu::Gpu};

    #[test]
    fn every_level_builds_with_no_gpu_and_can_cook_the_motion_database() {
        let recipes: [fn(&mut App); 4] = [
            crate::grid,
            crate::sanatorium,
            crate::retarget,
            crate::retargeted_sanatorium,
        ];
        for recipe in recipes {
            let mut app = App::new();
            recipe(&mut app);

            assert!(app.get_resource::<Gpu>().is_none());
            assert!(
                app.get_resource::<AssetServer>()
                    .unwrap()
                    .can_cook("MotionDatabase")
            );
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
