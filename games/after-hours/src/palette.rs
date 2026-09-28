//! Shared meshes and materials, created once at startup.
use concerto::{
    color::Color,
    ecs::{Res, ResMut, Resource},
    foundation::assets::{asset_server::AssetServer, handle::AssetHandle},
    render::assets::{material::StandardMaterial, mesh::Mesh, texture::Texture},
};
use glam::Vec3;

use crate::{
    level::RoomKind,
    meshes,
    textures::{self, PICTURES, Picture},
};

/// A figure finish: the body and the joint caps.
#[derive(Clone)]
pub struct Finish {
    pub main: AssetHandle<StandardMaterial>,
    pub joints: AssetHandle<StandardMaterial>,
}

/// Index of Clara's grey finish in [`Palette::finishes`].
pub const CLARA_FINISH: usize = 3;
/// How many finishes ordinary figures pick from.
pub const COMMON_FINISHES: usize = 3;

#[derive(Resource)]
pub struct Palette {
    pub floor: AssetHandle<StandardMaterial>,
    pub ceiling: AssetHandle<StandardMaterial>,
    pub wainscot: AssetHandle<StandardMaterial>,
    wallpapers: Vec<AssetHandle<StandardMaterial>>,
    pub linen: AssetHandle<StandardMaterial>,
    pub wood: AssetHandle<StandardMaterial>,
    pub gilt: AssetHandle<StandardMaterial>,
    pub marble: AssetHandle<StandardMaterial>,
    pub soot: AssetHandle<StandardMaterial>,
    pub brass: AssetHandle<StandardMaterial>,
    pub wax: AssetHandle<StandardMaterial>,
    pub books: Vec<AssetHandle<StandardMaterial>>,
    pub rugs: Vec<AssetHandle<StandardMaterial>>,
    pictures: Vec<(Picture, AssetHandle<StandardMaterial>)>,

    pub canvas_mesh: AssetHandle<Mesh>,
    pub flame_mesh: AssetHandle<Mesh>,
    pub flame: AssetHandle<StandardMaterial>,
    pub flame_out: AssetHandle<StandardMaterial>,
    pub moonlight: AssetHandle<StandardMaterial>,
    pub window: AssetHandle<StandardMaterial>,
    pub velvet: AssetHandle<StandardMaterial>,

    pub tag_mesh: AssetHandle<Mesh>,
    pub tag: AssetHandle<StandardMaterial>,
    pub tag_done: AssetHandle<StandardMaterial>,
    pub page_mesh: AssetHandle<Mesh>,
    pub page: AssetHandle<StandardMaterial>,
    pub oil_mesh: AssetHandle<Mesh>,
    pub oil: AssetHandle<StandardMaterial>,
    pub ledger: AssetHandle<StandardMaterial>,
    pub ledger_ready: AssetHandle<StandardMaterial>,

    /// Figure finishes; the last is Clara's.
    pub finishes: Vec<Finish>,
}

impl Palette {
    pub fn wallpaper(&self, kind: RoomKind) -> &AssetHandle<StandardMaterial> {
        let i = match kind {
            RoomKind::Hall | RoomKind::Corridor => 0,
            RoomKind::Studio => 1,
            RoomKind::Gallery | RoomKind::Dining => 2,
            RoomKind::Parlour | RoomKind::Library => 3,
            RoomKind::Bedroom => 4,
        };
        &self.wallpapers[i]
    }

    pub fn picture(&self, kind: Picture) -> &AssetHandle<StandardMaterial> {
        &self
            .pictures
            .iter()
            .find(|(k, _)| *k == kind)
            .expect("every picture is painted")
            .1
    }
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
    let leather = server.add(textures::leather());
    let books = [
        Color::srgba(0.35, 0.08, 0.06, 1.0),
        Color::srgba(0.1, 0.18, 0.1, 1.0),
        Color::srgba(0.12, 0.1, 0.2, 1.0),
        Color::srgba(0.4, 0.3, 0.18, 1.0),
        Color::srgba(0.15, 0.1, 0.07, 1.0),
    ]
    .map(|c| server.add(textured(leather.clone(), 0.3, 0.7).with_base_color_factor(c)))
    .to_vec();

    let wallpapers = [
        ([0.16, 0.2, 0.14], [0.24, 0.27, 0.17], 201),
        ([0.2, 0.07, 0.06], [0.28, 0.12, 0.08], 203),
        ([0.12, 0.13, 0.16], [0.2, 0.2, 0.2], 205),
        ([0.28, 0.22, 0.12], [0.36, 0.3, 0.16], 207),
        ([0.22, 0.2, 0.22], [0.3, 0.26, 0.28], 209),
    ]
    .map(|(ground, figure, seed)| {
        server.add(textured(
            server.add(textures::wallpaper(ground, figure, seed)),
            0.5,
            0.85,
        ))
    })
    .to_vec();

    let rugs = [1, 2]
        .map(|seed| {
            let mut m = StandardMaterial::new(Some(server.add(textures::rug(seed))), None);
            m.set_roughness_factor(1.0);
            m.set_metallic_factor(0.0);
            server.add(m)
        })
        .to_vec();

    let pictures = PICTURES
        .map(|kind| {
            let mut m = StandardMaterial::new(Some(server.add(textures::picture(kind))), None);
            m.set_roughness_factor(0.45);
            m.set_metallic_factor(0.0);
            (kind, server.add(m))
        })
        .to_vec();

    let paper = server.add(textures::paper());
    let paper_glow = |emissive: Vec3| {
        let mut m = StandardMaterial::new(Some(paper.clone()), None);
        m.set_roughness_factor(0.9);
        m.set_metallic_factor(0.0);
        m.set_emissive_factor(emissive);
        server.add(m)
    };

    let finish = |main: Color, main_rough: f32, joints: Color| Finish {
        main: server.add(solid(main, main_rough)),
        joints: server.add(solid(joints, 0.5)),
    };

    slot.0 = Some(Palette {
        floor: server.add(textured(server.add(textures::floorboards()), 2.0, 0.5)),
        ceiling: server.add(
            textured(server.add(textures::plaster()), 2.0, 0.95)
                .with_base_color_factor(Color::srgba(0.6, 0.58, 0.55, 1.0)),
        ),
        wainscot: server.add(textured(server.add(textures::wainscot()), 1.0, 0.55)),
        wallpapers,
        linen: server.add(textured(server.add(textures::linen()), 1.2, 1.0)),
        wood: server.add(textured(server.add(textures::wood()), 0.8, 0.45)),
        gilt: {
            let mut m = textured(server.add(textures::gilt()), 0.3, 0.35);
            m.set_metallic_factor(0.6);
            server.add(m)
        },
        marble: server.add(solid(Color::srgba(0.55, 0.53, 0.5, 1.0), 0.25)),
        soot: server.add(solid(Color::srgba(0.02, 0.02, 0.02, 1.0), 1.0)),
        brass: {
            let mut m = solid(Color::srgba(0.55, 0.4, 0.16, 1.0), 0.35);
            m.set_metallic_factor(0.8);
            server.add(m)
        },
        wax: server.add(solid(Color::srgba(0.85, 0.8, 0.66, 1.0), 0.6)),
        books,
        rugs,
        pictures,

        canvas_mesh: server.add(meshes::canvas()),
        flame_mesh: server.add(meshes::cuboid(Vec3::new(0.012, 0.03, 0.012))),
        flame: server.add(glowing(
            Color::srgba(1.0, 0.8, 0.4, 1.0),
            Vec3::new(4.0, 2.4, 0.8),
        )),
        flame_out: server.add(solid(Color::srgba(0.05, 0.04, 0.03, 1.0), 1.0)),
        window: {
            let mut m = StandardMaterial::new(Some(server.add(textures::window_glass())), None)
                .with_base_color_factor(Color::srgba(0.1, 0.1, 0.1, 1.0));
            m.set_roughness_factor(0.2);
            m.set_metallic_factor(0.0);
            m.set_emissive_factor(Vec3::new(0.18, 0.2, 0.28));
            server.add(m)
        },
        velvet: server.add(solid(Color::srgba(0.22, 0.04, 0.05, 1.0), 0.95)),
        moonlight: server.add(glowing(
            Color::srgba(0.2, 0.25, 0.35, 1.0),
            Vec3::new(0.25, 0.32, 0.5),
        )),

        tag_mesh: server.add(meshes::cuboid(Vec3::new(0.05, 0.035, 0.004))),
        tag: paper_glow(Vec3::new(0.35, 0.3, 0.2)),
        tag_done: paper_glow(Vec3::ZERO),
        page_mesh: server.add(meshes::cuboid(Vec3::new(0.1, 0.004, 0.14))),
        page: paper_glow(Vec3::new(0.25, 0.22, 0.15)),
        oil_mesh: server.add(meshes::cylinder(0.06, 0.16, 12)),
        oil: {
            let mut m = solid(Color::srgba(0.3, 0.32, 0.2, 1.0), 0.4);
            m.set_metallic_factor(0.6);
            m.set_emissive_factor(Vec3::new(0.08, 0.08, 0.04));
            server.add(m)
        },
        ledger: paper_glow(Vec3::new(0.05, 0.04, 0.03)),
        ledger_ready: paper_glow(Vec3::new(0.9, 0.75, 0.45)),

        finishes: vec![
            finish(
                Color::srgba(0.72, 0.56, 0.38, 1.0),
                0.5,
                Color::srgba(0.45, 0.32, 0.2, 1.0),
            ),
            finish(
                Color::srgba(0.36, 0.23, 0.14, 1.0),
                0.4,
                Color::srgba(0.2, 0.12, 0.07, 1.0),
            ),
            finish(
                Color::srgba(0.8, 0.72, 0.58, 1.0),
                0.6,
                Color::srgba(0.55, 0.47, 0.36, 1.0),
            ),
            finish(
                Color::srgba(0.5, 0.52, 0.55, 1.0),
                0.3,
                Color::srgba(0.3, 0.31, 0.34, 1.0),
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
