//! Builds a night's house from a [`Level`]: merged static meshes per material
//! for the shell and furniture, colliders, candles, pictures, and the things
//! the player picks up or catalogues.
use concerto::{
    color::Color,
    ecs::{CommandQueue, Component, Entity},
    foundation::{
        assets::{asset_server::AssetServer, handle::AssetHandle},
        transform::Transform,
    },
    mesh::MeshComponent,
    physics::collider::Collider,
    render::{
        MaterialComponent,
        assets::{material::StandardMaterial, mesh::Mesh},
        components::{light::Light, render_entity::SyncWithRenderWorld},
    },
};
use glam::{Quat, Vec3};

use crate::{
    game::Rand,
    level::{CELL, Cell, Dir, Level, RoomKind, WallSpot},
    meshes::MeshBuilder,
    palette::Palette,
    textures::Picture,
};

pub const CEILING: f32 = 3.6;
pub const WALL_THICKNESS: f32 = 0.24;
pub const DOOR_WIDTH: f32 = 1.9;
pub const DOOR_HEIGHT: f32 = 2.6;
const WAINSCOT: f32 = 1.0;

/// Everything spawned for the current night; despawned wholesale on rebuild.
#[derive(Component)]
pub struct HouseEntity;

/// A light the observation rule must account for.
#[derive(Component)]
pub struct LightSource;

#[derive(Component)]
pub struct Candle {
    pub base_intensity: f32,
    pub flame: Entity,
    pub phase: f32,
    /// Seconds of guttering left before it goes out; `None` while steady.
    pub guttering: Option<f32>,
    pub out: bool,
}

pub const CANDLE_RANGE: f32 = 6.0;
pub const CANDLE_INTENSITY: f32 = 5.0;

/// A picture marked for the sale.
#[derive(Component)]
pub struct Lot {
    pub number: u32,
    pub title: &'static str,
    /// Middle of the canvas.
    pub center: Vec3,
    pub tag: Entity,
    pub done: bool,
}

#[derive(Component)]
pub struct OilPickup {
    pub base: Vec3,
}

#[derive(Component)]
pub struct PagePickup {
    pub base: Vec3,
}

/// The ledger on the hall desk: sign it to end the night.
#[derive(Component)]
pub struct Ledger {
    pub point: Vec3,
    pub book: Entity,
}

const TITLES: &[&str] = &[
    "The Sitter at Rest",
    "Figure with a Grey Shawl",
    "Study of Hands",
    "The Long Room at Evening",
    "Two Figures, Waiting",
    "The Model Turned Away",
    "Interior with Easel",
    "The Sitter, Unfinished",
    "A Figure on the Stair",
    "Portrait without a Face",
    "The Night Studio",
    "Figure at the Window",
    "The Patient One",
    "Still Life with Lamp",
    "The Eleventh Figure",
];

struct Batches<'a> {
    palette: &'a Palette,
    builders: Vec<(AssetHandle<StandardMaterial>, MeshBuilder)>,
}

impl<'a> Batches<'a> {
    fn new(palette: &'a Palette) -> Self {
        Self {
            palette,
            builders: Vec::new(),
        }
    }

    fn of(&mut self, material: &AssetHandle<StandardMaterial>) -> &mut MeshBuilder {
        let i = match self
            .builders
            .iter()
            .position(|(m, _)| m.id() == material.id())
        {
            Some(i) => i,
            None => {
                self.builders
                    .push((material.clone(), MeshBuilder::default()));
                self.builders.len() - 1
            }
        };
        &mut self.builders[i].1
    }

    fn spawn(self, cmd: &mut CommandQueue, server: &AssetServer) {
        for (material, builder) in self.builders {
            if builder.is_empty() {
                continue;
            }
            cmd.spawn((
                HouseEntity,
                SyncWithRenderWorld,
                MeshComponent {
                    handle: server.add(builder.build()),
                },
                MaterialComponent { handle: material },
                Transform::IDENTITY,
            ));
        }
    }
}

fn collider(cmd: &mut CommandQueue, center: Vec3, half: Vec3) {
    cmd.spawn((
        HouseEntity,
        Collider::cuboid(half.x.max(0.01), half.y.max(0.01), half.z.max(0.01)),
        Transform::from_translation(center),
    ));
}

fn spawn_prop(
    cmd: &mut CommandQueue,
    mesh: &AssetHandle<Mesh>,
    material: &AssetHandle<StandardMaterial>,
    transform: Transform,
) -> Entity {
    cmd.spawn((
        HouseEntity,
        SyncWithRenderWorld,
        MeshComponent {
            handle: mesh.clone(),
        },
        MaterialComponent {
            handle: material.clone(),
        },
        transform,
    ))
    .entity()
}

#[derive(Clone, Copy)]
struct Frame {
    edge: Vec3,
    along: Vec3,
    inward: Vec3,
}

impl Frame {
    fn of(spot: WallSpot) -> Self {
        let out = spot.dir.vector();
        Self {
            edge: spot.cell.center() + out * CELL * 0.5,
            along: Vec3::new(-out.z, 0.0, out.x),
            inward: -out,
        }
    }

    fn at(&self, a: f32, y: f32, d: f32) -> Vec3 {
        self.edge + self.along * a + Vec3::Y * y + self.inward * d
    }

    fn boxed(&self, a0: f32, a1: f32, y0: f32, y1: f32, d0: f32, d1: f32) -> (Vec3, Vec3) {
        let center = self.at((a0 + a1) * 0.5, (y0 + y1) * 0.5, (d0 + d1) * 0.5);
        let half = (self.along * (a1 - a0) * 0.5).abs()
            + Vec3::Y * (y1 - y0) * 0.5
            + (self.inward * (d1 - d0) * 0.5).abs();
        (center, half)
    }

    fn facing(&self) -> Quat {
        Quat::from_rotation_y(self.inward.x.atan2(self.inward.z))
    }
}

fn wall_face(
    batches: &mut Batches,
    frame: &Frame,
    paper: &AssetHandle<StandardMaterial>,
    a0: f32,
    a1: f32,
    top: f32,
) {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let (c, h) = frame.boxed(a0, a1, WAINSCOT, top, 0.0, t);
    batches.of(paper).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(a0, a1, 0.0, WAINSCOT, 0.0, t + 0.025);
    batches.of(&palette.wainscot).cuboid(c, h, &[Vec3::NEG_Y]);
    let (c, h) = frame.boxed(a0, a1, WAINSCOT - 0.03, WAINSCOT + 0.04, 0.0, t + 0.05);
    batches.of(&palette.wood).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(a0, a1, 0.0, 0.16, 0.0, t + 0.045);
    batches.of(&palette.wood).cuboid(c, h, &[Vec3::NEG_Y]);
}

fn cornice(batches: &mut Batches, frame: &Frame, a0: f32, a1: f32) {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let (c, h) = frame.boxed(a0, a1, CEILING - 0.18, CEILING, 0.0, t + 0.09);
    batches.of(&palette.ceiling).cuboid(c, h, &[Vec3::Y]);
    let (c, h) = frame.boxed(a0, a1, CEILING - 0.45, CEILING - 0.42, 0.0, t + 0.03);
    batches.of(&palette.wood).cuboid(c, h, &[]);
}

fn build_wall_side(batches: &mut Batches, level: &Level, spot: WallSpot, doorway: bool) {
    let frame = Frame::of(spot);
    let kind = level
        .room(spot.cell)
        .map(|r| r.kind)
        .unwrap_or(RoomKind::Hall);
    let paper = batches.palette.wallpaper(kind).clone();
    let half = CELL * 0.5;
    if doorway {
        let d = DOOR_WIDTH * 0.5;
        wall_face(batches, &frame, &paper, -half, -d, CEILING);
        wall_face(batches, &frame, &paper, d, half, CEILING);
        let t = WALL_THICKNESS * 0.5;
        let (c, h) = frame.boxed(-d, d, DOOR_HEIGHT, CEILING, 0.0, t);
        batches.of(&paper).cuboid(c, h, &[]);
        let wood = batches.palette.wood.clone();
        for side in [-1.0, 1.0] {
            let (c, h) = frame.boxed(
                side * d - 0.09,
                side * d + 0.09,
                0.0,
                DOOR_HEIGHT + 0.1,
                0.0,
                t + 0.04,
            );
            batches.of(&wood).cuboid(c, h, &[Vec3::NEG_Y]);
        }
        let (c, h) = frame.boxed(
            -d - 0.09,
            d + 0.09,
            DOOR_HEIGHT,
            DOOR_HEIGHT + 0.14,
            0.0,
            t + 0.05,
        );
        batches.of(&wood).cuboid(c, h, &[]);
    } else {
        wall_face(batches, &frame, &paper, -half, half, CEILING);
    }
    cornice(batches, &frame, -half, half);
}

fn wall_colliders(cmd: &mut CommandQueue, level: &Level) {
    for wall in level.walls() {
        let thickness = if wall.outer {
            WALL_THICKNESS * 2.0
        } else {
            WALL_THICKNESS
        };
        let footprint = wall.half_extents(thickness);
        let along = if wall.along_x { Vec3::X } else { Vec3::Z };
        let across = if wall.along_x { Vec3::Z } else { Vec3::X };
        let long = CELL * 0.5 + thickness * 0.5;
        let thick = across
            * if wall.along_x {
                footprint.y
            } else {
                footprint.x
            };
        if wall.door {
            let piece = (long - DOOR_WIDTH * 0.5) * 0.5;
            for side in [-1.0, 1.0] {
                let center = wall.center
                    + along * side * (DOOR_WIDTH * 0.5 + piece)
                    + Vec3::Y * CEILING * 0.5;
                collider(cmd, center, along * piece + thick + Vec3::Y * CEILING * 0.5);
            }
            let lintel = (CEILING - DOOR_HEIGHT) * 0.5;
            collider(
                cmd,
                wall.center + Vec3::Y * (DOOR_HEIGHT + lintel),
                along * DOOR_WIDTH * 0.5 + thick + Vec3::Y * lintel,
            );
        } else {
            collider(
                cmd,
                wall.center + Vec3::Y * CEILING * 0.5,
                along * long + thick + Vec3::Y * CEILING * 0.5,
            );
        }
    }
}

fn hang_picture(
    cmd: &mut CommandQueue,
    batches: &mut Batches,
    frame: &Frame,
    a: f32,
    y: f32,
    size: (f32, f32),
    picture: Picture,
) -> Vec3 {
    let t = WALL_THICKNESS * 0.5;
    let (w, h) = size;
    let border = 0.08;
    let gilt = batches.palette.gilt.clone();
    for (a0, a1, y0, y1) in [
        (
            a - w * 0.5 - border,
            a + w * 0.5 + border,
            y + h * 0.5,
            y + h * 0.5 + border,
        ),
        (
            a - w * 0.5 - border,
            a + w * 0.5 + border,
            y - h * 0.5 - border,
            y - h * 0.5,
        ),
        (a - w * 0.5 - border, a - w * 0.5, y - h * 0.5, y + h * 0.5),
        (a + w * 0.5, a + w * 0.5 + border, y - h * 0.5, y + h * 0.5),
    ] {
        let (c, half) = frame.boxed(a0, a1, y0, y1, t, t + 0.07);
        batches.of(&gilt).cuboid(c, half, &[]);
    }
    let center = frame.at(a, y, t + 0.035);
    let palette = batches.palette;
    cmd.spawn((
        HouseEntity,
        SyncWithRenderWorld,
        MeshComponent {
            handle: palette.canvas_mesh.clone(),
        },
        MaterialComponent {
            handle: palette.picture(picture).clone(),
        },
        Transform {
            translation: center,
            rotation: frame.facing(),
            scale: Vec3::new(w, h, 1.0),
        },
    ));
    center
}

fn sconce(
    cmd: &mut CommandQueue,
    batches: &mut Batches,
    frame: &Frame,
    a: f32,
    lit: bool,
    rand: &mut Rand,
) {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let brass = palette.brass.clone();
    let wax = palette.wax.clone();
    let (c, h) = frame.boxed(a - 0.07, a + 0.07, 1.62, 1.86, t, t + 0.02);
    batches.of(&brass).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(a - 0.015, a + 0.015, 1.68, 1.71, t, t + 0.16);
    batches.of(&brass).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(a - 0.05, a + 0.05, 1.7, 1.72, t + 0.1, t + 0.2);
    batches.of(&brass).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(a - 0.02, a + 0.02, 1.72, 1.86, t + 0.13, t + 0.17);
    batches.of(&wax).cuboid(c, h, &[]);
    let flame_pos = frame.at(a, 1.9, t + 0.15);
    let flame = spawn_prop(
        cmd,
        &palette.flame_mesh,
        if lit {
            &palette.flame
        } else {
            &palette.flame_out
        },
        Transform::from_translation(flame_pos),
    );
    if lit {
        cmd.spawn((
            HouseEntity,
            LightSource,
            Candle {
                base_intensity: CANDLE_INTENSITY,
                flame,
                phase: rand.range(0.0, 100.0),
                guttering: None,
                out: false,
            },
            SyncWithRenderWorld,
            Light::point_light()
                .with_color(Color::srgba(1.0, 0.62, 0.3, 1.0))
                .with_intensity(CANDLE_INTENSITY)
                .with_range(CANDLE_RANGE),
            Transform::from_translation(flame_pos + frame.inward * 0.12),
        ));
    }
}

fn window(cmd: &mut CommandQueue, batches: &mut Batches, frame: &Frame) {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let (w, y0, y1) = (1.1, 0.9, 3.0);
    let wood = palette.wood.clone();
    for (a0, a1, b0, b1) in [
        (-w * 0.5 - 0.08, w * 0.5 + 0.08, y1, y1 + 0.1),
        (-w * 0.5 - 0.08, w * 0.5 + 0.08, y0 - 0.12, y0),
        (-w * 0.5 - 0.08, -w * 0.5, y0, y1),
        (w * 0.5, w * 0.5 + 0.08, y0, y1),
        (
            -w * 0.5,
            w * 0.5,
            (y0 + y1) * 0.5 - 0.03,
            (y0 + y1) * 0.5 + 0.03,
        ),
    ] {
        let (c, h) = frame.boxed(a0, a1, b0, b1, t, t + 0.06);
        batches.of(&wood).cuboid(c, h, &[]);
    }
    let velvet = palette.velvet.clone();
    for side in [-1.0, 1.0] {
        let a = side * (w * 0.5 + 0.12);
        let (c, h) = frame.boxed(a - 0.2, a + 0.2, 0.0, 3.3, t + 0.06, t + 0.2);
        batches.of(&velvet).cuboid(c, h, &[Vec3::NEG_Y]);
    }
    let (c, h) = frame.boxed(
        -w * 0.5 - 0.4,
        w * 0.5 + 0.4,
        3.25,
        3.35,
        t + 0.05,
        t + 0.24,
    );
    batches.of(&wood).cuboid(c, h, &[]);
    cmd.spawn((
        HouseEntity,
        SyncWithRenderWorld,
        MeshComponent {
            handle: palette.canvas_mesh.clone(),
        },
        MaterialComponent {
            handle: palette.window.clone(),
        },
        Transform {
            translation: frame.at(0.0, (y0 + y1) * 0.5, t + 0.01),
            rotation: frame.facing(),
            scale: Vec3::new(w, y1 - y0, 1.0),
        },
    ));
}

fn furnish_wall(
    batches: &mut Batches,
    frame: &Frame,
    kind: RoomKind,
    rand: &mut Rand,
) -> Option<(Vec3, Vec3)> {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let linen = palette.linen.clone();
    let wood = palette.wood.clone();
    let choice = rand.unit();
    let mut bounds: Option<(Vec3, Vec3)> = None;
    let mut add = |b: &mut Batches, m: &AssetHandle<StandardMaterial>, c: Vec3, h: Vec3| {
        b.of(m).cuboid(c, h, &[Vec3::NEG_Y]);
        bounds = Some(match bounds {
            None => (c - h, c + h),
            Some((lo, hi)) => (lo.min(c - h), hi.max(c + h)),
        });
    };
    let a = rand.range(-0.6, 0.6);
    match kind {
        RoomKind::Library => {
            let (c, h) = frame.boxed(-1.5, 1.5, 0.0, 2.4, t, t + 0.4);
            add(batches, &wood, c, h);
            let books = palette.books.clone();
            for shelf in 0..5 {
                let y = 0.12 + shelf as f32 * 0.46;
                let mut x = -1.4;
                while x < 1.35 {
                    let w = rand.range(0.03, 0.07);
                    let height = rand.range(0.24, 0.36);
                    if rand.unit() > 0.08 {
                        let (c, h) = frame.boxed(x, x + w, y, y + height, t + 0.4, t + 0.42);
                        let m = &books[rand.index(books.len())];
                        batches.of(m).cuboid(c, h, &[Vec3::NEG_Y]);
                    }
                    x += w + 0.004;
                }
            }
        }
        RoomKind::Dining if choice < 0.5 => {
            let (c, h) = frame.boxed(a - 0.8, a + 0.8, 0.0, 0.92, t, t + 0.5);
            add(batches, &wood, c, h);
        }
        RoomKind::Hall | RoomKind::Corridor => {
            if choice < 0.4 {
                let (c, h) = frame.boxed(a - 0.26, a + 0.26, 0.0, 2.2, t, t + 0.36);
                add(batches, &wood, c, h);
                let (c, h) = frame.boxed(a - 0.16, a + 0.16, 1.7, 2.0, t + 0.36, t + 0.37);
                batches.of(&palette.wax).cuboid(c, h, &[]);
            } else {
                let (c, h) = frame.boxed(a - 0.6, a + 0.6, 0.0, 0.85, t, t + 0.4);
                add(batches, &wood, c, h);
            }
        }
        RoomKind::Bedroom if choice < 0.5 => {
            let (c, h) = frame.boxed(-0.8, 0.8, 0.0, 1.5, t, t + 0.08);
            add(batches, &wood, c, h);
            let (c, h) = frame.boxed(-0.75, 0.75, 0.0, 0.62, t + 0.08, t + 1.7);
            add(batches, &linen, c, h);
        }
        RoomKind::Studio => {
            let (c, h) = frame.boxed(a - 0.7, a + 0.7, 0.0, 0.8, t, t + 0.6);
            add(batches, &wood, c, h);
            for i in 0..4 {
                let x = a - 0.5 + i as f32 * 0.3;
                let (c, h) = frame.boxed(
                    x,
                    x + 0.05,
                    0.8,
                    0.8 + rand.range(0.1, 0.25),
                    t + 0.2,
                    t + 0.25,
                );
                batches.of(&palette.brass).cuboid(c, h, &[]);
            }
        }
        _ => {
            if choice < 0.35 {
                let (c, h) = frame.boxed(a - 0.95, a + 0.95, 0.0, 0.5, t, t + 0.85);
                add(batches, &linen, c, h);
                let (c, h) = frame.boxed(a - 0.95, a + 0.95, 0.5, 0.95, t, t + 0.25);
                add(batches, &linen, c, h);
            } else if choice < 0.6 {
                let (c, h) = frame.boxed(a - 0.42, a + 0.42, 0.0, 0.5, t + 0.05, t + 0.85);
                add(batches, &linen, c, h);
                let (c, h) = frame.boxed(a - 0.42, a + 0.42, 0.5, 1.05, t + 0.05, t + 0.28);
                add(batches, &linen, c, h);
            } else if choice < 0.75 {
                let (c, h) = frame.boxed(a - 0.75, a + 0.75, 0.0, 1.3, t, t + 0.62);
                add(batches, &linen, c, h);
            } else {
                return None;
            }
        }
    }
    bounds.map(|(lo, hi)| ((lo + hi) * 0.5, (hi - lo) * 0.5))
}

fn fireplace(batches: &mut Batches, frame: &Frame) -> (Vec3, Vec3) {
    let t = WALL_THICKNESS * 0.5;
    let palette = batches.palette;
    let marble = palette.marble.clone();
    let soot = palette.soot.clone();
    for (a0, a1) in [(-0.8, -0.45), (0.45, 0.8)] {
        let (c, h) = frame.boxed(a0, a1, 0.0, 1.15, t, t + 0.3);
        batches.of(&marble).cuboid(c, h, &[Vec3::NEG_Y]);
    }
    let (c, h) = frame.boxed(-0.45, 0.45, 0.85, 1.15, t, t + 0.3);
    batches.of(&marble).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(-0.9, 0.9, 1.15, 1.22, t, t + 0.38);
    batches.of(&marble).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(-0.45, 0.45, 0.0, 0.85, t, t + 0.05);
    batches.of(&soot).cuboid(c, h, &[]);
    let (c, h) = frame.boxed(-0.9, 0.9, 0.0, 0.02, t, t + 0.6);
    batches.of(&marble).cuboid(c, h, &[]);
    frame.boxed(-0.8, 0.8, 0.0, 1.22, t, t + 0.3)
}

fn easel(
    cmd: &mut CommandQueue,
    batches: &mut Batches,
    at: Vec3,
    yaw: f32,
    picture: Option<Picture>,
) {
    let rot = Quat::from_rotation_y(yaw);
    let wood = batches.palette.wood.clone();
    let lean = Quat::from_rotation_x(-0.12);
    for side in [-0.32, 0.32] {
        let c = at + rot * Vec3::new(side, 0.95, 0.0);
        batches
            .of(&wood)
            .oriented_cuboid(c, Vec3::new(0.025, 0.95, 0.025), rot * lean);
    }
    let back = at + rot * Vec3::new(0.0, 0.85, -0.35);
    batches.of(&wood).oriented_cuboid(
        back,
        Vec3::new(0.025, 0.88, 0.025),
        rot * Quat::from_rotation_x(0.35),
    );
    let shelf = at + rot * Vec3::new(0.0, 0.8, 0.1);
    batches
        .of(&wood)
        .oriented_cuboid(shelf, Vec3::new(0.4, 0.02, 0.05), rot * lean);
    let canvas_center = at + rot * Vec3::new(0.0, 1.22, 0.14);
    let canvas_rot = rot * lean;
    let palette = batches.palette;
    batches.of(&palette.linen.clone()).oriented_cuboid(
        canvas_center - canvas_rot * Vec3::Z * 0.02,
        Vec3::new(0.38, 0.45, 0.015),
        canvas_rot,
    );
    if let Some(picture) = picture {
        cmd.spawn((
            HouseEntity,
            SyncWithRenderWorld,
            MeshComponent {
                handle: palette.canvas_mesh.clone(),
            },
            MaterialComponent {
                handle: palette.picture(picture).clone(),
            },
            Transform {
                translation: canvas_center,
                rotation: canvas_rot,
                scale: Vec3::new(0.72, 0.86, 1.0),
            },
        ));
    }
}

fn rug(cmd: &mut CommandQueue, palette: &Palette, center: Vec3, size: (f32, f32), which: usize) {
    cmd.spawn((
        HouseEntity,
        SyncWithRenderWorld,
        MeshComponent {
            handle: palette.canvas_mesh.clone(),
        },
        MaterialComponent {
            handle: palette.rugs[which % palette.rugs.len()].clone(),
        },
        Transform {
            translation: center + Vec3::Y * 0.006,
            rotation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(size.0, size.1, 1.0),
        },
    ));
}

pub struct BuiltHouse {
    pub ledger: Entity,
}

pub fn build_house(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    palette: &Palette,
    level: &Level,
    night: u32,
    seed: u64,
) -> BuiltHouse {
    let mut rand = Rand::new(seed ^ 0x5eed_4005);
    let mut batches = Batches::new(palette);
    let extent = level.extent();

    collider(
        cmd,
        Vec3::new(extent * 0.5, -0.5, extent * 0.5),
        Vec3::new(extent * 0.5 + 2.0, 0.5, extent * 0.5 + 2.0),
    );
    batches.of(&palette.floor).quad(
        Vec3::new(extent * 0.5, 0.0, extent * 0.5),
        Vec3::X * (extent * 0.5 + 1.0),
        Vec3::NEG_Z * (extent * 0.5 + 1.0),
        Vec3::Y,
    );
    batches.of(&palette.ceiling).quad(
        Vec3::new(extent * 0.5, CEILING, extent * 0.5),
        Vec3::X * (extent * 0.5 + 1.0),
        Vec3::Z * (extent * 0.5 + 1.0),
        Vec3::NEG_Y,
    );
    wall_colliders(cmd, level);

    for cell in level.cells() {
        for dir in Dir::ALL {
            let spot = WallSpot { cell, dir };
            if spot == level.exit {
                continue;
            }
            if level.is_wall(cell, dir) {
                build_wall_side(&mut batches, level, spot, false);
            } else if level.is_door(cell, dir) {
                build_wall_side(&mut batches, level, spot, true);
            }
        }
    }

    let mut used: Vec<WallSpot> = Vec::new();
    let mut titles: Vec<&'static str> = TITLES.to_vec();
    let mut lot_number = 3 + (seed % 40) as u32;
    for (i, &spot) in level.lots.iter().enumerate() {
        let frame = Frame::of(spot);
        let kind = level.room(spot.cell).map(|r| r.kind);
        let picture = if kind == Some(RoomKind::Bedroom) {
            Picture::Clara
        } else {
            [
                Picture::Sitter,
                Picture::Group,
                Picture::Sitter,
                Picture::StillLife,
                Picture::Landscape,
            ][i % 5]
        };
        let center = hang_picture(cmd, &mut batches, &frame, 0.0, 1.95, (0.9, 1.1), picture);
        let tag = spawn_prop(
            cmd,
            &palette.tag_mesh,
            &palette.tag,
            Transform::from_translation_rotation(
                frame.at(0.4, 1.28, WALL_THICKNESS * 0.5 + 0.08),
                frame.facing() * Quat::from_rotation_z(0.2),
            ),
        );
        let title = titles.swap_remove(rand.index(titles.len()));
        lot_number += 1 + rand.index(9) as u32;
        cmd.spawn((
            HouseEntity,
            Lot {
                number: lot_number,
                title,
                center,
                tag,
                done: false,
            },
        ));
        used.push(spot);
    }

    for &spot in &level.lights {
        let frame = Frame::of(spot);
        sconce(cmd, &mut batches, &frame, 1.25, true, &mut rand);
    }

    let hall = level.room_index(level.start);
    let mut fireplaces: Vec<usize> = Vec::new();
    let mut studio_easels = 0;
    for spot in level.wall_spots().collect::<Vec<_>>() {
        if used.contains(&spot) {
            continue;
        }
        let Some(room_index) = level.room_index(spot.cell) else {
            continue;
        };
        let kind = level.rooms[room_index].kind;
        let frame = Frame::of(spot);
        let lit = level.lights.contains(&spot);

        let wants_fire = matches!(
            kind,
            RoomKind::Parlour | RoomKind::Dining | RoomKind::Bedroom | RoomKind::Library
        ) && !fireplaces.contains(&room_index)
            && !lit;
        if wants_fire {
            fireplaces.push(room_index);
            let (c, h) = fireplace(&mut batches, &frame);
            collider(cmd, c, h);
            let picture = if kind == RoomKind::Bedroom {
                Picture::Clara
            } else {
                [Picture::Landscape, Picture::Seascape, Picture::Sitter][rand.index(3)]
            };
            hang_picture(cmd, &mut batches, &frame, 0.0, 2.25, (1.0, 0.75), picture);
            continue;
        }

        let outer = !level.contains(spot.cell.step(spot.dir));
        if outer && !lit && rand.unit() < 0.6 {
            window(cmd, &mut batches, &frame);
            if matches!(
                kind,
                RoomKind::Parlour | RoomKind::Dining | RoomKind::Gallery
            ) && rand.unit() < 0.4
                && let Some((c, h)) = furnish_wall(&mut batches, &frame, kind, &mut rand)
            {
                collider(cmd, c, h);
            }
            continue;
        }
        let picture_odds = match kind {
            RoomKind::Gallery => 0.95,
            RoomKind::Corridor | RoomKind::Hall => 0.55,
            RoomKind::Library => 0.0,
            RoomKind::Studio => 0.25,
            _ => 0.4,
        };
        let tall_furniture = kind == RoomKind::Library;
        if tall_furniture && !lit && rand.unit() < 0.8 {
            if let Some((c, h)) = furnish_wall(&mut batches, &frame, kind, &mut rand) {
                collider(cmd, c, h);
            }
            continue;
        }
        if rand.unit() < picture_odds {
            let pick = [
                Picture::Sitter,
                Picture::Landscape,
                Picture::Seascape,
                Picture::StillLife,
                Picture::Group,
            ];
            let size = if kind == RoomKind::Gallery {
                (rand.range(0.8, 1.4), rand.range(0.9, 1.3))
            } else {
                (rand.range(0.5, 0.9), rand.range(0.6, 0.9))
            };
            let a = if lit { -0.5 } else { rand.range(-0.4, 0.4) };
            hang_picture(
                cmd,
                &mut batches,
                &frame,
                a,
                2.0,
                size,
                pick[rand.index(pick.len())],
            );
        } else if !lit && rand.unit() < 0.25 {
            sconce(cmd, &mut batches, &frame, 1.25, false, &mut rand);
        }
        let furnish_odds = match kind {
            RoomKind::Hall => 0.3,
            RoomKind::Corridor => 0.25,
            RoomKind::Gallery => 0.15,
            _ => 0.55,
        };
        if Some(room_index) != hall
            && rand.unit() < furnish_odds
            && let Some((c, h)) = furnish_wall(&mut batches, &frame, kind, &mut rand)
        {
            collider(cmd, c, h);
        }
        if kind == RoomKind::Studio && studio_easels < 4 && rand.unit() < 0.6 {
            studio_easels += 1;
            let at = frame.at(rand.range(-1.3, 1.3), 0.0, 0.85);
            let yaw =
                frame.inward.x.atan2(frame.inward.z) + rand.range(-0.6, 0.6) + std::f32::consts::PI;
            let picture = [None, Some(Picture::Sitter), Some(Picture::Group)][rand.index(3)];
            easel(cmd, &mut batches, at, yaw, picture);
            collider(cmd, at + Vec3::Y * 0.9, Vec3::new(0.3, 0.9, 0.3));
        }
    }

    for (i, room) in level.rooms.iter().enumerate() {
        let min = Cell::new(room.x, room.y).center() - Vec3::new(CELL, 0.0, CELL) * 0.5;
        let size = Vec3::new(room.w as f32, 0.0, room.h as f32) * CELL;
        let middle = min + size * 0.5;
        match room.kind {
            RoomKind::Corridor => {
                let (w, h) = if room.w > room.h {
                    (size.x - 0.8, 1.1)
                } else {
                    (1.1, size.z - 0.8)
                };
                rug(cmd, palette, middle, (w, h), i);
            }
            RoomKind::Studio => {}
            _ => {
                rug(cmd, palette, middle, (size.x * 0.6, size.z * 0.6), i);
            }
        }
        if room.w >= 2 && room.h >= 2 {
            let vertex = Cell::new(room.x + room.w / 2, room.y + room.h / 2).center()
                - Vec3::new(CELL, 0.0, CELL) * 0.5;
            match room.kind {
                RoomKind::Dining => {
                    let half = Vec3::new(1.0, 0.38, 0.5);
                    batches.of(&palette.linen.clone()).cuboid(
                        vertex + Vec3::Y * 0.38,
                        half,
                        &[Vec3::NEG_Y],
                    );
                    collider(cmd, vertex + Vec3::Y * 0.38, half);
                    for (x, z) in [(-0.6, 0.8), (0.6, 0.8), (-0.6, -0.8), (0.6, -0.8)] {
                        let c = vertex + Vec3::new(x, 0.45, z);
                        batches.of(&palette.linen.clone()).cuboid(
                            c,
                            Vec3::new(0.22, 0.45, 0.22),
                            &[Vec3::NEG_Y],
                        );
                    }
                }
                RoomKind::Gallery | RoomKind::Parlour => {
                    let half = Vec3::new(0.8, 0.22, 0.35);
                    batches.of(&palette.linen.clone()).cuboid(
                        vertex + Vec3::Y * 0.22,
                        half,
                        &[Vec3::NEG_Y],
                    );
                    collider(cmd, vertex + Vec3::Y * 0.22, half);
                }
                RoomKind::Studio => {
                    let half = Vec3::new(0.7, 0.12, 0.7);
                    batches.of(&palette.wood.clone()).cuboid(
                        vertex + Vec3::Y * 0.12,
                        half,
                        &[Vec3::NEG_Y],
                    );
                    collider(cmd, vertex + Vec3::Y * 0.12, half);
                }
                _ => {}
            }
        }
    }

    let door = Frame::of(level.exit);
    let t = WALL_THICKNESS * 0.5;
    let paper = palette.wallpaper(RoomKind::Hall).clone();
    let d = DOOR_WIDTH * 0.5;
    let half = CELL * 0.5;
    wall_face(&mut batches, &door, &paper, -half, -0.7, CEILING);
    wall_face(&mut batches, &door, &paper, 0.7, half, CEILING);
    let (c, h) = door.boxed(-0.7, 0.7, DOOR_HEIGHT + 0.5, CEILING, 0.0, t);
    batches.of(&paper).cuboid(c, h, &[]);
    cornice(&mut batches, &door, -half, half);
    let (c, h) = door.boxed(-0.62, 0.62, 0.0, DOOR_HEIGHT, -0.05, 0.05);
    batches.of(&palette.wood.clone()).cuboid(c, h, &[]);
    collider(cmd, c, h);
    for side in [-1.0, 1.0] {
        let (c, h) = door.boxed(
            side * 0.7 - 0.08,
            side * 0.7 + 0.08,
            0.0,
            DOOR_HEIGHT + 0.5,
            0.0,
            t + 0.04,
        );
        batches
            .of(&palette.wood.clone())
            .cuboid(c, h, &[Vec3::NEG_Y]);
    }
    let (c, h) = door.boxed(-0.3, 0.3, 1.0, 1.02, 0.05, 0.07);
    batches.of(&palette.brass.clone()).cuboid(c, h, &[]);
    let _ = d;
    cmd.spawn((
        HouseEntity,
        SyncWithRenderWorld,
        MeshComponent {
            handle: palette.canvas_mesh.clone(),
        },
        MaterialComponent {
            handle: palette.moonlight.clone(),
        },
        Transform {
            translation: door.at(0.0, DOOR_HEIGHT + 0.25, 0.0),
            rotation: door.facing(),
            scale: Vec3::new(1.3, 0.45, 1.0),
        },
    ));
    cmd.spawn((
        HouseEntity,
        LightSource,
        SyncWithRenderWorld,
        Light::point_light()
            .with_color(Color::srgba(0.45, 0.55, 0.8, 1.0))
            .with_intensity(1.4)
            .with_range(4.0),
        Transform::from_translation(door.at(0.0, DOOR_HEIGHT + 0.2, 0.4)),
    ));

    let desk_a = 1.25;
    let (c, h) = door.boxed(desk_a - 0.5, desk_a + 0.5, 0.0, 0.8, t, t + 0.55);
    batches
        .of(&palette.wood.clone())
        .cuboid(c, h, &[Vec3::NEG_Y]);
    collider(cmd, c, h);
    let ledger_pos = door.at(desk_a, 0.83, t + 0.3);
    let book = spawn_prop(
        cmd,
        &palette.page_mesh,
        &palette.ledger,
        Transform {
            translation: ledger_pos,
            rotation: door.facing(),
            scale: Vec3::new(1.6, 4.0, 1.4),
        },
    );
    let ledger = cmd
        .spawn((
            HouseEntity,
            Ledger {
                point: ledger_pos,
                book,
            },
            Transform::from_translation(ledger_pos),
        ))
        .entity();

    for &cell in &level.oil {
        let jitter = Vec3::new(rand.range(-1.0, 1.0), 0.0, rand.range(-1.0, 1.0));
        let base = cell.center() + jitter;
        let tin = spawn_prop(
            cmd,
            &palette.oil_mesh,
            &palette.oil,
            Transform::from_translation(base),
        );
        cmd.insert(OilPickup { base }, tin);
    }
    if let Some(cell) = level.page {
        let jitter = Vec3::new(rand.range(-0.8, 0.8), 0.0, rand.range(-0.8, 0.8));
        let base = cell.center() + jitter + Vec3::Y * 0.01;
        let page = spawn_prop(
            cmd,
            &palette.page_mesh,
            &palette.page,
            Transform::from_translation_rotation(base, Quat::from_rotation_y(rand.range(0.0, 6.0))),
        );
        cmd.insert(PagePickup { base }, page);
    }
    let _ = night;

    batches.spawn(cmd, server);
    BuiltHouse { ledger }
}

/// What a room is called on screen.
pub fn room_name(kind: RoomKind) -> &'static str {
    match kind {
        RoomKind::Hall => "The Hall",
        RoomKind::Corridor => "A Passage",
        RoomKind::Studio => "The Studio",
        RoomKind::Gallery => "The Long Gallery",
        RoomKind::Parlour => "The Parlour",
        RoomKind::Library => "The Library",
        RoomKind::Dining => "The Dining Room",
        RoomKind::Bedroom => "Clara's Room",
    }
}
