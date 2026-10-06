# STILL LIFE

A short first-person horror game made with Concerto. You are an auctioneer's
clerk cataloguing a dead painter's house in 1893, and his lay figures only
move when you are not looking. Design notes live in [DESIGN.md](DESIGN.md).
The crate is still called `after-hours`, after the game's first version.

## Play it locally

The game targets the browser. You need the wasm target and
[Trunk](https://trunkrs.dev):

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cd games/after-hours
trunk serve --release          # then open http://127.0.0.1:8080
```

It also builds and runs natively (`cargo run -p after-hours --release`), but
native audio is not implemented yet, so the desktop build is silent.

## Build for itch.io

```sh
cd games/after-hours
trunk build --release --public-url ./
cd dist && zip -r -9 ../after-hours-web.zip .
```

`--public-url ./` keeps every URL relative, which itch.io needs because it
serves HTML games from a sub-path. The build is about 15 MB (8 MB of wasm,
3 MB gzipped, plus 6 MB of mannequin animation data).

The `after-hours-web` GitHub Actions workflow does the same on pull requests
and uploads the zip as an artifact. It can also publish straight to itch.io
(see "Publishing" below).

### itch.io project settings

- **Kind of project:** HTML. Upload `after-hours-web.zip` and tick
  *This file will be played in the browser*.
- **Viewport:** 1280 × 720, with *Fullscreen button* enabled. Leave
  *Mobile friendly* off (it needs a mouse and keyboard).
- **Frame options:** enable *Click to launch in fullscreen* if you like; the
  game asks for pointer lock on the first click either way.
- **SharedArrayBuffer support:** not needed.
- Suggested page copy, tags and content warnings are in
  [ITCH_PAGE.md](ITCH_PAGE.md).

### Publishing from CI

Add to the GitHub repository:

- a secret `BUTLER_API_KEY` (from https://itch.io/user/settings/api-keys), and
- a variable `ITCH_TARGET`, e.g. `your-itch-username/after-hours`.

Then either push a tag like `after-hours-v0.2.0`, or run the workflow by hand
with *publish* ticked. It pushes to the `html5` channel with butler.

## How the content is shipped

The mannequin comes from the Universal Animation Library import in
`examples/tech-demo/content/UAL1`. The game ships only what it uses:
`content-manifest.txt` lists those files, and

- `build.rs` turns each line into an `AssetId` constant (`src/content.rs`)
  and stages the files, with a registry trimmed to them, next to the native
  binary;
- `stage-content.sh` (a Trunk post-build hook) does the same for the web.

To use another clip, add a line to the manifest and reference its constant.

## Debugging and automated testing

URL query flags (native: `AFTER_HOURS_DEBUG=flag1,flag2`):

| Flag | Effect |
|------|--------|
| `nolock` | Treat the pointer as locked, so play never pauses (headless browsers cannot lock it). |
| `noui` | Hide every screen, for clean screenshots. |
| `nohunt` | Figures never move. |
| `trace` | Log camera, player and hunter state. |
| `seed=N` | Fix the first night's house. |
| `night=N` | Start at night `N`. |
| `room=N` | Start each night in a corner of room `N` (the log names each room). |
| `escape` | Treat every lot as catalogued, to test the night flow. |
| `poses`, `poses0`–`poses2` | Replace the title house with a gallery of the pose library. |

Phase changes (`phase: Title -> Intro (night 1)`) and night builds are
always logged to the browser console, which makes the game easy to drive
from Playwright: wait for a log line, act, screenshot.

## Credits

- Engine: Concerto.
- Mannequin model and animations: Universal Animation Library by
  Quaternius (CC0), https://quaternius.com.
- Font: IM FELL English by Igino Marini (SIL Open Font License, see
  `fonts/OFL.txt`).
- Everything else (geometry, textures, sound and music) is generated in code.
