use concerto_reflect::{PathSegment, PropertyPath, Reflect, ReflectRef, with_property};

use super::{Property, PropertyKind, write::WriteRefusal};

/// How many elements of a list `inspect` shows before a note stands for the rest.
pub(crate) const LIST_LIMIT: usize = 16;

pub(crate) struct ListedPath<'a> {
    pub(crate) path: PropertyPath,
    pub(crate) row: Option<&'a Property>,
    pub(crate) names_enum: bool,
}

impl<'a> ListedPath<'a> {
    /// The row whose editor writes this path, or `None` when `set_property` writes it whole: an enum, a list, or a path no editor claims.
    pub(crate) fn adapter(&self) -> Option<&'a Property> {
        self.row.filter(|property| {
            !self.names_enum && property.list_length().is_none() && property.value().is_some()
        })
    }
}

pub(crate) fn listed_paths<'a>(value: &dyn Reflect, rows: &'a [Property]) -> Vec<ListedPath<'a>> {
    let mut listed: Vec<ListedPath> = Vec::new();
    for row in rows
        .iter()
        .filter(|row| !matches!(row.kind, PropertyKind::Heading))
    {
        let segments = row.path.segments();
        for depth in 0..segments.len() {
            let above = PropertyPath::new(segments[..depth].iter().copied());
            if names_enum(value, &above) && listed.iter().all(|listed| listed.path != above) {
                listed.push(ListedPath {
                    path: above,
                    row: None,
                    names_enum: true,
                });
            }
        }
        listed.push(ListedPath {
            path: row.path.clone(),
            row: Some(row),
            names_enum: names_enum(value, &row.path),
        });
    }
    listed
}

/// One entry of a component's `paths`: a listed path, or the note that stands for a long list's cut elements.
pub(crate) enum ShownPath<'a> {
    Listed(&'a ListedPath<'a>),
    Cut { list: PropertyPath, total: usize },
}

/// The paths `inspect` shows: each list's first [`LIST_LIMIT`] elements, then a note in place of the rest.
pub(crate) fn shown_paths<'a>(listed: &'a [ListedPath<'a>]) -> Vec<ShownPath<'a>> {
    let mut shown: Vec<ShownPath> = Vec::new();
    for entry in listed {
        let segments = entry.path.segments();
        let Some((depth, index)) =
            segments
                .iter()
                .enumerate()
                .find_map(|(depth, segment)| match segment {
                    PathSegment::Index(index) if *index >= LIST_LIMIT => Some((depth, *index)),
                    _ => None,
                })
        else {
            shown.push(ShownPath::Listed(entry));
            continue;
        };
        let list = PropertyPath::new(segments[..depth].iter().copied());
        match shown.last_mut() {
            Some(ShownPath::Cut { list: last, total }) if *last == list => *total = index + 1,
            _ => shown.push(ShownPath::Cut {
                list,
                total: index + 1,
            }),
        }
    }
    shown
}

/// What a [`ShownPath::Cut`] says, in the form `value`'s cut list does.
pub(crate) fn cut_note(list: &PropertyPath, total: usize) -> String {
    format!("… {} more of {list}, {total} in all", total - LIST_LIMIT)
}

fn names_enum(value: &dyn Reflect, path: &PropertyPath) -> bool {
    let mut found = false;
    let _ = with_property(value, path, &mut |field| {
        found = matches!(field.reflect_ref(), ReflectRef::Enum(_));
    });
    found
}

fn shown_path(path: &PropertyPath) -> String {
    path.to_string()
}

pub(crate) fn find_path<'a>(
    component: &str,
    path: &str,
    listed: &'a [ListedPath<'a>],
) -> Result<&'a ListedPath<'a>, WriteRefusal> {
    listed
        .iter()
        .find(|listed| shown_path(&listed.path) == path)
        .ok_or_else(|| WriteRefusal::unknown_path(component, path, listed))
}

impl WriteRefusal {
    pub(crate) fn unknown_path(component: &str, path: &str, listed: &[ListedPath]) -> Self {
        if listed.is_empty() {
            return Self::new(
                "unknown_path",
                format!(
                    "{component} has no reflection data the editor can write, so it has no paths; inspect shows it unavailable."
                ),
            );
        }
        let paths: Vec<String> = shown_paths(listed)
            .iter()
            .map(|shown| match shown {
                ShownPath::Listed(listed) if listed.path.depth() == 0 => "\"\"".to_owned(),
                ShownPath::Listed(listed) => shown_path(&listed.path),
                ShownPath::Cut { list, total } => cut_note(list, *total),
            })
            .collect();
        Self::new(
            "unknown_path",
            format!(
                "{component} has no path `{path}`; its paths are {}.",
                paths.join(", ")
            ),
        )
    }
}
