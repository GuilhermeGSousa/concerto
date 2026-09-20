use crate::{Editable, PropertyVisitor, PropertyVisitorMut};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PropertyPath(Vec<&'static str>);

impl PropertyPath {
    pub fn new(segments: impl IntoIterator<Item = &'static str>) -> Self {
        Self(segments.into_iter().collect())
    }

    pub fn segments(&self) -> &[&'static str] {
        &self.0
    }

    pub fn name(&self) -> &'static str {
        self.0.last().copied().unwrap_or("")
    }

    pub fn depth(&self) -> usize {
        self.0.len()
    }

    pub fn get_depth(&self, index: usize) -> Option<&'static str> {
        self.0.get(index).copied()
    }
}

/// The requested field is absent. An empty path always addresses the root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathError {
    NotFound,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("property path not found")
    }
}
impl std::error::Error for PathError {}

/// Access a leaf, composite value, or root without retaining a borrowed value.
pub fn with_property(
    root: &dyn Editable,
    path: &PropertyPath,
    callback: &mut dyn FnMut(&dyn Editable),
) -> Result<(), PathError> {
    access(root, path.segments(), callback)
}

fn access(
    value: &dyn Editable,
    segments: &[&'static str],
    callback: &mut dyn FnMut(&dyn Editable),
) -> Result<(), PathError> {
    let Some((name, rest)) = segments.split_first() else {
        callback(value);
        return Ok(());
    };
    struct Access<'a> {
        name: &'static str,
        rest: &'a [&'static str],
        callback: &'a mut dyn FnMut(&dyn Editable),
        result: Option<Result<(), PathError>>,
    }
    impl PropertyVisitor for Access<'_> {
        fn field(&mut self, name: &'static str, value: &dyn Editable) {
            if self.result.is_none() && name == self.name {
                self.result = Some(access(value, self.rest, self.callback));
            }
        }
    }
    let mut visitor = Access {
        name,
        rest,
        callback,
        result: None,
    };
    value.visit(&mut visitor);
    visitor.result.unwrap_or(Err(PathError::NotFound))
}

/// Mutably access a leaf, composite value, or root. The callback owns any
/// validation policy; this function only resolves the path.
pub fn with_property_mut(
    root: &mut dyn Editable,
    path: &PropertyPath,
    callback: &mut dyn FnMut(&mut dyn Editable),
) -> Result<(), PathError> {
    access_mut(root, path.segments(), callback)
}

fn access_mut(
    value: &mut dyn Editable,
    segments: &[&'static str],
    callback: &mut dyn FnMut(&mut dyn Editable),
) -> Result<(), PathError> {
    let Some((name, rest)) = segments.split_first() else {
        callback(value);
        return Ok(());
    };
    struct Access<'a> {
        name: &'static str,
        rest: &'a [&'static str],
        callback: &'a mut dyn FnMut(&mut dyn Editable),
        result: Option<Result<(), PathError>>,
    }
    impl PropertyVisitorMut for Access<'_> {
        fn field(&mut self, name: &'static str, value: &mut dyn Editable) {
            if self.result.is_none() && name == self.name {
                self.result = Some(access_mut(value, self.rest, self.callback));
            }
        }
    }
    let mut visitor = Access {
        name,
        rest,
        callback,
        result: None,
    };
    value.visit_mut(&mut visitor);
    visitor.result.unwrap_or(Err(PathError::NotFound))
}
