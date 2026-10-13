Gens Engine v1.0.0 — bundle contents
=====================================

This zip was built by GitHub Actions from the release tag (no local
builds). License: MIT OR Apache-2.0.

WHAT IS INSIDE
--------------
(Windows builds end in .exe; Linux builds have no extension.)

  gens-editor      The Gens visual editor, demo build. Runs a
                   deterministic, scripted editor session over a real
                   scene (25 entities: hierarchy tree, AI guards,
                   physics crates, scripted spaceship) through the FULL
                   editor UI — hierarchy, 3D viewport, inspector, asset
                   browser, console — and writes the results:

                     editor-demo.png   1600x900 composite of the editor
                                       UI rendered through your GPU
                     editor-demo.json  per-frame hash evidence

                   Usage:
                     gens-editor --steps 24 --out editor-demo

                   Requires a GPU adapter (Vulkan / Metal / DirectX 12).
                   On a machine without one it exits with an adapter
                   error rather than pretending.

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

HONEST SCOPE
------------
Headless / offscreen by design — the engine ships without a window
shell (winit deferred, ADR-0028). There is no interactive window yet;
the binaries above render and simulate for real, and write evidence
files you can inspect. GENSPKG1 carries scenes + Rhai scripts + data
documents; texture/audio streaming inside packages is future work.

Source: https://github.com/salim77007j/gens
