---
name: wasm-run
description: Use when running, debugging, or verifying a Concerto example in a browser, when the wasm32 build breaks, or when something works natively but fails on the web - no adapter found, missing font, asset 404, unsupported WebGL2 feature
---

# Running on wasm

The web target is WebGL2 through `trunk`. `cargo run --target wasm32-unknown-unknown`
is not the path.

## Run

```sh
cd examples/tech-demo
trunk serve
```

It serves at `http://127.0.0.1:8080`. `tech-demo` has a
`Trunk.toml` and an `index.html`; an example without them does not run on the
web yet.

The CI check is only that it builds:

```sh
cargo build --target wasm32-unknown-unknown
```

A build that passes proves nothing about the browser. A wasm change is verified
by loading the page and reading the console.

## What differs from native

| Area | On wasm |
|---|---|
| Graphics backend | `wgpu::Backends::GL` only, with `downlevel_webgl2` limits |
| Optional GPU features | Read from `crates/render/src/capabilities.rs`; never request one unconditionally |
| Physics | Rapier. Jolt is not compiled |
| Threads | None. The multithreaded executor is native only |
| Fonts | No system fonts; the UI embeds DejaVu Sans Mono for every family |
| Assets | Fetched over HTTP from the page's origin; `index.html` copies `content/` with a `copy-dir` link |
| File system | None. Anything that opens a path must fetch bytes instead |

## Common mistakes

| Symptom | Cause |
|---|---|
| "no adapter" at start | Something requested a backend or feature WebGL2 lacks |
| Asset requests return 404 | `content/` is not copied into `dist/`; check the `copy-dir` link |
| "no default font found" | A font path that relies on system fonts |
| `cargo run` on the wasm target fails looking for `index.html` | The `wasm-server-runner` entry in `.cargo/config.toml` is legacy; use `trunk` |

`trunk` reads one thing from `.cargo/config.toml`: the `+bulk-memory` target
feature. Leave it.
