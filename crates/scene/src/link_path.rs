use crate::node_id::name_paths;
use crate::scene::Scene;

/// The path of every node of `scene`, by node index, relative to the scene's roots.
pub fn node_paths(scene: &Scene) -> Vec<String> {
    name_paths(&scene.nodes)
}

/// The path of the node named by `segment` under `parent`; an empty `parent` means a root.
pub fn child_path(parent: &str, segment: &str) -> String {
    match parent.is_empty() {
        true => segment.to_owned(),
        false => format!("{parent}/{segment}"),
    }
}

/// The rest of `path` below `prefix`, when `prefix` names whole leading segments of it.
pub fn strip_prefix(path: &str, prefix: &str) -> Option<String> {
    let path = segments(path);
    let prefix = segments(prefix);
    (path.len() > prefix.len() && path[..prefix.len()] == prefix[..])
        .then(|| path[prefix.len()..].join("/"))
}

fn segments(path: &str) -> Vec<&str> {
    if path.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (at, c) in path.char_indices() {
        match (escaped, c) {
            (true, _) => escaped = false,
            (false, '\\') => escaped = true,
            (false, '/') => {
                segments.push(&path[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    segments.push(&path[start..]);
    segments
}
