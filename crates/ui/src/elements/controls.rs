use concerto_color::Color;
use concerto_ecs::{
    component::bundle::IntoBundle,
    signal::listener::{IntoListener, Listener},
};
use glam::Vec2;

use super::{Layout, Listening, Shape, Themed, Typography};
use crate::{
    checkbox::{UICheckbox, UICheckboxChanged, toggle_checkbox},
    color_picker::{UIColorChanged, UIColorPicker},
    interaction::{Interactable, UIClick, UIDrag, UIPointerDown},
    material::UIMaterial,
    node::{AlignContent, AlignItems, FlexDirection, UINode, UIRect},
    scroll::UISplitAxis,
    slider::{UISlider, UISliderChanged, drag_slider, press_slider},
    splitter::{SPLITTER_HIT_SIZE, UISplitter, UISplitterDragged, drag_splitter},
    text::UIText,
    theme::UITheme,
    transform::UIValue,
};

/// A labelled checkbox that fills with the accent when checked.
pub struct Checkbox {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    checkbox: UICheckbox,
    text: UIText,
}

/// A horizontal value slider.
pub struct Slider {
    theme: UITheme,
    node: UINode,
    material: UIMaterial,
    slider: UISlider,
}

/// A color picker laid out as a column; see [`UIColorPicker`].
pub struct ColorPicker {
    theme: UITheme,
    node: UINode,
    picker: UIColorPicker,
}

/// A draggable gap between two siblings, with a line while hovered or dragged.
///
/// It moves nothing itself: its owner decides what a drag means.
pub struct Splitter {
    node: UINode,
    splitter: UISplitter,
}

impl UITheme {
    /// A [`UICheckbox`] labelled `label`, ticked when `checked`, in the accent colour.
    pub fn checkbox(&self, label: impl Into<String>, checked: bool) -> Checkbox {
        Checkbox {
            theme: self.clone(),
            node: UINode::default()
                .with_height(UIValue::Px(self.control_height))
                .with_flex_shrink(0.0)
                .with_padding(UIRect::axes(self.spacing_xs, self.spacing_sm)),
            material: UIMaterial::with_border(self.surface_raised, self.border, 1.0),
            checkbox: UICheckbox {
                checked,
                checked_color: self.accent,
                unchecked_color: self.surface_raised,
            },
            text: UIText {
                wrap: false,
                ..self.body_text(label)
            },
        }
    }

    /// A [`UISlider`] from `min` to `max` at `value`, a control's height tall.
    pub fn slider(&self, value: f32, min: f32, max: f32) -> Slider {
        Slider {
            theme: self.clone(),
            node: UINode::default().with_height(UIValue::Px(self.control_height)),
            material: UIMaterial::flat(self.surface_raised),
            slider: UISlider::new(value, min, max),
        }
    }

    /// A [`UIColorPicker`] showing `color`.
    pub fn color_picker(&self, color: Color) -> ColorPicker {
        ColorPicker {
            theme: self.clone(),
            node: UINode::default()
                .with_flex_direction(FlexDirection::Column)
                .with_align_items(AlignItems::Start)
                .with_gap(Vec2::splat(self.spacing_sm)),
            picker: UIColorPicker::new(color),
        }
    }

    /// A splitter dragged along `axis`.
    ///
    /// A `Horizontal` splitter stands between columns, a `Vertical` one between rows.
    pub fn splitter(&self, axis: UISplitAxis) -> Splitter {
        let node = match axis {
            UISplitAxis::Horizontal => UINode::default()
                .with_width(UIValue::Px(SPLITTER_HIT_SIZE))
                .with_flex_direction(FlexDirection::Row),
            UISplitAxis::Vertical => UINode::default()
                .with_height(UIValue::Px(SPLITTER_HIT_SIZE))
                .with_flex_direction(FlexDirection::Column),
        };
        Splitter {
            node: node
                .with_flex_shrink(0.0)
                .with_align_self(AlignItems::Stretch)
                .with_align_items(AlignItems::Stretch)
                .with_justify_content(AlignContent::Center),
            splitter: UISplitter {
                axis,
                color: Color::TRANSPARENT,
                active_color: self.accent,
            },
        }
    }
}

impl Checkbox {
    /// Runs `listener` each time the box is toggled.
    ///
    /// Attach it after the other modifiers.
    pub fn on_change<M>(
        self,
        listener: impl IntoListener<UICheckboxChanged, M>,
    ) -> Listening<Self, UICheckboxChanged> {
        Listening::new(self, listener)
    }
}

impl Slider {
    /// Runs `listener` each time a drag changes the value.
    ///
    /// Attach it after the other modifiers.
    pub fn on_change<M>(
        self,
        listener: impl IntoListener<UISliderChanged, M>,
    ) -> Listening<Self, UISliderChanged> {
        Listening::new(self, listener)
    }
}

impl ColorPicker {
    /// Runs `listener` each time the user changes the color.
    ///
    /// Attach it after the other modifiers.
    pub fn on_change<M>(
        self,
        listener: impl IntoListener<UIColorChanged, M>,
    ) -> Listening<Self, UIColorChanged> {
        Listening::new(self, listener)
    }
}

impl Splitter {
    /// Runs `listener` with each movement of a drag along the axis.
    ///
    /// Attach it after the other modifiers.
    pub fn on_drag<M>(
        self,
        listener: impl IntoListener<UISplitterDragged, M>,
    ) -> Listening<Self, UISplitterDragged> {
        Listening::new(self, listener)
    }
}

impl IntoBundle for Checkbox {
    type Bundle = (
        UINode,
        UIMaterial,
        UICheckbox,
        Interactable,
        UIText,
        Listener<UIClick>,
    );

    fn into_bundle(self) -> Self::Bundle {
        (
            self.node,
            self.material,
            self.checkbox,
            Interactable,
            self.text,
            toggle_checkbox.into_listener(),
        )
    }
}

impl IntoBundle for Slider {
    type Bundle = (
        UINode,
        UIMaterial,
        UISlider,
        Interactable,
        Listener<UIPointerDown>,
        Listener<UIDrag>,
    );

    fn into_bundle(self) -> Self::Bundle {
        (
            self.node,
            self.material,
            self.slider,
            Interactable,
            press_slider.into_listener(),
            drag_slider.into_listener(),
        )
    }
}

impl IntoBundle for ColorPicker {
    type Bundle = (UINode, UIColorPicker);

    fn into_bundle(self) -> Self::Bundle {
        (self.node, self.picker)
    }
}

impl IntoBundle for Splitter {
    type Bundle = (UINode, UISplitter, Interactable, Listener<UIDrag>);

    fn into_bundle(self) -> Self::Bundle {
        (
            self.node,
            self.splitter,
            Interactable,
            drag_splitter.into_listener(),
        )
    }
}

impl Themed for Checkbox {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Checkbox {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Checkbox {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Typography for Checkbox {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}

impl Themed for Slider {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for Slider {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Shape for Slider {
    fn material_mut(&mut self) -> &mut UIMaterial {
        &mut self.material
    }
}

impl Themed for ColorPicker {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Layout for ColorPicker {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}

impl Layout for Splitter {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}
