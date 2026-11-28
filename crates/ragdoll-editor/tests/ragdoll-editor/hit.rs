use std::path::Path;

use concerto_animation::{
    clip::AnimationClip,
    graph::AnimationGraph,
    message::MessageType,
    node::{AnimationClipNode, AnimationNode, SlotEntry, SlotNode},
    player::AnimationHandleComponent,
};
use concerto_app::{App, schedule_groups::Startup};
use concerto_ecs::{
    Entity, Res,
    command::CommandQueue,
    component::{name::Name as EntityName, scene::SceneEntityRef},
};
use concerto_foundation::{
    assets::{
        asset_server::AssetServer, content::save_content_asset_into, handle::AssetHandle,
        registry::AssetRegistry,
    },
    transform::Transform,
};
use concerto_mcp::host::run_with;
use concerto_mesh::SkeletonComponent;
use concerto_name::name;
use concerto_physics::{
    collider::Collider,
    layers::CollisionLayers,
    rigid_body::{AllowedDofs, RigidBody},
};
use concerto_ragdoll::{
    CharacterRagdoll, FlinchNode, RagdollMode,
    messages::{Hit, Recovered},
    testing::{humanoid_skeleton, reference_humanoid},
};
use concerto_render::gpu::Gpu;
use concerto_scene::{
    node_id::NodeId,
    scene::{Scene, SceneNode},
};
use glam::Vec3;
use rmcp::ServiceExt;
use serde_json::json;

use crate::create::{Client, SIZE, call, game, ok, text, until_idle};

const SCENE: &str = "content/scene.gasset";

fn write_scene(root: &Path) {
    let mut node = SceneNode {
        id: NodeId::from_name_path("prop"),
        name: "prop".into(),
        children: vec![],
        components: vec![],
        link: None,
    };
    node.push_component(&Transform::IDENTITY).unwrap();
    let scene = Scene {
        nodes: vec![node],
        referenced_assets: vec![],
    };
    save_content_asset_into(&scene, root, SCENE, &mut AssetRegistry::new()).unwrap();
}

struct Casting {
    reacting: AssetHandle<AnimationGraph>,
    inert: AssetHandle<AnimationGraph>,
}

fn over_clip(node: impl AnimationNode, server: &AssetServer) -> AnimationGraph {
    let clip = server.add(AnimationClip::default());
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(node)
        .input(AnimationClipNode::new(clip));
    graph
}

fn spawn_character(
    cmd: &mut CommandQueue,
    server: &AssetServer,
    label: &str,
    x: f32,
    graph: AssetHandle<AnimationGraph>,
) {
    let reference = reference_humanoid();
    let skeleton = humanoid_skeleton();
    let skeleton_handle = server.add(reference.skeleton);
    let mut ragdoll = reference.ragdoll;
    ragdoll.skeleton = skeleton_handle.clone();
    let ragdoll = server.add(ragdoll);
    let root = Vec3::new(x, 0.02, 0.0);
    let capsule = cmd
        .spawn((
            RigidBody {
                allowed_dofs: AllowedDofs::TRANSLATION,
                ..RigidBody::default()
            },
            Collider::capsule(0.5, 0.3),
            CollisionLayers::character(),
            Transform::from_translation(root + Vec3::Y * 0.85),
        ))
        .entity();
    let character = cmd
        .spawn((EntityName::new(label), Transform::from_translation(root)))
        .entity();
    let bones: Vec<Entity> = skeleton
        .rest_pose
        .iter()
        .map(|rest| {
            cmd.spawn(Transform::from_translation_rotation(
                rest.translation,
                rest.rotation,
            ))
            .entity()
        })
        .collect();
    for (bone, parent) in skeleton.parents.iter().enumerate() {
        let parent = parent.map_or(character, |parent| bones[usize::from(parent)]);
        cmd.add_child(parent, bones[bone]);
    }
    cmd.insert(
        (
            SkeletonComponent::new(
                skeleton_handle,
                bones.iter().copied().map(SceneEntityRef::Entity).collect(),
                bones.first().copied().map(SceneEntityRef::Entity),
            ),
            CharacterRagdoll::new(ragdoll)
                .with_capsule(capsule)
                .with_mode(RagdollMode::Graph),
            AnimationHandleComponent::new(graph),
        ),
        character,
    );
}

fn spawn_characters(server: Res<AssetServer>, mut cmd: CommandQueue) {
    let casting = Casting {
        reacting: server.add(over_clip(
            SlotNode::new().with_entry(
                SlotEntry::new(name!("flinch"), FlinchNode::default())
                    .with_on(MessageType::of::<Hit>())
                    .with_out(MessageType::of::<Recovered>()),
            ),
            &server,
        )),
        inert: server.add({
            let clip = server.add(AnimationClip::default());
            AnimationGraph::from_node(AnimationClipNode::new(clip))
        }),
    };
    cmd.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    spawn_character(&mut cmd, &server, "Reacting", 0.0, casting.reacting);
    spawn_character(&mut cmd, &server, "Inert", 4.0, casting.inert);
}

fn hitting_game(app: &mut App) {
    game(app);
    app.add_system(Startup, spawn_characters);
}

fn id_of(tree: &str, label: &str) -> String {
    tree.lines()
        .find(|line| line.contains(label))
        .and_then(|line| {
            line.split(|c: char| !c.is_alphanumeric())
                .find(|word| word.contains('v') && word.starts_with(|c: char| c.is_ascii_digit()))
        })
        .unwrap_or_else(|| panic!("no {label} in {tree}"))
        .to_owned()
}

struct Hits {
    reacted: String,
    inert: String,
    report: String,
    mid_flinch: Option<String>,
}

fn amounts(report: &str) -> Vec<f32> {
    report
        .lines()
        .filter_map(|line| {
            line.split_once(" · physics ")?
                .1
                .split(' ')
                .next()?
                .parse()
                .ok()
        })
        .collect()
}

async fn report_while_reacting(client: &Client, entity: &str) -> Option<String> {
    for _ in 0..400 {
        let report = ok(client, "body_report", json!({ "entity": entity })).await;
        if amounts(&report).iter().any(|amount| *amount > 0.0) {
            return Some(report);
        }
    }
    None
}

async fn hit_characters(client: Client) -> Hits {
    until_idle(&client).await;
    ok(&client, "open_asset", json!({ "asset": SCENE })).await;
    until_idle(&client).await;
    ok(&client, "play", json!({})).await;
    let tree = ok(&client, "scene_tree", json!({})).await;
    let reacting = id_of(&tree, "Reacting");
    let inert = id_of(&tree, "Inert");
    let hit = |entity: String| {
        let client = &client;
        async move {
            for _ in 0..200 {
                let result = call(
                    client,
                    "hit_character",
                    json!({ "entity": entity, "bone": "spine_02", "impulse": [0.0, 0.0, -60.0] }),
                )
                .await;
                if result.is_error != Some(true) {
                    return text(&result);
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("{entity} never became hittable");
        }
    };
    let reacted = hit(reacting.clone()).await;
    let inert = hit(inert).await;
    let report = ok(&client, "body_report", json!({ "entity": reacting })).await;
    let (_, mid_flinch) = tokio::join!(
        hit(reacting.clone()),
        report_while_reacting(&client, &reacting)
    );
    client.cancel().await.unwrap();
    Hits {
        reacted,
        inert,
        report,
        mid_flinch,
    }
}

#[test]
fn hit_character_waits_for_the_body_to_return_and_body_report_shows_each_limb() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    write_scene(&root);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                Some(hit_characters(client).await)
            })
    });
    run_with(
        server_io,
        gpu,
        concerto_editor::mcp::editor(
            hitting_game,
            root.clone(),
            vec![concerto_ragdoll_editor::install],
        ),
        SIZE,
    );
    let hits = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");

    let peak: f32 = hits
        .reacted
        .split("the part got ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("no peak distance in {}", hits.reacted));
    assert!(peak > 0.0, "{}", hits.reacted);
    assert!(
        hits.reacted.contains("back on its animation after"),
        "{}",
        hits.reacted
    );
    assert!(
        hits.inert.contains("the graph did not react"),
        "{}",
        hits.inert
    );
    assert!(
        hits.report
            .lines()
            .any(|line| line.starts_with("spine_02 · physics ") && line.contains(" · cover ")),
        "{}",
        hits.report
    );
    assert_eq!(
        amounts(&hits.report).len(),
        reference_humanoid().ragdoll.parts.len(),
        "{}",
        hits.report
    );
    assert!(hits.report.contains("graph states: "), "{}", hits.report);
    assert!(
        hits.mid_flinch.is_some(),
        "no limb was drawn from physics while the character flinched"
    );
}
