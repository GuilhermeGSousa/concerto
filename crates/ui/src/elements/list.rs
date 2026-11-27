use concerto_color::Color;
use concerto_ecs::{
    command::{CommandQueue, EntityCommandQueue},
    entity::{Entity, hierarchy::ChildOf},
    signal::listener::Listener,
};

use super::{Layout, Shape, Typography};
use crate::{
    items::{UIItemActivated, UISelected},
    list::UIList,
    node::{UINode, UIRect},
    search::UISearchText,
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};

/// A `UIList` or `UITree` with its items, spawned in one call.
///
/// A searchable one also gets a search field over it.
pub struct ListBuilder {
    theme: UITheme,
    node: UINode,
    tree: bool,
    row_height: Option<f32>,
    selectable: bool,
    click_activates: bool,
    search: Option<String>,
    empty: Option<String>,
    items: Vec<ListItem>,
}

/// One row of a [`ListBuilder`].
///
/// A row shows a label or content of the caller's, and carries its search text,
/// its pick and, in a tree, its children.
pub struct ListItem {
    theme: UITheme,
    content: ItemContent,
    search: Option<String>,
    selected: bool,
    expanded: bool,
    activate: Option<Listener<UIItemActivated>>,
    children: Vec<ListItem>,
}

type BuildContent = Box<dyn for<'a> FnOnce(&mut EntityCommandQueue<'a>)>;

enum ItemContent {
    Label {
        label: String,
        detail: Option<String>,
    },
    Custom(BuildContent),
}

/// The entities a [`ListBuilder`] spawned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnedList {
    /// The `UIList` or `UITree`, or the empty label when there were no items and an empty text was given.
    pub list: Entity,
    /// The search field, when the list is searchable and has items.
    pub field: Option<Entity>,
}

impl UITheme {
    /// A list whose items are rows of the theme's row height, selectable, activated by a double click or Enter.
    pub fn list(&self) -> ListBuilder {
        self.list_of(false)
    }

    /// A tree whose items nest through [`ListItem::child`], with carets and connector lines.
    pub fn tree(&self) -> ListBuilder {
        self.list_of(true)
    }

    /// A row showing `label`, which a search matches.
    pub fn list_item(&self, label: impl Into<String>) -> ListItem {
        let label = label.into();
        ListItem {
            theme: self.clone(),
            search: Some(label.clone()),
            content: ItemContent::Label {
                label,
                detail: None,
            },
            selected: false,
            expanded: true,
            activate: None,
            children: Vec::new(),
        }
    }

    /// A tree row showing `label`, expanded; the same as [`UITheme::list_item`].
    pub fn tree_item(&self, label: impl Into<String>) -> ListItem {
        self.list_item(label)
    }

    /// A row whose content `build` adds as children, matched by a search on `search`.
    pub fn list_item_with(
        &self,
        search: impl Into<String>,
        build: impl for<'a> FnOnce(&mut EntityCommandQueue<'a>) + 'static,
    ) -> ListItem {
        ListItem {
            theme: self.clone(),
            search: Some(search.into()),
            content: ItemContent::Custom(Box::new(build)),
            selected: false,
            expanded: true,
            activate: None,
            children: Vec::new(),
        }
    }

    fn list_of(&self, tree: bool) -> ListBuilder {
        ListBuilder {
            theme: self.clone(),
            node: UINode::default().with_flex_shrink(0.0),
            tree,
            row_height: None,
            selectable: true,
            click_activates: false,
            search: None,
            empty: None,
            items: Vec::new(),
        }
    }
}

impl ListBuilder {
    /// Sets the height of every row, in logical pixels, in place of the theme's `row_height`.
    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = Some(height);
        self
    }

    /// Rows that cannot be highlighted, selected or activated.
    pub fn unselectable(mut self) -> Self {
        self.selectable = false;
        self
    }

    /// One click selects and activates a row, as in a menu.
    pub fn click_activates(mut self) -> Self {
        self.click_activates = true;
        self
    }

    /// A search field above the list, showing `placeholder`, that narrows it to the rows whose search text holds its text.
    pub fn searchable(mut self, placeholder: impl Into<String>) -> Self {
        self.search = Some(placeholder.into());
        self
    }

    /// A muted label shown instead of the list when it has no items.
    pub fn empty_text(mut self, text: impl Into<String>) -> Self {
        self.empty = Some(text.into());
        self
    }

    /// Adds a row.
    pub fn item(mut self, item: ListItem) -> Self {
        self.items.push(item);
        self
    }

    /// Adds every row of `items`.
    pub fn items(mut self, items: impl IntoIterator<Item = ListItem>) -> Self {
        self.items.extend(items);
        self
    }

    /// Spawns the field, if any, then the list, as the last children of `parent`.
    pub fn spawn_in(self, parent: Entity, cmd: &mut CommandQueue) -> SpawnedList {
        if self.items.is_empty()
            && let Some(empty) = self.empty
        {
            let list = cmd
                .spawn((
                    self.theme
                        .label(empty)
                        .muted()
                        .font_size(self.theme.font_size_sm)
                        .no_wrap()
                        .padding(UIRect::axes(6.0, 8.0)),
                    ChildOf::new(parent),
                ))
                .entity();
            return SpawnedList { list, field: None };
        }
        let list = if self.tree {
            cmd.spawn((
                self.node,
                UITree {
                    row_height: self.row_height,
                    selectable: self.selectable,
                    click_activates: self.click_activates,
                    ..Default::default()
                },
            ))
        } else {
            cmd.spawn((
                self.node,
                UIList {
                    row_height: self.row_height,
                    selectable: self.selectable,
                    click_activates: self.click_activates,
                },
            ))
        }
        .entity();
        for item in self.items {
            item.spawn(list, self.tree, cmd);
        }
        let field = self.search.map(|placeholder| {
            cmd.spawn((
                self.theme.search_field(placeholder, list),
                ChildOf::new(parent),
            ))
            .entity()
        });
        cmd.add_child(parent, list);
        SpawnedList { list, field }
    }
}

impl ListItem {
    /// Muted text after the label, which a search also matches.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        if let ItemContent::Label {
            label,
            detail: slot,
        } = &mut self.content
        {
            self.search = Some(format!("{label} {detail}"));
            *slot = Some(detail);
        }
        self
    }

    /// What a search matches the row on, in place of its label.
    pub fn search_text(mut self, text: impl Into<String>) -> Self {
        self.search = Some(text.into());
        self
    }

    /// Keeps the row shown whatever a search holds.
    pub fn always_shown(mut self) -> Self {
        self.search = None;
        self
    }

    /// Starts the row selected, as the value a menu was opened on.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether a tree row starts with its children shown; true by default.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Runs `listener` when the row is activated: by a double click, Enter, or one click in a list that `click_activates`.
    pub fn on_activate(mut self, listener: Listener<UIItemActivated>) -> Self {
        self.activate = Some(listener);
        self
    }

    /// Nests `child` under this row.
    ///
    /// Only a tree shows it.
    pub fn child(mut self, child: ListItem) -> Self {
        self.children.push(child);
        self
    }

    /// Nests every row of `children` under this row.
    pub fn children(mut self, children: impl IntoIterator<Item = ListItem>) -> Self {
        self.children.extend(children);
        self
    }

    fn spawn(self, parent: Entity, tree: bool, cmd: &mut CommandQueue) -> Entity {
        let theme = self.theme;
        let mut row = cmd.spawn((
            theme
                .canvas()
                .fill(Color::TRANSPARENT)
                .radius_sm()
                .row()
                .gap(theme.spacing_sm)
                .padding(UIRect::axes(0.0, theme.spacing_sm)),
            ChildOf::new(parent),
        ));
        if tree {
            row.insert(UITreeElement {
                expanded: self.expanded,
            });
        }
        if let Some(search) = self.search {
            row.insert(UISearchText(search));
        }
        if self.selected {
            row.insert(UISelected);
        }
        if let Some(activate) = self.activate {
            row.insert(activate);
        }
        match self.content {
            ItemContent::Label { label, detail } => {
                row.add_child(
                    theme
                        .label(label)
                        .font_size(theme.font_size_sm)
                        .single_line()
                        .grow()
                        .shrink(1.0)
                        .min_width(UIValue::Px(0.0)),
                );
                if let Some(detail) = detail {
                    row.add_child(theme.label(detail).muted().font_size(10.5).no_wrap());
                }
            }
            ItemContent::Custom(build) => build(&mut row),
        }
        let entity = row.entity();
        if tree {
            for child in self.children {
                child.spawn(entity, tree, cmd);
            }
        }
        entity
    }
}

impl Layout for ListBuilder {
    fn node_mut(&mut self) -> &mut UINode {
        &mut self.node
    }
}
