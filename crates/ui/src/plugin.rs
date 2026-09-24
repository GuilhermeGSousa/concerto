use concerto_app::{
    plugins::Plugin,
    schedule_groups::{Extract, LateUpdate, Render},
};
use concerto_ecs::IntoSystemConfig;
use concerto_render::{
    device::RenderDevice, material_plugin::MaterialPlugin, queue::RenderQueue,
    resources::RenderContext,
};
use glyphon::{Cache, SwashCache, Viewport};

use crate::{
    anchor::{UIDismissPanel, UIPanelStack, dismiss_panels, track_panel_stack},
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
        UICollapsibleChanged, UITabChanged, sync_tab_bodies, update_tooltips, update_widgets,
    },
};

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_plugin(MaterialPlugin::<UIMaterial>::pipeline_only());

        {
            let actions = app
                .get_resource_mut::<concerto_window::input::actions::ActionMap>()
                .expect("WindowPlugin must be registered before UIPlugin");
            actions.bind_global(
                UIFocusNext,
                concerto_window::input::actions::Shortcut::key(
                    concerto_window::input::KeyCode::Tab,
                ),
            );
            actions.bind_global(
                UIFocusPrevious,
                concerto_window::input::actions::Shortcut::key(
                    concerto_window::input::KeyCode::Tab,
                )
                .with_shift(),
            );
            actions.bind_global(
                UIDismissPanel,
                concerto_window::input::actions::Shortcut::key(
                    concerto_window::input::KeyCode::Escape,
                ),
            );
        }

        app.insert_resource(HoveredNode::default());
        app.insert_resource(UIInputState::default());
        app.insert_resource(FocusedWidget::default());
        app.insert_resource(UITheme::default());
        app.insert_resource(UILayoutEngine::default());
        app.insert_resource(UILayoutDiagnostics::default());
        app.insert_resource(UIPanelStack::default());
        let render_diagnostics = UIRenderDiagnostics::default();
        app.insert_resource(render_diagnostics.clone());
        app.render_mut().insert_resource(render_diagnostics);

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

        app.add_system(LateUpdate, update_focus.after(update_ui_interaction));
        app.add_system(LateUpdate, sync_text_capture);
        app.add_system(LateUpdate, toggle_checkboxes);
        app.add_system(LateUpdate, dismiss_panels.after(update_text_inputs));
        app.add_system(LateUpdate, update_widgets);
        app.add_system(LateUpdate, sync_tab_bodies);
        app.add_system(LateUpdate, update_tooltips);
        app.add_system(LateUpdate, update_scroll_areas);
        app.add_system(LateUpdate, update_virtual_lists);
        app.add_system(LateUpdate, update_split_panes);
        app.add_system(LateUpdate, update_slider_drag);
        app.add_system(LateUpdate, drag_scrollbar_thumbs);
        app.add_system(LateUpdate, setup_slider_visuals);
        app.add_system(LateUpdate, setup_scrollbars);
        app.add_system(LateUpdate, sync_slider_fill);
        app.add_system(LateUpdate, sync_scroll_content);
        app.add_system(LateUpdate, sync_split_panes);
        app.add_system(LateUpdate, sync_checkbox_material);
        app.add_system(LateUpdate, sync_viewport_textures);
        app.add_system(LateUpdate, apply_interaction_styles);

        app.add_system(LateUpdate, compute_ui_nodes.after(track_panel_stack));
        app.add_system(LateUpdate, sync_material_params);
        app.add_system(LateUpdate, sync_scrollbar_tracks);
        app.add_system(LateUpdate, sync_scrollbar_thumbs);

        app.render_mut()
            .add_system(Extract, extract_ui_nodes)
            .add_system(Extract, extract_ui_materials)
            .add_system(Extract, extract_text_nodes)
            .add_system(Render, update_text_viewport)
            .add_system(Render, prepare_text_renderer)
            .add_system(Render, ui_renderpass);
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
