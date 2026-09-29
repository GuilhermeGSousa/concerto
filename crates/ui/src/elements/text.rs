//! Text builders.
use super::{Layout, Themed, Typography};
use crate::{node::UINode, text::UIText, theme::UITheme};

/// Text for an entity that already has a node: a button, a field, an icon slot.
pub struct Text {
    pub(crate) theme: UITheme,
    pub(crate) text: UIText,
}

/// A standalone text node.
pub struct Label {
    node: UINode,
    text: Text,
}

impl UITheme {
    /// Body text: the medium size in the theme's text colour.
    pub fn text(&self, value: impl Into<String>) -> Text {
        Text {
            theme: self.clone(),
            text: self.body_text(value),
        }
    }

    /// A node holding body text.
    pub fn label(&self, value: impl Into<String>) -> Label {
        Label {
            node: UINode::default(),
            text: self.text(value),
        }
    }

    pub(crate) fn body_text(&self, value: impl Into<String>) -> UIText {
        UIText {
            text: value.into(),
            font_size: self.font_size_md,
            line_height: self.line_height(self.font_size_md),
            color: self.text,
            ..Default::default()
        }
    }
}

impl Text {
    fn into_parts(self) -> UIText {
        self.text
    }
}

impl From<Text> for UIText {
    fn from(text: Text) -> Self {
        text.text
    }
}

impl Label {
    fn into_parts(self) -> (UINode, UIText) {
        (self.node, self.text.text)
    }
}

bundle!(Text => UIText);
bundle!(Label => (UINode, UIText));

impl Themed for Text {
    fn theme(&self) -> &UITheme {
        &self.theme
    }
}

impl Typography for Text {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text
    }
}

impl Themed for Label {
    fn theme(&self) -> &UITheme {
        &self.text.theme
    }
}

impl Typography for Label {
    fn text_mut(&mut self) -> &mut UIText {
        &mut self.text.text
    }
}

impl Layout for Label {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}
