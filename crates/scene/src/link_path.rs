use std::collections::HashMap;

use crate::scene::Scene;

/// The path of every node of `scene`, by node index, relative to the scene's roots.
pub fn node_paths(scene: &Scene) -> Vec<String> {
    let mut is_child = vec![false; scene.nodes.len()];
    for node in &scene.nodes {
        for &child in &node.children {
            if let Some(flag) = is_child.get_mut(child) {
                *flag = true;
            }
        }
    }
    let roots: Vec<usize> = (0..scene.nodes.len()).filter(|&i| !is_child[i]).collect();
    let mut paths: Vec<Option<String>> = vec![None; scene.nodes.len()];
    let mut pending = vec![PendingSiblings {
        parent: String::new(),
        nodes: roots,
    }];
    while let Some(group) = pending.pop() {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for index in group.nodes {
            let Some(node) = scene.nodes.get(index) else {
                continue;
            };
            let count = seen.entry(node.name.as_str()).or_insert(0);
            let segment = match (node.name.is_empty(), *count) {
                (true, n) => format!("#{n}"),
                (false, 0) => escape(&node.name),
                (false, n) => format!("{}#{n}", escape(&node.name)),
            };
            *count += 1;
            if paths[index].is_some() {
                continue;
            }
            let path = child_path(&group.parent, &segment);
            pending.push(PendingSiblings {
                parent: path.clone(),
                nodes: node.children.clone(),
            });
            paths[index] = Some(path);
        }
    }
    paths.into_iter().map(Option::unwrap_or_default).collect()
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

struct PendingSiblings {
    parent: String,
    nodes: Vec<usize>,
}

fn escape(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '/' | '#' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
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
