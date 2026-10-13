Gens Engine v1.1.0 — bundle contents
=====================================

This zip was built by GitHub Actions from the release tag (no local
builds). License: MIT OR Apache-2.0.

WHAT IS INSIDE
--------------
(Windows builds end in .exe; Linux/macOS builds have no extension.)

  gens-editor      The INTERACTIVE Gens editor (new in v1.1, ADR-0036).
                   Opens a REAL OS window with the live 3D viewport,
                   the full editor chrome, and a working play mode:

                     - Play / Pause / Stop: run the simulation, restore
                       the scene exactly on Stop
                     - Save / Load: lossless gensscene-1 scene files
                     - click objects to select, drag to move, drag
                       empty space to orbit
                     - menus: File / Edit / GameObject / View / Help
                     - shortcuts: Ctrl+S, Ctrl+O, Ctrl+Z, Ctrl+D, Del
                     - editable Inspector (Transform / Mesh / Physics)

                   Just run it with no arguments. Requires a GPU
                   adapter (Vulkan / Metal / DirectX 12); on a machine
                   without one it exits with an adapter error rather
                   than pretending.

                   Verification mode (what CI runs):
                     gens-editor --smoke-frames 22 \
                       --screenshot editor_window.png \
                       --json editor_window.json
                   which opens the same window headlessly (xvfb on
                   Linux), clicks the real toolbar buttons, and writes
                   PNG + JSON evidence.

  gens-editor-headless-replay
                   The v1.0.0 editor demo build: a deterministic,
                   scripted editor session over the docking-bay scene
                   through the FULL editor UI, writing per-frame hash
                   evidence:
                     gens-editor-headless-replay --steps 24 --out editor-demo

  gens-player      Headless game runner. Loads a GENSPKG1 package,
                   steps the real engine loop (scripts -> schedules),
                   validates world invariants and writes a
                   gens-run-report-1 JSON (entities, script ticks,
                   final world hash, phase timings). Exit code 0 iff
                   the world validates clean.

                   Usage:
                     gens-player --package hello-patrol.genspkg ^
                                 --steps 600 --out report.json

  gens-pkg         Package CLI. Build and verify GENSPKG1 packages:
                     gens-pkg build --name demo --scene scene.json ^
                                --asset scripts/mover.rhai --out demo.genspkg
                     gens-pkg verify demo.genspkg

  README.txt       This file.

EXAMPLE GAMES
-------------
Download hello-patrol.genspkg / hello-sink.genspkg from the release
page and run them with gens-player (above). hello-patrol walks a mover
along x in [0, 3); hello-sink descends a sinker from y = 2.0 and resets
it. Both are deterministic: the final world hash is stable run to run
and OS to OS.

HONEST SCOPE (v1.1)
-------------------
The window shell is REAL (winit 0.30) but v1.1 keeps documented
boundaries (ADR-0036 / docs/RELEASE_V1_1_NOTES.md): presentation is
readback-based (one CPU round-trip per frame — fine for an editor,
documented as future-work territory for direct-to-surface); save/load
use the deterministic scene.gensscene path (no native file dialogs
yet); undo covers transform edits / deletes / creates with new entity
ids on restore (no redo); the UI renders at pixels-per-point 1.0 (no
DPI scaling); Game View / Animator / Network tabs are honest empty
states. macOS builds are unsigned — right-click -> Open on first
launch.

Source: https://github.com/salim77007j/gens
