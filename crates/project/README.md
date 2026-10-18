# cargo concerto

The project tool. It creates Concerto game projects and adds the editor crate
to an existing game. It is a Cargo subcommand, a binary named `cargo-concerto`.

```sh
cargo install --path crates/project
cargo concerto new <dir> [--engine <path>]
cargo concerto editor <dir> [--engine <path>]
```

## `new`

Creates `<dir>` and a game in it, then does what `editor` does. It refuses a
directory that exists and is not empty. The package is named after the
directory; a name Cargo would reject (anything but letters, digits, `-` and
`_`, a leading digit, a Rust keyword, `test`, or a Cargo build directory name:
`build`, `deps`, `examples`, `incremental`) is refused, and so is the name of an
engine crate the project depends on (`concerto`, `concerto-editor`,
`concerto-asset-build`).

It writes:

- `Cargo.toml`: the game package, with a `[lib]` of the default crate type and
  a `[[bin]]` named after the package.
- `src/lib.rs`: the recipe, `pub fn game(app: &mut App)`, which registers
  `DefaultPlugins`.
- `src/main.rs`: `concerto::run(<crate>::game)`.
- `build.rs`: tracks `content/` and copies it next to the binary.
- `content/.registry.toml`: an empty `[assets]` table, so `content/` exists.
- `index.html` and `Trunk.toml`, so `trunk serve` runs the game in a browser.
- `.gitignore`: `/target` and `/dist`.

There is no scene and no startup scene.

## `editor`

Adds `<dir>/editor/`, a crate named `<game>-editor` that depends on the game
and on `concerto-editor`. Its `main` is one call:

```rust
fn main() {
    concerto_editor::run(my_game::game, concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
}
```

So the editor opens its own game's project by default; `--project <dir>`
overrides that. `editor` requires `<dir>/Cargo.toml` and `<dir>/src/lib.rs`,
refuses if `<dir>/editor` exists, and says that it assumes the recipe is
`<crate>::game`. Run the editor with `cargo run -p <game>-editor`.

If `editor` fails after writing `editor/`, it removes that directory, so the
command can be run again once the cause is fixed. If `new` fails after writing,
for example because the enclosing workspace's `members` is not an array, it
removes the project directory it created, or empties the one it was given.

## Engine dependencies

Without `--engine`, each engine dependency is the tool's own version:

```toml
concerto = "0.1.0"
```

That form builds only against a release on crates.io that contains this work,
and the published 0.1.0 does not, so the tool warns when `--engine` is absent.

With `--engine <path>`, a Concerto checkout resolved against the working
directory, each dependency also names its own crate in that checkout: `.` for
`concerto`, `crates/asset-build` for `concerto-asset-build`, and
`crates/editor` for `concerto-editor`.

```toml
concerto = { version = "0.1.0", path = "../.." }
```

The path is relative to the manifest it is written into when the project lies
inside the checkout, and absolute otherwise. Cargo uses the path to build and
the version to publish.

## Workspace edits

The tool makes the crates it creates members of the workspace they belong to,
editing that manifest with `toml_edit` so its formatting and comments are kept,
and lists every manifest it edited. Those manifests are the only files it
writes that it did not create.

- The workspace is the game's own `Cargo.toml` if it has a `[workspace]`
  table, and otherwise the first `Cargo.toml` with one found walking up from
  the game's parent directory.
- A member pattern that already matches a crate leaves it alone.
- If a member glob of the form `P/*` covers the game, the editor crate is added
  as `P/*/editor`; in this repository that is `"examples/*/editor"`. Otherwise
  the crate's own path is added.
- A game under the workspace's `exclude`, or under no workspace, is its own
  workspace: `editor` appends `[workspace] members = [".", "editor"]` to the
  game's `Cargo.toml`, and `new` writes that table, with
  `[profile.dev.package.concerto-jolt-ffi] opt-level = 3`, into the manifest it
  creates.
