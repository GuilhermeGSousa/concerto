import json
import os
import subprocess
import sys
from collections import deque

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
ALLOWED = {
    "engine": {"engine"},
    "gizmos": {"engine", "gizmos"},
    "editor": {"engine", "gizmos", "editor"},
    "example": {"engine", "gizmos"},
}
ADVICE = {
    "engine": "engine crates may depend only on engine crates. Move the shared code into an engine crate.",
    "gizmos": "gizmos crates may depend only on engine and gizmos crates. Move the shared code into an engine or gizmos crate.",
    "example": "examples may depend only on engine and gizmos crates. Move what the example needs from the editor into an engine crate.",
}
ROOT_PACKAGE = "concerto"
JOLT = "concerto-jolt-ffi"
WASM = "wasm32-unknown-unknown"


class CargoFailed(Exception):
    pass


def metadata(*extra):
    command = ["cargo", "metadata", "--locked", "--format-version", "1", *extra]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise CargoFailed(f"{' '.join(command)} failed:\n{result.stderr.strip()}")
    return json.loads(result.stdout)


def layer(package, examples):
    name = package["name"]
    if os.path.realpath(package["manifest_path"]).startswith(examples):
        return "example"
    if name == "concerto-editor" or (name.startswith("concerto-") and name.endswith("-editor")):
        return "editor"
    if name.startswith("concerto-") and name.endswith("-gizmos"):
        return "gizmos"
    return "engine"


def members(data):
    ids = set(data["workspace_members"])
    return {package["id"]: package for package in data["packages"] if package["id"] in ids}


def layer_violations(data):
    examples = os.path.join(os.path.realpath(data["workspace_root"]), "examples") + os.sep
    workspace = members(data)
    wrong = set()
    for node in data["resolve"]["nodes"]:
        package = workspace.get(node["id"])
        if package is None:
            continue
        own = layer(package, examples)
        for dep in node["deps"]:
            target = workspace.get(dep["pkg"])
            if target is None:
                continue
            theirs = layer(target, examples)
            if theirs not in ALLOWED[own]:
                wrong.add(f"{package['name']} ({own}) depends on {target['name']} ({theirs}): {ADVICE[own]}")
    return sorted(wrong)


def wasm_violations(data):
    names = {package["id"]: package["name"] for package in data["packages"]}
    nodes = {node["id"]: node for node in data["resolve"]["nodes"]}
    start = next((id for id, package in members(data).items() if package["name"] == ROOT_PACKAGE), None)
    if start is None:
        return [f"The workspace has no package named {ROOT_PACKAGE}, which the wasm32 check starts from. Update ROOT_PACKAGE in this script."]
    parent = {start: None}
    queue = deque([start])
    while queue:
        current = queue.popleft()
        if names[current] == JOLT:
            chain = []
            while current is not None:
                chain.append(names[current])
                current = parent[current]
            path = " -> ".join(reversed(chain))
            return [f"{JOLT} is in the wasm32 graph: {path}. Make it a dependency only for non-wasm targets, or put it behind a feature that wasm builds do not enable."]
        for dep in nodes[current]["deps"]:
            normal = any(kind["kind"] is None for kind in dep["dep_kinds"])
            if normal and dep["pkg"] not in parent:
                parent[dep["pkg"]] = current
                queue.append(dep["pkg"])
    return []


def main():
    try:
        wrong = layer_violations(metadata()) + wasm_violations(metadata("--filter-platform", WASM))
    except CargoFailed as error:
        print(error, file=sys.stderr)
        return 2
    for line in wrong:
        print(line)
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())
