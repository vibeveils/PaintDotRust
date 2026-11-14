# PaintDotRust

PaintDotRust is a reimplementation of the OpenPDN engine (the C# port of Paint.NET 3.36 under
`port/`) in Rust, with a framework-free HTML UI. It is built to run in the
browser as WebAssembly, and natively for tests, benchmarks and development.

## Layout

| Path | What it is |
|---|---|
| `crates/openpdn-core` | Engine: surfaces, blend modes, compositing, selections, history, resampling, transforms, the 22 tools, 9 adjustments, 37 effects |
| `crates/openpdn-formats` | PNG (all variants, tEXt/zTXt/iTXt), `.openpdn`, legacy Paint.NET `.pdn` (MS-NRBF) |
| `crates/openpdn-editor` | Application model behind one JSON + binary message API (`Editor::handle`) |
| `crates/openpdn-wasm` | `wasm-bindgen` wrapper used by `web/worker.js` |
| `crates/openpdn-devserver` | Serves `web/` and runs the editor natively over a WebSocket (`/ws`) |
| `crates/openpdn-parity` | `.pdnscript` interpreter for parity tests and benchmarks |
| `web/` | UI: `index.html`, `js/` (menus, windows, WebGL2 view, dialogs, I/O), `worker.js` |
| `parity/` | C# runner, scripts, fixtures generators, benchmark scripts |

The UI never touches pixels itself. It sends requests (pointer events, menu
commands, dialog values) and receives state snapshots plus the composited
pixels of the region that changed, which it uploads into tiled WebGL textures.
In the browser the editor runs in a worker, so the page stays responsive.

## Running the UI without wasm

```sh
cargo run --release -p openpdn-devserver   # http://127.0.0.1:8090/
```

## Building the browser engine

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
./build-wasm.sh              # stable: single-threaded + SIMD128
./build-wasm.sh --threads    # nightly + rust-src: rayon on Web Workers
```

Then serve `web/` with any static server (threads need `Cross-Origin-Opener-Policy:
same-origin` and `Cross-Origin-Embedder-Policy: require-corp`; the dev server
sends both). The page uses the wasm worker by default; `?engine=ws` selects the
dev server.

## Hosting on GitHub Pages

The built `web/` folder is a static site: every path in it is relative, so it
works from a project subpath such as `https://<user>.github.io/PaintDotRust/`.

1. Run `./build-wasm.sh` (fills `web/pkg/`).
2. Publish the contents of `web/`, including `pkg/`. `web/.gitignore` excludes
   `pkg/`, so either publish from a build step or drop that line in the branch
   you serve from. `web/.nojekyll` stops Pages from running Jekyll over it.

GitHub Pages can't send the COOP/COEP headers that wasm threads need, so the
hosted app uses the single-threaded (stable) build. Settings and custom
palettes are kept in the browser per origin, so they are shared by all project
sites under the same `<user>.github.io` address (keys are prefixed `openpdn.`).

## Installing as an app

PaintDotRust is a progressive web app, so it can be installed from the
browser on phones, tablets and desktops (Help → Install App… explains how
where the browser has no install button). Installed, it opens in its own
window from the home screen, dock or app list, and works offline.

- `web/manifest.webmanifest`: name, icons (`web/icons/`), standalone display,
  a "New image" shortcut, file handling (opening images with the app on
  desktop) and a share target (sharing a picture to it from Android's share
  sheet).
- `web/sw.js`: the service worker. Network first, falling back to the saved
  copy after 4 s or offline, so a deploy reaches users on their next online
  visit with no version bump. When adding a file the app loads at start-up,
  add it to `SHELL` there so it is saved on install.
- Installing needs HTTPS (GitHub Pages provides it) or `localhost`.

## Parity with the C# port

`parity/run.sh` replays every script in `parity/scripts/` with the C# runner
(against `port/src/OpenPDN.Core` and `OpenPDN.Formats`) and the Rust
interpreter, then compares all outputs byte for byte. That covers every tool,
option, adjustment, effect and transform. It also checks PNG decoding against
Skia for every PNG variant, legacy `.pdn` reading, and `.openpdn` files
exchanged in both directions. Currently 244 of 244 outputs are identical.

The few .NET behaviours effects rely on are ported exactly: seeded
`System.Random`, `List<T>.Sort`'s introsort, and banker's rounding.

## Benchmarks

`parity/bench/*.pdnscript`, run by both engines with `--bench N`.
`OPENPDN_TRACE=1` prints per-command timings.

## Tests

```sh
cargo test --release            # unit, fuzz (corrupt files), editor scenarios
parity/run.sh                   # C# parity (needs the .NET SDK)
```
