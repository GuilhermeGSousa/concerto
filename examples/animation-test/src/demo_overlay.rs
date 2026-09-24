//! On-screen debug overlay (load state + current FSM state) and one-shot debug-gizmo
//! markers for the important entities (spawner, skinned mesh, root bone).

use concerto::{
    animation::{player::AnimationPlayer, root::AnimationRootBone},
    ecs::{command::CommandQueue, component::Component, query::Query, With},
    foundation::transform::GlobalTransform,
    mesh::skeleton::SkeletonComponent,
    scene::spawner::SceneSpawnerComponent,
    ui::{
        node::{UINode, UIRect},
        text::{FontFamily, TextComponent},
        transform::UIValue,
    },
};
use concerto_color::Color;
use concerto_debug_gizmos::DebugGizmos;

use crate::movement_animation::AnimatedCharacter;

/// Marks the overlay text node so `update_overlay` can find and rewrite it.
#[derive(Component)]
pub(crate) struct OverlayText;

/// Startup: spawn the top-left overlay text node.
pub(crate) fn spawn_overlay(mut cmd: CommandQueue) {
    cmd.spawn((
        UINode {
            // Explicit size: taffy does not measure text, so an Auto-sized node would
            // collapse to 0x0 and clip the glyphs.
            width: UIValue::Px(460.0),
            height: UIValue::Px(64.0),
            margin: UIRect {
                top: 12.0,
                left: 12.0,
                ..Default::default()
            },
            ..Default::default()
        },
        TextComponent {
            text: "Load: starting…\nState: —".to_string(),
            font_size: 16.0,
            line_height: 22.0,
            font_family: FontFamily::Monospace,
            ..Default::default()
        },
        OverlayText,
    ));
}

/// Update: reflect the character's load progress in the overlay.
pub(crate) fn update_overlay(
    texts: Query<&mut TextComponent, With<OverlayText>>,
    spawners: Query<&SceneSpawnerComponent, With<AnimatedCharacter>>,
    players: Query<&AnimationPlayer>,
) {
    let load = if players.iter().next().is_some() {
        "Ready"
    } else if spawners.iter().next().is_some() {
        "Loading model…"
    } else {
        "Setting up animations…"
    };

    let new_text = format!("Load: {load}");
    for mut text in texts.iter() {
        if text.text != new_text {
            text.text = new_text.clone();
        }
    }
}

/// Update (every frame): draw a coloured wireframe marker on the spawner, the
/// skinned-mesh entity, and the root bone. Because gizmos are immediate mode the
/// markers are re-drawn each frame and therefore track the entities as they move.
/// These are distinct entities (the skinned-mesh node is not the root joint), so
/// the three markers land apart.
pub(crate) fn draw_entity_gizmos(
    mut gizmos: DebugGizmos,
    character: Query<&GlobalTransform, With<AnimatedCharacter>>,
    skinned_mesh: Query<&GlobalTransform, With<SkeletonComponent>>,
    root_bone: Query<&GlobalTransform, With<AnimationRootBone>>,
) {
    let (Some(spawner), Some(mesh), Some(root)) = (
        character.iter().next(),
        skinned_mesh.iter().next(),
        root_bone.iter().next(),
    ) else {
        // Not fully loaded yet (root bone transform not propagated) — try again next frame.
        return;
    };

    gizmos.sphere(spawner.translation(), 0.15, Color::rgba(1.0, 0.2, 0.2, 1.0));
    gizmos.sphere(mesh.translation(), 0.15, Color::rgba(0.2, 1.0, 0.2, 1.0));
    gizmos.sphere(root.translation(), 0.12, Color::rgba(0.3, 0.5, 1.0, 1.0));
}
