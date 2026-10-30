use concerto_app::{
    plugins::Plugin,
    schedule_groups::{Extract, LateUpdate, Render},
};
use concerto_ecs::{IntoSetConfig, IntoSystemConfig};
use concerto_render::{
    device::RenderDevice, material_plugin::MaterialPlugin, queue::RenderQueue,
    resources::RenderContext, sets::RenderSet,
};
use concerto_window::input::{
    KeyCode,
    actions::{ActionMap, Shortcut},
};
use glyphon::{Cache, SwashCache, Viewport};

use crate::{
    anchor::{UIDismissPanel, UIPanelStack, dismiss_panels, track_panel_stack},
    checkbox::sync_checkbox_material,
    color_picker::{commit_color_picker_fields, setup_color_pickers, sync_color_picker_parts},
    focus::{
        FocusedWidget, UIFocusGained, UIFocusLost, UIFocusNext, UIFocusPrevious, sync_text_capture,
        update_focus,
    },
    interaction::{HoveredNode, UIInputState, apply_interaction_styles, update_ui_interaction},
    items::{
        UIListActivate, UIListFirst, UIListLast, UIListNext, UIListPrevious, UITreeCollapse,
        UITreeExpand, install_listeners, keep_focus_on_shown_items, navigate_items, paint_items,
    },
    list::update_lists,
    material::UIMaterial,
    node::{
        UILayoutDiagnostics, UILayoutEngine, UIQuadIndices, UITextMeasure, compute_ui_nodes,
        extract_shared_viewports, extract_ui_materials, extract_ui_nodes, install_layout_listeners,
        sync_material_params, sync_shared_viewport_textures, sync_viewport_textures,
    },
    render::{prepare_text_renderer, ui_renderpass, update_text_viewport},
    resources::UIRenderDiagnostics,
    scroll::{
        setup_scrollbars, sync_scroll_content, sync_scrollbar_thumbs, sync_scrollbar_tracks,
        sync_split_panes, update_scroll_areas,
    },
    sets::UiSet,
    slider::{setup_slider_visuals, sync_slider_fill},
    text::{
        extract_text_nodes,
        fonts::{UIFonts, build_font_system},
        resources::{
            TextAtlas, TextCache, TextFontSystem, TextRenderers, TextSwashCache, TextViewport,
        },
    },
    text_input::{
        UITextInputCancelled, UITextInputChanged, UITextInputSubmitted, project_text_inputs,
        setup_text_inputs, update_text_inputs,
    },
    theme::UITheme,
    tree::update_trees,
    widgets::{sync_tab_bodies, update_tooltips},
};

fn bind_default_actions(actions: &mut ActionMap) {
    actions.bind_global(UIFocusNext, Shortcut::key(KeyCode::Tab));
    actions.bind_global(UIFocusPrevious, Shortcut::key(KeyCode::Tab).with_shift());
    actions.bind_global(UIDismissPanel, Shortcut::key(KeyCode::Escape));
    actions.bind_global(UIListNext, Shortcut::key(KeyCode::ArrowDown));
    actions.bind_global(UIListPrevious, Shortcut::key(KeyCode::ArrowUp));
    actions.bind_global(UIListFirst, Shortcut::key(KeyCode::Home));
    actions.bind_global(UIListLast, Shortcut::key(KeyCode::End));
    actions.bind_global(UIListActivate, Shortcut::key(KeyCode::Enter));
    actions.bind_global(UITreeExpand, Shortcut::key(KeyCode::ArrowRight));
    actions.bind_global(UITreeCollapse, Shortcut::key(KeyCode::ArrowLeft));
}

/// Registers the UI's resources, events and systems, ordered by [`UiSet`] in `LateUpdate`.
///
/// # Panics
///
/// Panics if `WindowPlugin` and `RenderPlugin` are not registered before it.
pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_plugin(MaterialPlugin::<UIMaterial>::pipeline_only());

        bind_default_actions(
            app.get_resource_mut::<ActionMap>()
                .expect("WindowPlugin must be registered before UIPlugin"),
        );

        app.insert_resource(HoveredNode::default());
        app.insert_resource(UIInputState::default());
        app.insert_resource(FocusedWidget::default());
        app.insert_resource(UITheme::default());
        app.insert_resource(UILayoutEngine::default());
        app.insert_resource(UILayoutDiagnostics::default());
        app.insert_resource(UIPanelStack::default());
        crate::list::install_listeners(app.main_mut().world_mut());
        crate::tree::install_listeners(app.main_mut().world_mut());
        install_listeners(app.main_mut().world_mut());
        install_layout_listeners(app.main_mut().world_mut());
        let render_diagnostics = UIRenderDiagnostics::default();
        app.insert_resource(render_diagnostics.clone());
        app.render_mut().insert_resource(render_diagnostics);

        app.register_event::<UITextInputChanged>();
        app.register_event::<UITextInputSubmitted>();
        app.register_event::<UITextInputCancelled>();
        app.register_event::<UIFocusGained>();
        app.register_event::<UIFocusLost>();

        app.configure_sets(
            LateUpdate,
            (
                UiSet::Input,
                UiSet::Widgets,
                UiSet::Setup,
                UiSet::Project,
                UiSet::Materials,
                UiSet::Layout,
                UiSet::PostLayout,
            )
                .chain(),
        );

        app.add_system(
            LateUpdate,
            (
                update_ui_interaction,
                update_focus.after(update_ui_interaction),
                sync_text_capture,
            )
                .in_set(UiSet::Input),
        );
        app.add_system(
            LateUpdate,
            (
                update_text_inputs,
                commit_color_picker_fields.after(update_text_inputs),
                dismiss_panels.after(update_text_inputs),
                sync_tab_bodies,
                update_tooltips,
                update_scroll_areas,
                keep_focus_on_shown_items,
                navigate_items.after(keep_focus_on_shown_items),
            )
                .in_set(UiSet::Widgets),
        );
        app.add_system(
            LateUpdate,
            (
                setup_slider_visuals,
                setup_color_pickers,
                setup_scrollbars,
                setup_text_inputs,
            )
                .in_set(UiSet::Setup),
        );
        app.add_system(
            LateUpdate,
            (
                sync_slider_fill,
                sync_color_picker_parts.before(project_text_inputs),
                project_text_inputs,
                sync_scroll_content,
                sync_split_panes,
                update_lists,
                update_trees,
            )
                .in_set(UiSet::Project),
        );
        app.add_system(
            LateUpdate,
            (
                sync_checkbox_material,
                sync_viewport_textures,
                sync_shared_viewport_textures,
                apply_interaction_styles,
                paint_items,
            )
                .in_set(UiSet::Materials),
        );
        app.add_system(
            LateUpdate,
            (
                track_panel_stack,
                compute_ui_nodes.after(track_panel_stack),
                sync_material_params,
            )
                .in_set(UiSet::Layout),
        );
        app.add_system(
            LateUpdate,
            (sync_scrollbar_tracks, sync_scrollbar_thumbs).in_set(UiSet::PostLayout),
        );

        app.render_mut()
            .add_system(Extract, extract_ui_nodes)
            .add_system(
                Extract,
                extract_shared_viewports.before(extract_ui_materials),
            )
            .add_system(Extract, extract_ui_materials)
            .add_system(Extract, extract_text_nodes)
            .add_system(Render, update_text_viewport)
            .add_system(Render, prepare_text_renderer)
            .add_system(Render, ui_renderpass.after(RenderSet::Overlay));
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let device = app
            .render()
            .get_resource::<RenderDevice>()
            .expect("RenderDevice resource not found");

        let context = app
            .render()
            .get_resource::<RenderContext>()
            .expect("RenderContext resource not found");

        let queue = app
            .render()
            .get_resource::<RenderQueue>()
            .expect("RenderQueue resource not found");

        let fonts = app.get_resource::<UIFonts>();
        let default_fonts = UIFonts::default();
        let fonts = fonts.unwrap_or(&default_fonts);
        let measure = build_font_system(fonts);
        let font_system = build_font_system(fonts);
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);

        let atlas = glyphon::TextAtlas::new(device, queue, &cache, context.surface_config.format);
        let quad_indices = UIQuadIndices::new(device);

        app.insert_resource(UITextMeasure::new(measure));
        app.render_mut()
            .insert_resource(TextRenderers::default())
            .insert_resource(TextCache(cache))
            .insert_resource(TextSwashCache(swash_cache))
            .insert_resource(TextViewport(viewport))
            .insert_resource(TextFontSystem(font_system))
            .insert_resource(TextAtlas(atlas))
            .insert_resource(quad_indices);
    }
}

#[cfg(test)]
mod tests {
    use concerto_window::input::actions::ActionLabel;

    use super::*;

    #[test]
    fn the_list_and_tree_keys_resolve_to_their_actions() {
        let mut actions = ActionMap::default();
        bind_default_actions(&mut actions);

        for (key, action) in [
            (KeyCode::ArrowDown, UIListNext.intern()),
            (KeyCode::ArrowUp, UIListPrevious.intern()),
            (KeyCode::Home, UIListFirst.intern()),
            (KeyCode::End, UIListLast.intern()),
            (KeyCode::Enter, UIListActivate.intern()),
            (KeyCode::ArrowRight, UITreeExpand.intern()),
            (KeyCode::ArrowLeft, UITreeCollapse.intern()),
        ] {
            assert_eq!(actions.resolve(Shortcut::key(key)), Some(action), "{key:?}");
        }
    }
}
