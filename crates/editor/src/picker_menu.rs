use concerto_color::Color;
use concerto_ecs::{
    Entity, command::CommandQueue, component::Component, entity::hierarchy::ChildOf,
    signal::listener::Listener,
};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    anchor::{UIAnchorAlign, UIAnchorSide, UIAnchorTarget, UIAnchoredPanel},
    items::{UIItemActivated, UISelected},
    list::UIList,
    node::{UINode, UIRect},
    search::UISearchText,
    theme::UITheme,
    transform::UIValue,
    tree::{UITree, UITreeElement},
};

/// The width of every picker menu.
pub const MENU_WIDTH: f32 = 240.0;
/// The height of a picker menu's row.
pub const ITEM_HEIGHT: f32 = 26.0;
/// The height a picker menu's list grows to before it scrolls.
pub const LIST_MAX_HEIGHT: f32 = 280.0;

/// One row of a picker menu: what it shows, what a search matches it on, and what picking it does.
pub struct PickerEntry {
    label: String,
    detail: Option<String>,
    search: Option<String>,
    parent: Option<usize>,
    current: bool,
    pick: Listener<UIItemActivated>,
}

impl PickerEntry {
    /// A row showing `label`, matched by a search on `label`, running `pick` when chosen.
    pub fn new(label: impl Into<String>, pick: Listener<UIItemActivated>) -> Self {
        let label = label.into();
        Self {
            search: Some(label.clone()),
            label,
            detail: None,
            parent: None,
            current: false,
            pick,
        }
    }

    /// Muted text after the label, which a search also matches.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        self.search = Some(format!("{} {detail}", self.label));
        self.detail = Some(detail);
        self
    }

    /// Keeps the row shown whatever a search holds.
    pub fn always_shown(mut self) -> Self {
        self.search = None;
        self
    }

    /// Nests the row under the entry at `parent` in the same menu, which makes the menu a tree.
    pub fn with_parent(mut self, parent: Option<usize>) -> Self {
        self.parent = parent;
        self
    }

    /// Marks the row as the value the menu was opened on.
    pub fn with_current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
}

/// The entities a picker menu's content was spawned as, to be added to the menu in order.
pub struct PickerContent {
    /// The search field, when the menu has one; the panel focuses it on open.
    pub field: Option<Entity>,
    /// The list or tree of rows, or the label saying there are none.
    pub body: Entity,
}

impl PickerContent {
    /// Adds the content to `menu`, the field first.
    pub fn attach(&self, menu: Entity, cmd: &mut CommandQueue) {
        if let Some(field) = self.field {
            cmd.add_child(menu, field);
        }
        cmd.add_child(menu, self.body);
    }
}

/// A picker menu's placement below `button`, which toggles it, focusing `field` when it opens.
pub fn picker_panel(button: Entity, field: Option<Entity>) -> UIAnchoredPanel {
    let panel = UIAnchoredPanel::new(UIAnchorTarget::from_node(button))
        .with_owner(button)
        .with_side(UIAnchorSide::Below)
        .with_align(UIAnchorAlign::End)
        .with_gap(4.0)
        .with_open(true);
    match field {
        Some(field) => panel.with_focus_on_open(field),
        None => panel,
    }
}

/// Puts `content` in the picker menu `menu`, or in a new one, placed by `panel` and marked by `marker`.
pub fn show_picker_menu<M: Component>(
    menu: Option<Entity>,
    panel: UIAnchoredPanel,
    marker: M,
    content: PickerContent,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) -> Entity {
    let menu = match menu {
        Some(menu) => {
            cmd.entity(menu).despawn_children();
            cmd.insert((panel, marker), menu);
            menu
        }
        None => cmd
            .spawn((
                theme
                    .popup()
                    .width(UIValue::Px(MENU_WIDTH))
                    .padding(theme.spacing_xs)
                    .gap(theme.spacing_xs),
                panel,
                marker,
            ))
            .entity(),
    };
    content.attach(menu, cmd);
    menu
}

/// Spawns a picker menu's rows, with a search field over them when `search` names its placeholder, or `empty` when there are no rows.
pub fn spawn_picker_content(
    entries: Vec<PickerEntry>,
    search: Option<&str>,
    empty: &str,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) -> PickerContent {
    if entries.is_empty() {
        let body = cmd
            .spawn(
                theme
                    .label(empty.to_owned())
                    .muted()
                    .font_size(theme.font_size_sm)
                    .no_wrap()
                    .padding(UIRect::axes(6.0, 8.0)),
            )
            .entity();
        return PickerContent { field: None, body };
    }
    let tree = entries.iter().any(|entry| entry.parent.is_some());
    let node = UINode::default()
        .with_max_height(UIValue::Px(LIST_MAX_HEIGHT))
        .with_flex_shrink(0.0);
    let body = if tree {
        cmd.spawn((
            node,
            UITree {
                row_height: Some(ITEM_HEIGHT),
                click_activates: true,
                ..Default::default()
            },
        ))
    } else {
        cmd.spawn((
            node,
            UIList {
                row_height: Some(ITEM_HEIGHT),
                click_activates: true,
                ..Default::default()
            },
        ))
    }
    .entity();
    let mut rows: Vec<Entity> = Vec::with_capacity(entries.len());
    for entry in entries {
        let parent = entry
            .parent
            .and_then(|index| rows.get(index).copied())
            .unwrap_or(body);
        rows.push(spawn_row(entry, parent, tree, theme, cmd));
    }
    let field = search.map(|placeholder| {
        cmd.spawn(theme.search_field(placeholder.to_owned(), body))
            .entity()
    });
    PickerContent { field, body }
}

fn spawn_row(
    entry: PickerEntry,
    parent: Entity,
    tree: bool,
    theme: &UITheme,
    cmd: &mut CommandQueue,
) -> Entity {
    let mut row = cmd.spawn((
        theme
            .canvas()
            .fill(Color::TRANSPARENT)
            .radius_sm()
            .row()
            .gap(8.0)
            .padding(UIRect::axes(0.0, 8.0)),
        entry.pick,
        ChildOf::new(parent),
    ));
    if tree {
        row.insert(UITreeElement { expanded: true });
    }
    if let Some(search) = entry.search {
        row.insert(UISearchText(search));
    }
    if entry.current {
        row.insert(UISelected);
    }
    row.add_child(
        theme
            .label(entry.label)
            .font_size(theme.font_size_sm)
            .single_line()
            .grow()
            .shrink(1.0)
            .min_width(UIValue::Px(0.0)),
    );
    if let Some(detail) = entry.detail {
        row.add_child(theme.label(detail).muted().font_size(10.5).no_wrap());
    }
    row.entity()
}
