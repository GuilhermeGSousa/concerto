use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use concerto_app::App;
use concerto_ecs::Res;
use concerto_editor::hosted::EditWorld;
use concerto_foundation::assets::{
    AssetId,
    asset_store::AssetStore,
    content::{decode_content_asset, read_content_asset},
    handle::AssetHandle,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, host::run_with};
use concerto_render::{
    assets::texture::{Compression, MaxSize, Texture, TextureRole, TextureSettings},
    gpu::Gpu,
};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const SIZE: (u32, u32) = (800, 600);
const PATIENCE: Duration = Duration::from_secs(120);
const TEXTURE: &str = "content/crate/texture_0.gasset";

type Client = RunningService<RoleClient, ()>;

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
}

#[derive(Deserialize, JsonSchema)]
struct TextureFormat {
    texture: String,
}

fn texture_format(
    mut calls: ToolCalls<TextureFormat>,
    textures: EditWorld<Option<Res<AssetStore<Texture>>>>,
) {
    for (arguments, reply) in calls.read() {
        let format = AssetId::from_simple_hex(&arguments.texture)
            .ok()
            .and_then(|id| textures.as_deref()?.get(&AssetHandle::weak(id)))
            .and_then(Texture::levels)
            .map_or_else(
                || "not loaded".to_owned(),
                |levels| format!("{:?}", levels.format),
            );
        reply.ok(format);
    }
}

fn texture_tools(app: &mut App) {
    app.add_tool::<TextureFormat, _>(
        McpTool::read_only("texture_format").needs(Needs::Scene),
        texture_format,
    );
}

async fn call(client: &Client, tool: &'static str, arguments: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool)
                .with_arguments(arguments.as_object().cloned().unwrap()),
        )
        .await
        .unwrap()
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

async fn ok(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    text(&result)
}

async fn until(
    client: &Client,
    tool: &'static str,
    arguments: Value,
    done: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    loop {
        let answer = ok(client, tool, arguments.clone()).await;
        if done(&answer) {
            return answer;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "{tool} still answers {answer}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn until_idle(client: &Client) -> String {
    until(client, "status", json!({}), |status| {
        !status.contains("project: none") && status.ends_with("busy: no")
    })
    .await
}

fn texture_line(status: &str) -> String {
    status
        .lines()
        .find(|line| line.contains(&format!("Texture {TEXTURE}")))
        .unwrap_or_else(|| panic!("no texture tab in {status}"))
        .to_owned()
}

fn write_png(path: &Path, image: image::RgbaImage) -> Vec<u8> {
    image.save(path).unwrap();
    std::fs::read(path).unwrap()
}

fn write_crate_gltf(dir: &Path) -> PathBuf {
    let mut buffer = Vec::new();
    for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        buffer.extend(value.to_le_bytes());
    }
    for index in [0u16, 1, 2, 0] {
        buffer.extend(index.to_le_bytes());
    }
    for value in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0] {
        buffer.extend(value.to_le_bytes());
    }
    std::fs::write(dir.join("crate.bin"), &buffer).unwrap();
    let gltf = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "name": "Crate", "mesh": 0 }],
        "meshes": [{ "primitives": [{
            "attributes": { "POSITION": 0, "TEXCOORD_0": 2 },
            "indices": 1,
            "material": 0
        }] }],
        "materials": [{
            "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } },
            "normalTexture": { "index": 1 }
        }],
        "images": [{ "uri": "colour.png" }, { "uri": "normal.png" }],
        "textures": [{ "source": 0 }, { "source": 1 }],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "max": [1.0, 1.0, 0.0], "min": [0.0, 0.0, 0.0] },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" },
            { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 6 },
            { "buffer": 0, "byteOffset": 44, "byteLength": 24 }
        ],
        "buffers": [{ "byteLength": buffer.len(), "uri": "crate.bin" }]
    });
    std::fs::write(dir.join("crate.gltf"), gltf.to_string()).unwrap();
    dir.join("crate.gltf")
}

struct Edited {
    imported: String,
    opened: String,
    derived: String,
    role: CallToolResult,
    refused: CallToolResult,
    saved: String,
    reloaded: String,
}

async fn edit_a_texture(client: Client, project: &Path, gltf: String) -> Edited {
    until_idle(&client).await;
    let source = json!({ "source": gltf, "destination": "assets/crate.gltf" });
    let imported = ok(&client, "import_source", source.clone()).await;
    let texture = read_content_asset(&std::fs::read(project.join(TEXTURE)).unwrap())
        .unwrap()
        .0
        .asset_id
        .simple_hex();
    ok(
        &client,
        "create_asset",
        json!({ "kind": "Scene", "name": "Level" }),
    )
    .await;
    ok(
        &client,
        "open_asset",
        json!({ "asset": "content/Level.gasset" }),
    )
    .await;
    until_idle(&client).await;
    ok(
        &client,
        "place_asset",
        json!({ "asset": "content/crate/scene.gasset" }),
    )
    .await;
    ok(&client, "save", json!({})).await;
    until(
        &client,
        "texture_format",
        json!({ "texture": texture }),
        |format| format == "Bc1RgbaUnormSrgb",
    )
    .await;

    ok(&client, "open_asset", json!({ "asset": TEXTURE })).await;
    let opened = texture_line(&until_idle(&client).await);
    for (path, value) in [
        ("settings.compression", "Bc7"),
        ("settings.max_size", "S512"),
    ] {
        ok(
            &client,
            "set_property",
            json!({ "target": "Texture", "path": path, "value": value }),
        )
        .await;
    }
    let derived = texture_line(&until_idle(&client).await);
    let role = call(
        &client,
        "set_property",
        json!({ "target": "Texture", "path": "role", "value": "Normal" }),
    )
    .await;
    let compression = |value: &str| json!({ "target": "Texture", "path": "settings.compression", "value": value });
    ok(&client, "set_property", compression("Bc5")).await;
    let refused = call(&client, "save", json!({})).await;
    ok(&client, "set_property", compression("Bc7")).await;
    let saved = ok(&client, "save", json!({})).await;

    ok(
        &client,
        "open_asset",
        json!({ "asset": "content/Level.gasset" }),
    )
    .await;
    let reloaded = until(
        &client,
        "texture_format",
        json!({ "texture": texture }),
        |format| format == "Bc7RgbaUnormSrgb",
    )
    .await;
    ok(&client, "import_source", source).await;
    until_idle(&client).await;
    client.cancel().await.unwrap();
    Edited {
        imported,
        opened,
        derived,
        role,
        refused,
        saved,
        reloaded,
    }
}

#[test]
fn a_texture_tab_changes_compression_saves_and_survives_reimport() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    std::fs::create_dir(root.join("content")).unwrap();
    let sources = tempfile::tempdir().unwrap();
    let colour = write_png(
        &sources.path().join("colour.png"),
        image::RgbaImage::from_fn(1024, 1024, |x, y| {
            image::Rgba([(x / 4) as u8, (y / 4) as u8, 96, 255])
        }),
    );
    write_png(
        &sources.path().join("normal.png"),
        image::RgbaImage::from_pixel(64, 64, image::Rgba([128, 128, 255, 255])),
    );
    let gltf = write_crate_gltf(sources.path()).display().to_string();
    let project = root.clone();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                Some(edit_a_texture(client, &project, gltf).await)
            })
    });
    run_with(
        server_io,
        gpu,
        concerto_editor::mcp::editor(
            game,
            root.clone(),
            vec![concerto_texture_editor::install, texture_tools],
        ),
        SIZE,
    );
    let edited = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");

    assert!(edited.imported.contains(TEXTURE), "{}", edited.imported);
    assert!(
        edited
            .opened
            .ends_with("· \"1024×1024 · Bc1RgbaUnormSrgb · 11 mips · 0.7 MB on the GPU\""),
        "{}",
        edited.opened
    );
    assert!(
        edited
            .derived
            .ends_with("· unsaved · \"512×512 · Bc7RgbaUnormSrgb · 10 mips · 0.3 MB on the GPU\""),
        "{}",
        edited.derived
    );
    assert_eq!(edited.role.is_error, Some(true), "{:?}", edited.role);
    assert!(
        text(&edited.role).contains("role is set by the importer"),
        "{}",
        text(&edited.role)
    );
    assert_eq!(edited.refused.is_error, Some(true), "{:?}", edited.refused);
    assert!(
        text(&edited.refused).contains("sRGB") && text(&edited.refused).contains("then save"),
        "{}",
        text(&edited.refused)
    );
    assert!(edited.saved.contains(TEXTURE), "{}", edited.saved);
    assert_eq!(edited.reloaded, "Bc7RgbaUnormSrgb");
    let file = std::fs::read(root.join(TEXTURE)).unwrap();
    let (header, texture) = decode_content_asset::<Texture>(&file).unwrap();
    assert!(header.provenance.is_some());
    assert_eq!(texture.role, TextureRole::Color);
    assert_eq!(
        texture.settings,
        TextureSettings::default()
            .with_compression(Compression::Bc7)
            .with_max_size(MaxSize::S512)
    );
    assert_eq!(texture.source.unwrap().bytes, colour);
}
