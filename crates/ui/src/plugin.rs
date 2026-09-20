use app::{
    plugins::Plugin,
    schedule_groups::{Extract, LateUpdate, Render},
};
use glyphon::{Cache, SwashCache, Viewport};
use render::{
    device::RenderDevice, material_plugin::MaterialPlugin, queue::RenderQueue,
    resources::RenderContext,
};

use crate::{
    checkbox::{UICheckboxChanged, sync_checkbox_material, toggle_checkboxes},
    focus::{
        FocusedWidget, UIFocusGained, UIFocusLost, UIFocusNext, UIFocusPrevious, sync_text_capture,
        update_focus,
    },
    interaction::{
        HoveredNode, UIClick, UIDrag, UIInputState, UIPointerDown, UIPointerEnter, UIPointerLeave,
        UIPointerUp, apply_interaction_styles, update_ui_interaction,
    },
    material::UIMaterial,
    node::{
        UILayoutDiagnostics, UILayoutEngine, UITextMeasure, compute_ui_nodes, extract_ui_materials,
        extract_ui_nodes, sync_material_params, sync_viewport_textures,
    },
    render::{prepare_text_renderer, ui_renderpass, update_text_viewport},
    resources::UIRenderDiagnostics,
    scroll::{
        drag_scrollbar_thumbs, setup_scrollbars, sync_scroll_content, sync_scrollbar_thumbs,
        sync_scrollbar_tracks, sync_split_panes, update_scroll_areas, update_split_panes,
        update_virtual_lists,
    },
    slider::{UISliderChanged, setup_slider_visuals, sync_slider_fill, update_slider_drag},
    text::{
        extract_text_nodes,
        fonts::{UIFonts, build_font_system},
        resources::{
            TextAtlas, TextCache, TextFontSystem, TextRenderers, TextSwashCache, TextViewport,
        },
    },
    text_input::{
        UITextInputCancelled, UITextInputChanged, UITextInputSubmitted, update_text_inputs,
    },
    theme::UITheme,
    widgets::{
        UICollapsibleChanged, UITabChanged, sync_tab_bodies, update_popup_menus, update_tooltips,
        update_widgets,
    },
};

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut app::App) {
        app.register_plugin(MaterialPlugin::<UIMaterial>::pipeline_only());

        // Focus moves with Tab by default; rebind through ActionMap like any
        // other action.
        {
            let actions = app
                .get_resource_mut::<window::input::actions::ActionMap>()
                .expect("WindowPlugin must be registered before UIPlugin");
            actions.bind_global(
                UIFocusNext,
                window::input::actions::Shortcut::key(window::input::KeyCode::Tab),
            );
            actions.bind_global(
                UIFocusPrevious,
                window::input::actions::Shortcut::key(window::input::KeyCode::Tab).with_shift(),
            );
        }

        // Resources
        app.insert_resource(HoveredNode::default());
        app.insert_resource(UIInputState::default());
        app.insert_resource(FocusedWidget::default());
        app.insert_resource(UITheme::default());
        app.insert_resource(UILayoutEngine::default());
        app.insert_resource(UILayoutDiagnostics::default());
        let render_diagnostics = UIRenderDiagnostics::default();
        app.insert_resource(render_diagnostics.clone());
        app.render_mut().insert_resource(render_diagnostics);

        // Events
        app.register_event::<UIClick>();
        app.register_event::<UIPointerDown>();
        app.register_event::<UIPointerUp>();
        app.register_event::<UIPointerEnter>();
        app.register_event::<UIPointerLeave>();
        app.register_event::<UIDrag>();
        app.register_event::<UICheckboxChanged>();
        app.register_event::<UISliderChanged>();
        app.register_event::<UITextInputChanged>();
        app.register_event::<UITextInputSubmitted>();
        app.register_event::<UITextInputCancelled>();
        app.register_event::<UIFocusGained>();
        app.register_event::<UIFocusLost>();
        app.register_event::<UICollapsibleChanged>();
        app.register_event::<UITabChanged>();

        // ── LateUpdate: read input, mutate widget state, then lay out ───────
        // Systems run in registration order, so the layout pass at the end of
        // this list sees everything mutated before it — including application
        // code, as long as `UIPlugin` is registered after the plugins that
        // build UI. Hit testing itself reads the previous frame's `UILayout`;
        // everything it changes is laid out below, so a click, drag or scroll
        // is on screen in the frame that produced it.
        // 1. Hit test — updates HoveredNode and fires pointer events.
        app.add_system(LateUpdate, update_ui_interaction);
        // 2. Focus — reads HoveredNode and focus actions, updates FocusedWidget.
        app.add_system(LateUpdate, update_focus);
        app.add_system(LateUpdate, sync_text_capture);
        // 3. Widgets react to clicks and focus.
        app.add_system(LateUpdate, toggle_checkboxes);
        app.add_system(LateUpdate, update_text_inputs);
        app.add_system(LateUpdate, update_widgets);
        app.add_system(LateUpdate, sync_tab_bodies);
        app.add_system(LateUpdate, update_tooltips);
        app.add_system(LateUpdate, update_popup_menus);
        app.add_system(LateUpdate, update_scroll_areas);
        // Virtual ranges follow the scroll offset set above, so that the content
        // shift below is computed against this frame's range.
        app.add_system(LateUpdate, update_virtual_lists);
        app.add_system(LateUpdate, update_split_panes);
        app.add_system(LateUpdate, update_slider_drag);
        app.add_system(LateUpdate, drag_scrollbar_thumbs);
        // 4. Spawn child visuals for new widgets (commands flush immediately after).
        app.add_system(LateUpdate, setup_slider_visuals);
        app.add_system(LateUpdate, setup_scrollbars);
        // 5. Project widget state onto the UINodes the layout pass will read.
        app.add_system(LateUpdate, sync_slider_fill);
        app.add_system(LateUpdate, sync_scroll_content);
        app.add_system(LateUpdate, sync_split_panes);
        // 6. Materials follow interaction state; independent of layout.
        app.add_system(LateUpdate, sync_checkbox_material);
        app.add_system(LateUpdate, sync_viewport_textures);
        app.add_system(LateUpdate, apply_interaction_styles);

        // 7. Resolve layout and everything derived from it, last.
        // Taffy layout pass — computes UILayout for all nodes.
        app.add_system(LateUpdate, compute_ui_nodes);
        // Sync engine-managed border_params uniform from user-facing border_width.
        app.add_system(LateUpdate, sync_material_params);
        // Scrollbars read the viewport measured by the layout pass above.
        app.add_system(LateUpdate, sync_scrollbar_tracks);
        app.add_system(LateUpdate, sync_scrollbar_thumbs);

        // ── Render ──────────────────────────────────────────────────────────────
        app.render_mut()
            .add_system(Extract, extract_ui_nodes)
            .add_system(Extract, extract_ui_materials)
            .add_system(Extract, extract_text_nodes)
            .add_system(Render, update_text_viewport)
            .add_system(Render, prepare_text_renderer)
            // Viewport nodes: create fresh bind groups before ui_renderpass.
            .add_system(Render, ui_renderpass);
    }

    fn finish(&self, app: &mut app::App) {
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

        // Both font systems are built here, from the same registry, so
        // measurement and rendering cannot disagree about what a face is.
        let fonts = app.get_resource::<UIFonts>();
        let default_fonts = UIFonts::default();
        let fonts = fonts.unwrap_or(&default_fonts);
        let measure = build_font_system(fonts);
        let font_system = build_font_system(fonts);
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);

        // Text renderers are created on demand, one per z-layer that carries
        // text, by `prepare_text_renderer`.
        let atlas = glyphon::TextAtlas::new(device, queue, &cache, context.surface_config.format);

        app.insert_resource(UITextMeasure::new(measure));
        app.render_mut()
            .insert_resource(TextRenderers::default())
            .insert_resource(TextCache(cache))
            .insert_resource(TextSwashCache(swash_cache))
            .insert_resource(TextViewport(viewport))
            .insert_resource(TextFontSystem(font_system))
            .insert_resource(TextAtlas(atlas));
    }
}
