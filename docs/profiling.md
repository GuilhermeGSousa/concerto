# Profiling the engine

Everything here uses open source tools. There are two complementary ways to
look at performance:

1. **Sampling profilers** (samply, perf, cargo-flamegraph) — no code changes,
   statistical view of *where CPU time goes*, including inside Jolt's C++ and
   wgpu-hal. Best for finding unknown hotspots.
2. **Instrumented tracing** (the [`profiling`](https://crates.io/crates/profiling)
   crate + [Tracy](https://github.com/wolfpld/tracy)) — per-frame timeline with
   named zones for every schedule and ECS system, across all worker threads.
   Best for understanding *frame structure*: spikes, scheduler stalls, sync
   points, fixed-update storms.

Both need symbols. The stock `release` profile strips them, so use the
`profiling` cargo profile (release speed + debug symbols):

```sh
cargo build --profile profiling -p render-test
```

## The benchmark scenes

| Workload | Command | What it stresses |
|---|---|---|
| Rendering (Sponza) | `cargo run --profile profiling -p render-test` | draw submission, materials, wgpu |
| Gameplay / animation | `cargo run --profile profiling -p tech-demo` | full stack: ECS, animation, physics, render |
| Physics | `cargo run --profile profiling -p physics-test` | Jolt step, transform write-back |
| UI | `cargo run --profile profiling -p ui-showcase`, then select the Stress tab | layout, text measurement, UI extraction |

For comparable captures: keep the window size fixed, don't move the camera,
let the scene warm up ~10 s, then capture ~30 s.

> **Vsync caveat:** the surface uses `surface_caps.present_modes[0]`
> (`crates/render/src/plugin.rs`), which is typically FIFO (vsync). When the
> engine is not CPU/GPU bound, threads sit in `present`/`get_current_texture`
> waits — that's idle time, not a hotspot. Judge CPU cost by zone times in
> Tracy rather than by frame rate until a present-mode override exists.

## Sampling profilers (zero instrumentation)

### samply (Linux, macOS, Windows — recommended first tool)

```sh
cargo install samply
cargo build --profile profiling -p render-test
samply record ./target/profiling/render-test
```

Opens the Firefox Profiler UI: per-thread timelines, flame graph, inverted
call tree. Worker threads show up by name (`compute-N`, `asset-load-N`).

### cargo-flamegraph / perf (Linux)

```sh
cargo install flamegraph
cargo flamegraph --profile profiling -p render-test
```

or raw perf:

```sh
perf record --call-graph dwarf ./target/profiling/render-test
perf report
```

If perf complains about permissions:
`echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid` (or run via sudo).

## Instrumented tracing with Tracy

The engine is instrumented with the `profiling` crate. The macros compile to
**nothing** unless a backend feature is enabled, so shipping builds pay zero
cost. Enable the Tracy backend with the root `tracy` feature:

```sh
cargo run --profile profiling --features tracy -p render-test
# examples forward the feature; equivalently:
cargo run --profile profiling -p tech-demo --features tracy
```

Then connect the Tracy server (GUI) to the running process. Install a Tracy
release that matches the `tracy-client-sys` protocol version in use — check
`cargo tree -p tracy-client-sys` and the compatibility table in the
[tracy-client README](https://github.com/nagisa/rust_tracy_client). At the
time of writing the workspace resolves `tracy-client-sys` 0.28.0, which
matches Tracy **v0.13.1**.

What you should see:

- **Frame marks** once per frame of the host's own `App` (i.e. after present).
  An `App` hosted inside another, such as the editor's edit `App`, does not
  mark frames.
- Zones: `App::update` → `schedule::update` / `schedule::render` / … →
  `system` zones, one per ECS system, running in parallel across the named
  `compute-N` threads. `fixed_update_step` zones tick at ~30 Hz.
- `apply_deferred` zones at sync points — long gaps right before them mean
  the scheduler is stalled waiting on one straggler system.
- wgpu-core's own zones (device/queue internals) appear automatically: wgpu
  is instrumented with the same `profiling` crate, and enabling our backend
  feature lights it up via feature unification.
- `jolt::step` / `jolt::write_back_transforms` (physics). Asset loading is
  async (scopes can't be held across `.await`), so loads show up as activity
  on the `asset-load-N` threads rather than as named zones — use Tracy's
  sampling or samply for detail there.

Per-system zones are runtime-named after the system itself (via
`profiling::scope!(sys.name())`, which goes through `tracy_client::span_alloc`
under the hood), so they show up individually — searchable and rankable by
name — in Tracy's *Statistics* / *Find Zone* views, rather than bucketed under
one static `system` zone.

Tracy also has its own sampling mode (call stacks on top of zones) — enable
"Sampling" in the Tracy server when capturing to combine both views.

### Overhead

Instrumentation cost with `--features tracy` is normally low single-digit
percent. If you need a number for your machine: run a scene for 60 s with and
without the feature and compare average frame time from the frame-stats log.

## Frame stats without any profiler

A `FrameStats` resource (inserted by `TimePlugin`) keeps a rolling window of
CPU frame times and logs a summary once per second (avg / p99 / worst, FPS):

```sh
RUST_LOG=info cargo run --profile profiling -p render-test
```

Apps using `DefaultPlugins` also get a small on-screen
frame-time overlay in the top-left corner (`FrameStatsOverlayPlugin` in the
`ui` crate).

## How to actually find "the code that needs improvement most"

1. Run samply on `render-test` and `tech-demo`. Anything unexpected dominating
   the inverted call tree is your first target.
2. Run the same scene with Tracy. Sort *Statistics* by total self time;
   inspect the frame timeline for spikes and for scheduler gaps (threads idle
   while one `system` zone runs long — a serialization problem, fixable with
   system reordering or splitting).
3. Cross-check: a hotspot samply sees but Tracy doesn't means missing spans
   *inside* a system — add finer `profiling::scope!`s there and re-measure.
4. Re-run after every change; keep captures comparable (same scene, camera,
   duration).

## Future work

- **GPU pass timing** via [`wgpu-profiler`](https://github.com/Wumpf/wgpu-profiler)
  (timestamp queries): request
  `adapter.features() & GpuProfiler::ALL_WGPU_TIMER_FEATURES` at device
  creation, wrap the render passes in `crates/render/src/material_plugin.rs`,
  resolve queries before submit in `RenderDevice::finish`. With its `tracy`
  feature the GPU appears as its own row in Tracy.
- Benchmark repeatability: present-mode override (e.g. `ENGINE_PRESENT_MODE`),
  fixed-camera flag in render-test, and a headless scheduler micro-benchmark
  built on `concerto::headless`.
