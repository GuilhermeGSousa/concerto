use std::path::Path;

use concerto_app::App;
use concerto_ecs::{
    CommandQueue, Entity, IntoSystem, Query, Res, System, World,
    entity::hierarchy::{ChildOf, Children},
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::{
    asset_picker::{
        AssetPickerMenu, asset_name, open_asset_menu, picker_button, picker_caret, picker_label,
    },
    clip_editor::{
        ClipFacts, default_scene,
        preview::{ClearClip, clip_preview},
        read_clip, read_shown_scene,
    },
    hosted::HostedApp,
    scene::load_scene,
};
use concerto_foundation::assets::{
    AssetId,
    registry::{AssetEntry, AssetRegistry},
};
use concerto_ui::{anchor::UIAnchoredPanel, interaction::UIClick, theme::UITheme};

fn pick(_: AssetId) -> Listener<UIClick> {
    (|_: On<UIClick>| {}).into_listener()
}

fn open_menu(
    menus: Query<(Entity, &mut AssetPickerMenu, &mut UIAnchoredPanel)>,
    registry: Res<AssetRegistry>,
    theme: Res<UITheme>,
    buttons: Query<Entity>,
    mut cmd: CommandQueue,
) {
    let button = buttons.iter().next().unwrap();
    open_asset_menu(
        button, "Skeleton", None, None, pick, &menus, &registry, &theme, &mut cmd,
    );
}

#[test]
fn an_extension_crate_opens_the_shared_asset_menu() {
    let mut world = World::new();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.insert_resource(AssetRegistry::default());
    world.insert_resource(UITheme::default());
    world.spawn(());
    let mut system = open_menu.into_system();
    system.initialize(&mut world);

    system.run_and_apply((), &mut world);

    assert_eq!(
        world
            .query::<&AssetPickerMenu, ()>()
            .iter(&mut world)
            .count(),
        1
    );
}

#[test]
fn an_extension_crate_reaches_what_it_builds_on() {
    let theme = UITheme::default();
    let _ = (
        picker_button(&theme),
        picker_label(&theme),
        picker_caret(&theme),
    );
    assert_eq!(
        asset_name(&AssetRegistry::default(), AssetId::new()),
        "Missing"
    );
    let _ = ClearClip;
    let _: fn(&mut App) = clip_preview;
    let _: fn(&Path, &AssetEntry) -> anyhow::Result<_> = read_clip;
    let _: for<'a> fn(&AssetEntry, &'a AssetRegistry) -> Option<&'a AssetEntry> = default_scene;
    let _ = read_shown_scene;
    let _ = load_scene;
    let _ = ClipFacts::of;
    let _ = HostedApp::world;
    let _ = HostedApp::world_mut;
    let _ = HostedApp::mark_saved;
}
