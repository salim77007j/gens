# Gens Engine v1.1 — The Interactive Editor

**Release theme:** the one piece v1.0.0 honestly deferred — a real OS window. v1.1 ships the interactive editor shell: you double-click `gens-editor`, a window opens, the 3D scene renders live, objects drag, the toolbar plays the simulation, and everything the window does is verified on CI on every push.

---

## What the interactive editor is

A single binary — `gens-editor` — built on the same shell, renderer, and command executor that the headless build has proven on CI since Phase 7. No flags: it opens a resizable window (winit 0.30) and presents every frame through the engine's own wgpu pipeline. What is inside the window:

| Capability | Where | Notes |
|---|---|---|
| Real OS window | winit 0.30 `ApplicationHandler` | Resizable; vsync-paced continuous redraw |
| Live 3D viewport | the Phase-7 viewport, presented to the window | Same composition path as every CI screenshot — what you see is what CI proved |
| Toolbar | Play / Pause / Stop / Save / Load | Buttons queue intent; the shared executor executes (identical code to the headless example) |
| Menu bar | File / Edit / GameObject / View / Help | All items execute real commands; decoration is absent |
| Keyboard shortcuts | Ctrl+S save, Ctrl+O load, Ctrl+Z undo, Ctrl+D duplicate, Del delete | Suppressed while typing in a text field (except save/load) |
| Scene View picking | click to select, exact geometry | Pointer ray unprojected through the inverse view_proj; OBB slab test for cubes/markers, bounded-plane test for quads — no bounding-sphere approximations |
| Scene View dragging | drag an object to move it | Moves on the camera-facing plane through the object; the physics body follows so Play never snaps it back. Drag empty space to orbit. Mid-simulation drags are disabled (the simulation owns the transform) — by design, stated in Help |
| Inspector | Transform / Mesh / Physics | Editable since Phase 7; edits reach the real components the simulation runs |
| Play mode | Play captures a pre-play snapshot; Stop restores it exactly | Fixed 60 Hz steps on a wall-clock accumulator (capped); physics + transform sync run in play mode |
| Undo | transform edits, deletes, creates | Honest scope: restored entities get NEW entity ids (documented in Help/ADR-0036); redo deliberately not offered (id remapping is future work) |
| Console | `help`, `entities`, `validate`, `stats`, `clear` | The Phase-8 command row, live in the window |

## What was already there (v1.0.0 and phases 0–10)

The engine underneath is unchanged by v1.1 — 20 crates, 34+ ADRs, deterministic replays, physics (rapier), scripting (rhai), animation, networking (QUIC), audio, the build pipeline (`gens-pkg`/`gens-player` and `.genspkg` packages), and the headless editor whose full chrome (hierarchy, inspector, asset browser, console, view tabs) v1.1 now presents in a window. v1.0.0's release notes (the Phase 10 report) remain accurate for everything except the "no window shell" boundary, which this release retires.

## How to use it

1. Download the zip for your platform below and unpack it.
2. Run `gens-editor` (Linux/macOS) or `gens-editor.exe` (Windows).
3. In the window:
   - click **Play** — the Crate falls under gravity; **Stop** puts it back exactly where it was;
   - click any object to select it; drag it to move; drag empty space to orbit; **Reset View** restores the camera;
   - edit the selection in the Inspector (position / rotation / scale / mesh / physics);
   - **Ctrl+S** writes `scene.gensscene` (the lossless `gensscene-1` editor document) beside the binary; **Load** restores it;
   - the console row at the bottom accepts `help` / `entities` / `validate`.

## Honest boundaries of v1.1

- **Presentation is readback-based.** Each frame is composed offscreen, read back, and uploaded to the swapchain (ADR-0036). The cost (~5.5 MB CPU round-trip per frame at 1600×900) is invisible at editor frame rates on any real GPU, but it is the documented performance boundary of v1.1; direct-to-surface rendering is future work.
- **No file dialogs.** Save/Load use the deterministic `scene.gensscene` path beside the working directory (the editor is session-scoped, like the reference design). Native dialogs are future work.
- **No redo.** Undo covers transform edits, deletions and creations; restored entities receive new entity ids, and redo would need id remapping — deliberately out of v1.1 scope.
- **HiDPI renders at native resolution with UI scale 1.0** (pixels-per-point pinned, the engine's determinism position). On a 150%-scaled Windows display the UI is smaller than OS-scaled apps — correct, just not DPI-scaled.
- **Game View / Animator / Network tabs** remain honest empty states (the runtimes behind them are real; the panel wiring is future work).
- **macOS builds are unsigned** (no notarization); the first launch needs right-click → Open.

## How CI verifies the window on every push

The `--smoke-frames` mode runs the SAME window code path headlessly (xvfb on Linux, the runner desktop on Windows/macOS): it opens the window, presents frames, clicks the real Play/Pause/Stop toolbar buttons at their layout-captured rects, drives real pointer drags, sends Ctrl+S as real key events, verifies the play/pause/stop state machine at pinned frames (any miss fails the run), and writes a PNG of the last presented frame plus a JSON evidence blob (format `gens-editor-smoke-1`) containing the adapter summary and per-interaction verdicts. This is the mechanism behind the "Verified: the window opens and interacts on Windows/Linux/macOS CI" claims below — no manual testing anywhere in this project's history.

## Verification evidence (this release)

- CI green on all three OSes (build, clippy `-D warnings`, unit tests, GPU integration, headless editor replay, **interactive window smoke** with per-interaction verdicts, size gates, release manifest gate).
- The release job builds the zips from this tag on real runners, computes `SHA256SUMS` over every attached asset, and a verification job downloads the **published** zips and runs the window smoke from the unpacked binaries before the release is declared good.

## Assets

| Asset | Contents |
|---|---|
| `gens-editor-windows-x86_64.zip` | `gens-editor.exe` — the interactive editor (Windows) |
| `gens-editor-linux-x86_64.zip` | `gens-editor` — the interactive editor (Linux) |
| `gens-editor-macos-aarch64.zip` | `gens-editor` — the interactive editor (Apple Silicon) |
| `gens-runtime-windows-x86_64.zip` | `gens-pkg.exe`, `gens-player.exe` — build/run `.genspkg` packages (Windows) |
| `gens-runtime-linux-x86_64.zip` | `gens-pkg`, `gens-player` (Linux) |
| `gens-runtime-macos-aarch64.zip` | `gens-pkg`, `gens-player` (Apple Silicon) |
| `hello-patrol.genspkg`, `hello-sink.genspkg` | Example games — try `gens-player --package hello-patrol.genspkg --steps 600 --out report.json` |
| `gens-release-manifest.json` | The self-validating `gens-release-1` inventory |
| `SHA256SUMS` | Checksums over every asset |

Each zip carries a README stating what the binaries are and how they were verified.
