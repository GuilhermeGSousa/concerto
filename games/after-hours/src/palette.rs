//! Shared meshes and materials, created once at startup.
use concerto::{
    color::Color,
    ecs::{Res, ResMut, Resource},
    foundation::assets::{asset_server::AssetServer, handle::AssetHandle},
    render::assets::{material::StandardMaterial, mesh::Mesh, texture::Texture},
};
use glam::Vec3;

use crate::{meshes, textures};

/// A mannequin finish: the body and the joint caps.
#[derive(Clone)]
pub struct Finish {
    pub main: AssetHandle<StandardMaterial>,
    pub joints: AssetHandle<StandardMaterial>,
}

#[derive(Resource)]
pub struct Palette {
    pub floor: AssetHandle<StandardMaterial>,
    pub ceiling: AssetHandle<StandardMaterial>,
    pub drywall: AssetHandle<StandardMaterial>,
    pub steel: AssetHandle<StandardMaterial>,
    /// Stock on the shelves: cardboard plus a few printed-package colours.
    pub stock: Vec<AssetHandle<StandardMaterial>>,

    pub panel_mesh: AssetHandle<Mesh>,
    pub panel_on: AssetHandle<StandardMaterial>,
    pub panel_off: AssetHandle<StandardMaterial>,

    pub key_mesh: AssetHandle<Mesh>,
    pub key: AssetHandle<StandardMaterial>,
    pub battery_mesh: AssetHandle<Mesh>,
    pub battery: AssetHandle<StandardMaterial>,

    pub door: AssetHandle<StandardMaterial>,
    pub sign_mesh: AssetHandle<Mesh>,
    pub sign_locked: AssetHandle<StandardMaterial>,
    pub sign_unlocked: AssetHandle<StandardMaterial>,

    /// Mannequin finishes.
    pub finishes: Vec<Finish>,
}

pub fn solid(color: Color, roughness: f32) -> StandardMaterial {
    let mut material = StandardMaterial::new(None, None).with_base_color_factor(color);
    material.set_roughness_factor(roughness);
    material.set_metallic_factor(0.0);
    material
}

pub fn glowing(color: Color, emissive: Vec3) -> StandardMaterial {
    let mut material = solid(color, 0.4);
    material.set_emissive_factor(emissive);
    material
}

fn textured(texture: AssetHandle<Texture>, meters: f32, roughness: f32) -> StandardMaterial {
    let mut material = StandardMaterial::new(Some(texture), None).with_uv_scale([1.0 / meters; 2]);
    material.set_roughness_factor(roughness);
    material.set_metallic_factor(0.0);
    material
}

pub fn create_palette(server: Res<AssetServer>, mut slot: ResMut<PaletteSlot>) {
    let server = &*server;
    let cardboard = server.add(textures::cardboard());
    let steel_texture = server.add(textures::steel());

    let mut stock = vec![server.add(textured(cardboard.clone(), 0.6, 0.9))];
    for color in [
        Color::srgba(0.55, 0.12, 0.1, 1.0),
        Color::srgba(0.12, 0.25, 0.45, 1.0),
        Color::srgba(0.8, 0.75, 0.6, 1.0),
        Color::srgba(0.2, 0.35, 0.2, 1.0),
    ] {
        let mut material = textured(cardboard.clone(), 0.6, 0.7).with_base_color_factor(color);
        material.set_metallic_factor(0.0);
        stock.push(server.add(material));
    }

    let finish = |main: Color, main_rough: f32, joints: Color| Finish {
        main: server.add(solid(main, main_rough)),
        joints: server.add(solid(joints, 0.5)),
    };

    slot.0 = Some(Palette {
        floor: server.add(textured(server.add(textures::floor()), 2.0, 0.55)),
        ceiling: server.add(textured(server.add(textures::ceiling()), 1.2, 0.95)),
        drywall: server.add(textured(server.add(textures::drywall()), 3.4, 0.9)),
        steel: server.add(textured(steel_texture, 1.0, 0.5)),
        stock,

        panel_mesh: server.add(meshes::cuboid(Vec3::new(0.6, 0.03, 0.15))),
        panel_on: server.add(glowing(
            Color::srgba(0.9, 0.95, 1.0, 1.0),
            Vec3::new(2.2, 2.5, 2.8),
        )),
        panel_off: server.add(solid(Color::srgba(0.35, 0.35, 0.34, 1.0), 0.4)),

        key_mesh: server.add(meshes::cuboid(Vec3::new(0.05, 0.1, 0.012))),
        key: server.add(glowing(
            Color::srgba(1.0, 0.8, 0.3, 1.0),
            Vec3::new(1.4, 0.9, 0.2),
        )),
        battery_mesh: server.add(meshes::cylinder(0.05, 0.2, 10)),
        battery: server.add(glowing(
            Color::srgba(0.2, 0.8, 0.3, 1.0),
            Vec3::new(0.1, 0.6, 0.15),
        )),

        door: server.add(solid(Color::srgba(0.35, 0.08, 0.06, 1.0), 0.6)),
        sign_mesh: server.add(meshes::cuboid(Vec3::new(0.45, 0.14, 0.04))),
        sign_locked: server.add(glowing(
            Color::srgba(0.8, 0.1, 0.08, 1.0),
            Vec3::new(2.5, 0.2, 0.15),
        )),
        sign_unlocked: server.add(glowing(
            Color::srgba(0.1, 0.8, 0.2, 1.0),
            Vec3::new(0.2, 3.0, 0.4),
        )),

        finishes: vec![
            finish(
                Color::srgba(0.86, 0.85, 0.82, 1.0),
                0.25,
                Color::srgba(0.55, 0.55, 0.55, 1.0),
            ),
            finish(
                Color::srgba(0.08, 0.08, 0.09, 1.0),
                0.35,
                Color::srgba(0.25, 0.25, 0.27, 1.0),
            ),
            finish(
                Color::srgba(0.78, 0.6, 0.5, 1.0),
                0.45,
                Color::srgba(0.5, 0.36, 0.3, 1.0),
            ),
        ],
    });
}

/// Holds the palette once `create_palette` has run (in `Startup`).
#[derive(Resource, Default)]
pub struct PaletteSlot(pub Option<Palette>);

impl PaletteSlot {
    pub fn get(&self) -> Option<&Palette> {
        self.0.as_ref()
    }
}
