use crate::{PropertyVisitor, PropertyVisitorMut, Reflect, ReflectMut, ReflectRef};

/// One step of a [`PropertyPath`]: a field by name, tuple fields being `"0"`, `"1"`, …, or a list element by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSegment {
    Field(&'static str),
    Index(usize),
}

impl From<&'static str> for PathSegment {
    fn from(name: &'static str) -> Self {
        Self::Field(name)
    }
}

impl From<usize> for PathSegment {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl PartialEq<&str> for PathSegment {
    fn eq(&self, other: &&str) -> bool {
        matches!(self, Self::Field(name) if name == other)
    }
}

impl std::fmt::Display for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Field(name) => f.write_str(name),
            Self::Index(index) => write!(f, "{index}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PropertyPath(Vec<PathSegment>);

impl PropertyPath {
    pub fn new(segments: impl IntoIterator<Item = impl Into<PathSegment>>) -> Self {
        Self(segments.into_iter().map(Into::into).collect())
    }

    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// The last field's name, or `""` when no segment is a field.
    pub fn name(&self) -> &'static str {
        self.0
            .iter()
            .rev()
            .find_map(|segment| match segment {
                PathSegment::Field(name) => Some(*name),
                PathSegment::Index(_) => None,
            })
            .unwrap_or("")
    }

    pub fn depth(&self) -> usize {
        self.0.len()
    }

    pub fn get_depth(&self, index: usize) -> Option<PathSegment> {
        self.0.get(index).copied()
    }
}

impl std::fmt::Display for PropertyPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (depth, segment) in self.0.iter().enumerate() {
            if depth > 0 {
                f.write_str(".")?;
            }
            write!(f, "{segment}")?;
        }
        Ok(())
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
    root: &dyn Reflect,
    path: &PropertyPath,
    callback: &mut dyn FnMut(&dyn Reflect),
) -> Result<(), PathError> {
    access(root, path.segments(), callback)
}

fn access(
    value: &dyn Reflect,
    segments: &[PathSegment],
    callback: &mut dyn FnMut(&dyn Reflect),
) -> Result<(), PathError> {
    let Some((segment, rest)) = segments.split_first() else {
        callback(value);
        return Ok(());
    };
    let name = match *segment {
        PathSegment::Field(name) => name,
        PathSegment::Index(index) => {
            let ReflectRef::List(list) = value.reflect_ref() else {
                return Err(PathError::NotFound);
            };
            let element = list.get(index).ok_or(PathError::NotFound)?;
            return access(element, rest, callback);
        }
    };
    struct Access<'a> {
        name: &'static str,
        rest: &'a [PathSegment],
        callback: &'a mut dyn FnMut(&dyn Reflect),
        result: Option<Result<(), PathError>>,
    }
    impl PropertyVisitor for Access<'_> {
        fn field(&mut self, name: &'static str, value: &dyn Reflect) {
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
    root: &mut dyn Reflect,
    path: &PropertyPath,
    callback: &mut dyn FnMut(&mut dyn Reflect),
) -> Result<(), PathError> {
    access_mut(root, path.segments(), callback)
}

fn access_mut(
    value: &mut dyn Reflect,
    segments: &[PathSegment],
    callback: &mut dyn FnMut(&mut dyn Reflect),
) -> Result<(), PathError> {
    let Some((segment, rest)) = segments.split_first() else {
        callback(value);
        return Ok(());
    };
    let name = match *segment {
        PathSegment::Field(name) => name,
        PathSegment::Index(index) => {
            let ReflectMut::List(list) = value.reflect_mut() else {
                return Err(PathError::NotFound);
            };
            let element = list.get_mut(index).ok_or(PathError::NotFound)?;
            return access_mut(element, rest, callback);
        }
    };
    struct Access<'a> {
        name: &'static str,
        rest: &'a [PathSegment],
        callback: &'a mut dyn FnMut(&mut dyn Reflect),
        result: Option<Result<(), PathError>>,
    }
    impl PropertyVisitorMut for Access<'_> {
        fn field(&mut self, name: &'static str, value: &mut dyn Reflect) {
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
