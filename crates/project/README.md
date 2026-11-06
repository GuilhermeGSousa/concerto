# cargo concerto

The project tool. It creates Concerto game projects, adds the editor crate to
an existing game, and builds a game with its derived data cooked. It is a Cargo
subcommand, a binary named `cargo-concerto`.

```sh
cargo install --path crates/project
cargo concerto new <dir> [--engine <path>]
cargo concerto editor <dir> [--engine <path>]
cargo concerto editor <dir> --update [--engine <path>]
cargo concerto build [<dir>] [--cook] [--release] [--target wasm32-unknown-unknown] [--public-url <url>] [--dist <dir>]
```

## `new`

Creates `<dir>` and a game in it, then does what `editor` does. It refuses a
directory that exists and is not empty. The package is named after the
directory; a name Cargo would reject (anything but letters, digits, `-` and
`_`, a leading digit, a Rust keyword, `test`, or a Cargo build directory name:
`build`, `deps`, `examples`, `incremental`) is refused, and so is the name of an
engine crate the project depends on (`concerto`, `concerto-editor`,
`concerto-asset-build`, `concerto-animation-editor`).

It writes:

- `Cargo.toml`: the game package, with a `[lib]` of the default crate type, a
  `[[bin]]` named after the package, and a `[package.metadata.concerto.editor]`
  table listing `concerto-animation-editor`, so the project's editor has the
  clip editor and calls `run_with`; see Editor plugins.
- `src/lib.rs`: the recipe, `pub fn game(app: &mut App)`, which registers
  `DefaultPlugins`.
- `src/main.rs`: `concerto::run(<crate>::game, concerto::content_registry!())`.
  `content_registry!()` embeds the asset registry and the project root, and a
  native game keeps derived data in `<project>/derived-cache/`.
- `build.rs`: tracks `content/`, writes the asset registry the binary embeds,
  and copies `content/` next to the binary.
- `content/.gitkeep`, so `content/` exists.
- `index.html` and `Trunk.toml`, so `trunk serve` runs the game in a browser.
- `.gitignore`: `/target`, `/dist`, `content/.registry.toml`, so a registry
  file from an older engine is never committed, and `derived-cache/`, the
  local cache of derived data.

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
`<crate>::game`. Run the editor with `cargo run -p <game>-editor`. A game that
lists editor plugins gets a `run_with` call instead; see Editor plugins. The
editor crate has a `tracy` feature that turns on `concerto-editor/tracy`; see
[profiling.md](../../docs/profiling.md).

If `editor` fails after writing `editor/`, it removes that directory, so the
command can be run again once the cause is fixed. If `new` fails after writing,
for example because the enclosing workspace's `members` is not an array, it
removes the project directory it created, or empties the one it was given.

## Editor plugins

A game names the crates that extend its editor in its own `Cargo.toml`. They
are data there, so the game gains no dependency on them:

```toml
[package.metadata.concerto.editor]
plugins = [{ package = "concerto-motion-matching-editor", install = "concerto_motion_matching_editor::install" }]
```

`editor` writes each package into `editor/Cargo.toml` and calls `run_with`
instead of `run`:

```rust
fn main() {
    concerto_editor::run_with(
        my_game::game,
        concat!(env!("CARGO_MANIFEST_DIR"), "/.."),
        &[concerto_motion_matching_editor::install],
    );
}
```

With no list, or an empty one, the generated files are as above.

`new` writes one entry, the clip editor, and a project that doesn't want it
removes it and runs `editor --update`:

```toml
[package.metadata.concerto.editor]
plugins = [
    { package = "concerto-animation-editor", install = "concerto_animation_editor::install" },
]
```

`editor` adds nothing to a game's list, so a game without that entry has no clip
tab until it adds the entry and runs `editor --update`.

An entry has these keys, and no others:

- `package`: the Cargo package. It cannot be the game or `concerto-editor`, and
  it appears once.
- `install`: the `fn(&mut App)` to call, as a Rust path of at least two
  segments.
- `version`, `path`: where the package comes from. A `path` is relative to the
  game's `Cargo.toml`, and is rewritten for `editor/Cargo.toml`.

A package named `concerto-<dir>` with neither `version` nor `path` is the engine
crate `crates/<dir>`, and resolves as `concerto-editor` does; see Engine
dependencies. Any other package must give `version` or `path`.

## `editor --update`

Rewrites `editor/Cargo.toml` and `editor/src/main.rs` from the game's list, and
nothing else, so run it after changing the list. Anything written by hand in
those two files is lost. It refuses a missing `editor/`.

Its engine is the one `editor/Cargo.toml` already names, so it never swaps a
path dependency for the published crate; `--engine` replaces it. It refuses an
`editor/Cargo.toml` it can't read an engine from, unless `--engine` is given:
one with no `concerto-editor`, one that inherits it from the workspace, one that
takes it from a git repository or another registry, or one whose path doesn't
end in `crates/editor`.

`editor` and `editor --update` validate the whole list before writing anything.
A malformed entry is quoted in the error.

## `build`

Builds the game in `<dir>`, `.` by default. Without `--cook` it is a plain
build: `cargo build` of the game's binary natively, or `trunk build` with
`--target wasm32-unknown-unknown`. With `--cook` it also cooks the game's
derived data and stages the cooked files over the build, so the game applies
each derived blob at load instead of deriving it.

The package is the `[package] name` in `<dir>/Cargo.toml`, and the binary is
the bin target of the same name. The target directory is the one
`cargo metadata` reports. The `cargo` it runs is `$CARGO` when set, so under
`cargo concerto` it is the toolchain that ran the command.

The steps, each run only when the options call for it:

1. Cargo: `cargo build --manifest-path <dir>/Cargo.toml -p <pkg> --bin <pkg>`,
   with `--release` when given. Diagnostics stream as usual; the executable is
   the one cargo reports for that bin. A web cook builds this native binary
   too, in the same profile, because the cook runs natively.
2. Cook: runs `<exe> --cook <target>/cooked/<pkg>`. Every game that calls
   `concerto::run` accepts `--cook <out>`: it builds the game's `App` with no
   window and no GPU and writes each derived asset to `<out>/<address>`, its
   committed file plus the derived blob. A derive that fails fails the build,
   naming the asset. The cook refuses an `<out>` that is not empty and was not
   written by an earlier cook, which leaves a `.concerto-cook` file in it.
3. Trunk: `trunk build` in `<dir>`, with `--release` and `--public-url <url>`
   when given, and always `--dist <dist>`. `<dist>` is `--dist` resolved
   against the working directory, or `<dir>/dist`. `trunk` must be on `PATH`;
   install it with `cargo install trunk`. A web build checks for it before
   any step runs, so a missing `trunk` fails at once rather than after the
   native build and cook.
4. Overlay: copies each cooked file over the file at the same relative path,
   in `<exe dir>/<exe stem>-content/` natively, beside the executable cargo
   reported, so a `build.target` in cargo's config is followed, and in
   `<dist>` for the web. Only files the build already has are replaced; a
   cooked asset the build doesn't ship, such as one the web build's
   `index.html` doesn't copy, is skipped and listed as a note.

A step that fails stops the build and names its command line and exit status.
The report lists each cooked file as written and each replaced file as edited.

The staged files cannot go stale. They only exist in a build directory or dist
the command just built. A later plain `cargo build` that changed `content/`
reruns `build.rs`, which copies the raw content back over the cooked files, and
the game derives at load again; one that changed nothing leaves the cooked
files, which still match.

## Engine dependencies

Without `--engine`, each engine dependency is the tool's own version:

```toml
concerto = "0.1.0"
```

That form builds only against a release on crates.io that contains this work,
and the published 0.1.0 does not, so the tool warns when `--engine` is absent.

With `--engine <path>`, a Concerto checkout resolved against the working
directory, each dependency also names its own crate in that checkout: `.` for
`concerto`, `crates/asset-build` for `concerto-asset-build`, `crates/editor` for
`concerto-editor`, and `crates/<dir>` for a plugin package `concerto-<dir>`.

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
  game's `Cargo.toml`, and `new` writes that table into the manifest it
  creates, with a `dev` profile at `opt-level = 1` that builds every
  dependency, the engine included, at `opt-level = 3`.
