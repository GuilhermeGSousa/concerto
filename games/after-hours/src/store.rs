//! Builds a night's store floor from a [`Level`]: merged static meshes for
//! the floor, ceiling, walls and stocked shelving, one collider per wall
//! segment, ceiling fixtures, pickups and the staff exit.
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
        components::light::Light,
    },
};
use glam::{Quat, Vec3};

use crate::{
    game::Rand,
    level::{CELL, Level},
    meshes::{self, MeshBuilder},
    palette::Palette,
};

pub const CEILING: f32 = 3.4;
pub const SHELF_HEIGHT: f32 = 2.4;
pub const WALL_THICKNESS: f32 = 0.6;
const OUTER_THICKNESS: f32 = 0.3;

/// Everything spawned for the current night; despawned wholesale on rebuild.
#[derive(Component)]
pub struct StoreEntity;

#[derive(Component)]
pub struct CeilingLight {
    pub base_intensity: f32,
    /// Flickering tubes stutter now and then.
    pub flickers: bool,
    pub on: bool,
    pub timer: f32,
    /// Stuttering toward going out for good.
    pub dying: bool,
    pub death_flickers: u32,
    pub dead: bool,
    /// The glowing panel mesh, swapped dark while the tube is off.
    pub panel: Entity,
}

/// Radius of floor a working ceiling light illuminates well enough to see by.
pub const LIGHT_POOL: f32 = 3.2;

#[derive(Component)]
pub struct KeyPickup {
    pub base: Vec3,
}

#[derive(Component)]
pub struct BatteryPickup {
    pub base: Vec3,
}

#[derive(Component)]
pub struct ExitDoor {
    /// Point just inside the door the player must reach.
    pub threshold: Vec3,
    pub sign: Entity,
}

fn spawn_mesh(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    builder: MeshBuilder,
    material: &AssetHandle<StandardMaterial>,
) {
    if builder.is_empty() {
        return;
    }
    cmd.spawn((
        StoreEntity,
        MeshComponent {
            handle: server.add(builder.build()),
        },
        MaterialComponent {
            handle: material.clone(),
        },
        Transform::IDENTITY,
    ));
}

fn spawn_prop(
    cmd: &mut CommandQueue,
    mesh: &AssetHandle<Mesh>,
    material: &AssetHandle<StandardMaterial>,
    transform: Transform,
) -> Entity {
    cmd.spawn((
        StoreEntity,
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

/// Stocks one side of a shelving unit: boxes of varied size on each shelf.
fn stock_shelf(
    boxes: &mut [MeshBuilder],
    rand: &mut Rand,
    center: Vec3,
    along_x: bool,
    side: f32,
    shelf_heights: &[f32],
) {
    let length = CELL - 0.3;
    let depth = WALL_THICKNESS * 0.5 - 0.06;
    for &h in shelf_heights {
        let mut t = -length * 0.5;
        while t < length * 0.5 - 0.2 {
            let w = rand.range(0.18, 0.5).min(length * 0.5 - t);
            if rand.unit() < 0.18 {
                // A gap where stock was sold.
                t += w + 0.05;
                continue;
            }
            let height = rand.range(0.15, 0.42);
            let d = rand.range(0.12, depth);
            let along = t + w * 0.5;
            let out = side * (0.05 + d * 0.5);
            let offset = if along_x {
                Vec3::new(along, h + height * 0.5, out)
            } else {
                Vec3::new(out, h + height * 0.5, along)
            };
            let half = if along_x {
                Vec3::new(w * 0.5 - 0.01, height * 0.5, d * 0.5)
            } else {
                Vec3::new(d * 0.5, height * 0.5, w * 0.5 - 0.01)
            };
            let which = rand.index(boxes.len());
            boxes[which].cuboid(center + offset, half, &[Vec3::NEG_Y]);
            t += w + 0.02;
        }
    }
}

pub struct BuiltStore {
    pub exit: Entity,
}

pub fn build_store(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    palette: &Palette,
    level: &Level,
    seed: u64,
) -> BuiltStore {
    let mut rand = Rand::new(seed ^ 0x5eed_5707);
    let extent = level.extent();

    // Floor and ceiling slabs.
    cmd.spawn((
        StoreEntity,
        Collider::cuboid(extent * 0.5 + 2.0, 0.5, extent * 0.5 + 2.0),
        Transform::from_translation(Vec3::new(extent * 0.5, -0.5, extent * 0.5)),
    ));
    let mut floor = MeshBuilder::default();
    floor.quad(
        Vec3::new(extent * 0.5, 0.0, extent * 0.5),
        Vec3::X * (extent * 0.5 + 1.0),
        Vec3::NEG_Z * (extent * 0.5 + 1.0),
        Vec3::Y,
    );
    spawn_mesh(cmd, server, floor, &palette.floor);
    let mut ceiling = MeshBuilder::default();
    ceiling.quad(
        Vec3::new(extent * 0.5, CEILING, extent * 0.5),
        Vec3::X * (extent * 0.5 + 1.0),
        Vec3::Z * (extent * 0.5 + 1.0),
        Vec3::NEG_Y,
    );
    spawn_mesh(cmd, server, ceiling, &palette.ceiling);

    // Walls.
    let mut drywall = MeshBuilder::default();
    let mut steel = MeshBuilder::default();
    let mut boxes: Vec<MeshBuilder> = (0..palette.stock.len())
        .map(|_| MeshBuilder::default())
        .collect();
    let shelf_heights = [0.12, 0.62, 1.12, 1.62, 2.12];
    for wall in level.walls() {
        if wall.outer {
            let half = wall.half_extents(OUTER_THICKNESS);
            let half = Vec3::new(half.x, CEILING * 0.5, half.y);
            let center = wall.center + Vec3::Y * CEILING * 0.5;
            cmd.spawn((
                StoreEntity,
                Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(center),
            ));
            drywall.cuboid(center, half, &[Vec3::NEG_Y, Vec3::Y]);
            continue;
        }

        let half = wall.half_extents(WALL_THICKNESS);
        let center = wall.center + Vec3::Y * SHELF_HEIGHT * 0.5;
        cmd.spawn((
            StoreEntity,
            Collider::cuboid(half.x, SHELF_HEIGHT * 0.5, half.y),
            Transform::from_translation(center),
        ));
        // The unit: a solid back panel, end uprights, shelves on both faces,
        // and stock. Shelves stop short of the ends so units read as units.
        let base = wall.center;
        let (long, thick) = if wall.along_x {
            (Vec3::X, Vec3::Z)
        } else {
            (Vec3::Z, Vec3::X)
        };
        let len = CELL * 0.5 - 0.1;
        steel.cuboid(
            base + Vec3::Y * SHELF_HEIGHT * 0.5,
            long * len + thick * 0.04 + Vec3::Y * SHELF_HEIGHT * 0.5,
            &[Vec3::NEG_Y],
        );
        for end in [-1.0, 1.0] {
            steel.cuboid(
                base + long * end * (len + 0.05) + Vec3::Y * SHELF_HEIGHT * 0.5,
                long * 0.05 + thick * (WALL_THICKNESS * 0.5) + Vec3::Y * SHELF_HEIGHT * 0.5,
                &[Vec3::NEG_Y],
            );
        }
        for &h in &shelf_heights {
            steel.cuboid(
                base + Vec3::Y * h,
                long * len + thick * (WALL_THICKNESS * 0.5) + Vec3::Y * 0.015,
                &[],
            );
        }
        for side in [-1.0, 1.0] {
            if rand.unit() < 0.85 {
                stock_shelf(
                    &mut boxes,
                    &mut rand,
                    base,
                    wall.along_x,
                    side,
                    &shelf_heights,
                );
            }
        }
    }
    spawn_mesh(cmd, server, drywall, &palette.drywall);
    spawn_mesh(cmd, server, steel, &palette.steel);
    for (builder, material) in boxes.into_iter().zip(palette.stock.iter()) {
        spawn_mesh(cmd, server, builder, material);
    }

    // Ceiling fixtures: a panel in every cell, lit or dead.
    let panel_mesh = &palette.panel_mesh;
    for cell in level.cells() {
        let working = level.lights.contains(&cell);
        let pos = cell.center() + Vec3::Y * (CEILING - 0.04);
        let panel = spawn_prop(
            cmd,
            panel_mesh,
            if working {
                &palette.panel_on
            } else {
                &palette.panel_off
            },
            Transform::from_translation(pos),
        );
        if working {
            let flickers = rand.unit() < 0.3 && cell != level.start;
            let intensity = 11.0;
            cmd.spawn((
                StoreEntity,
                CeilingLight {
                    base_intensity: intensity,
                    flickers,
                    on: true,
                    timer: rand.range(0.5, 6.0),
                    dying: false,
                    death_flickers: 0,
                    dead: false,
                    panel,
                },
                Light::point_light()
                    .with_color(Color::srgba(0.85, 0.95, 1.0, 1.0))
                    .with_intensity(intensity),
                Transform::from_translation(pos - Vec3::Y * 0.3),
            ));
        }
    }

    // Pickups.
    for &cell in &level.keys {
        let base = cell.center() + Vec3::Y * 1.0;
        let key = spawn_prop(
            cmd,
            &palette.key_mesh,
            &palette.key,
            Transform::from_translation(base),
        );
        cmd.insert(KeyPickup { base }, key);
        let glow = cmd
            .spawn((
                Light::point_light()
                    .with_color(Color::srgba(1.0, 0.8, 0.35, 1.0))
                    .with_intensity(1.2),
                Transform::IDENTITY,
            ))
            .entity();
        cmd.add_child(key, glow);
    }
    for &cell in &level.batteries {
        // Tucked toward a corner so they reward looking around.
        let jitter = Vec3::new(rand.range(-1.0, 1.0), 0.0, rand.range(-1.0, 1.0));
        let base = cell.center() + jitter + Vec3::Y * 0.9;
        let battery = spawn_prop(
            cmd,
            &palette.battery_mesh,
            &palette.battery,
            Transform::from_translation(base),
        );
        cmd.insert(BatteryPickup { base }, battery);
    }

    // The staff exit: a door in the outer wall with a sign over it.
    let (exit_cell, dir) = level.exit;
    let door_center = exit_cell.center() + dir.vector() * CELL * 0.5;
    let along_x = dir.vector().x == 0.0;
    let rotation = if along_x {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
    };
    // Wall either side of and above the door.
    let side_half = Vec3::new(0.8, CEILING * 0.5, OUTER_THICKNESS * 0.5);
    let lintel_half = Vec3::new(0.62, (CEILING - 2.3) * 0.5, OUTER_THICKNESS * 0.5);
    let mut exit_wall = MeshBuilder::default();
    for (offset, half) in [
        (Vec3::new(-1.2, CEILING * 0.5, 0.0), side_half),
        (Vec3::new(1.2, CEILING * 0.5, 0.0), side_half),
        (
            Vec3::new(0.0, 2.3 + (CEILING - 2.3) * 0.5, 0.0),
            lintel_half,
        ),
    ] {
        let world_center = door_center + rotation * offset;
        let world_half = (rotation * half).abs();
        cmd.spawn((
            StoreEntity,
            Collider::cuboid(world_half.x, world_half.y, world_half.z),
            Transform::from_translation(world_center),
        ));
        exit_wall.cuboid(world_center, world_half, &[Vec3::NEG_Y]);
    }
    spawn_mesh(cmd, server, exit_wall, &palette.drywall);
    let door_half = (rotation * Vec3::new(0.6, 1.15, 0.05)).abs();
    let door_pos = door_center + Vec3::Y * 1.15 + dir.vector() * 0.1;
    cmd.spawn((
        StoreEntity,
        Collider::cuboid(door_half.x, door_half.y, door_half.z),
        MeshComponent {
            handle: server.add(meshes::cuboid(door_half)),
        },
        MaterialComponent {
            handle: palette.door.clone(),
        },
        Transform::from_translation(door_pos),
    ));
    let sign = spawn_prop(
        cmd,
        &palette.sign_mesh,
        &palette.sign_locked,
        Transform::from_translation_rotation(
            door_center + Vec3::Y * 2.65 - dir.vector() * 0.2,
            rotation,
        ),
    );
    let exit = cmd
        .spawn((
            StoreEntity,
            ExitDoor {
                threshold: door_center - dir.vector() * 0.9,
                sign,
            },
            Light::point_light()
                .with_color(Color::srgba(1.0, 0.2, 0.15, 1.0))
                .with_intensity(2.5),
            Transform::from_translation(door_center + Vec3::Y * 2.3 - dir.vector() * 0.6),
        ))
        .entity();

    BuiltStore { exit }
}
