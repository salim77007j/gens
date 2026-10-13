# Gens Engine v1.0.0

The roadmap-completion release of the **Gens** engine — a small, honest, cross-platform Rust game engine (20 workspace crates: ECS, math, wgpu renderer, physics, audio, input, animation, UI, AI, scripting, assets, networking, cinematics, worldgen, profiler, build pipeline, editor, runtime, plugin API). Phases 0–10 are complete, CI-verified on Linux / Windows / macOS: **489 tests, 19 verification gates mirrored by CI on every push, 34 ADRs, 35 tracked risks, 11 wire formats.**

The full, verbatim Phase 10 report (the project's final self-audit) follows below the download table.

## What to download

| File | What it is |
|---|---|
| `gens-editor-windows-x86_64.zip` | **Windows** — the Gens editor (demo build) + `README.txt` |
| `gens-runtime-windows-x86_64.zip` | **Windows** — `gens-player` (headless game runner) + `gens-pkg` (package CLI) + `README.txt` |
| `gens-editor-linux-x86_64.zip` | **Linux** — the Gens editor (demo build) + `README.txt` |
| `gens-runtime-linux-x86_64.zip` | **Linux** — `gens-player` + `gens-pkg` + `README.txt` |
| `hello-patrol.genspkg` | Example game #1 — a scripted patrol scene (GENSPKG1) |
| `hello-sink.genspkg` | Example game #2 — a scripted descent scene (GENSPKG1) |
| `gens-release-manifest.json` | The self-validating `gens-release-1` inventory (crate set, ADRs, risks, roadmap, wire formats) |
| `SHA256SUMS` | SHA-256 checksums of every attached asset |

Every binary was built by GitHub Actions from this exact tag (no local builds, no cross-compilation), and the release was launch-verified on a real Windows runner before being marked Latest.

## Try it in 60 seconds (Windows)

```powershell
# 1. Download + extract both Windows zips, then:
.\gens-editor.exe --steps 24 --out editor-demo
#    -> editor-demo.png  : the full editor UI (hierarchy, 3D viewport,
#       inspector, asset browser, console) rendered through your GPU
#    -> editor-demo.json : per-frame hash evidence

.\gens-player.exe --package hello-patrol.genspkg --steps 600 --out patrol-report.json
#    -> patrol-report.json : a gens-run-report-1 (entities, script ticks,
#       final world hash, phase timings). Exit code 0 = world validated clean.

.\gens-pkg.exe verify hello-patrol.genspkg
#    -> every file hash inside the package, re-checked on your machine
```

## Try it in 60 seconds (Linux)

```bash
unzip gens-editor-linux-x86_64.zip gens-runtime-linux-x86_64.zip
./gens-editor --steps 24 --out editor-demo
./gens-player --package hello-patrol.genspkg --steps 600 --out patrol-report.json
./gens-pkg verify hello-patrol.genspkg
```

## Honest scope of this release — read before downloading

- **Headless / offscreen by design.** The engine deliberately shipped without a window shell: no interactive window yet (winit deferred and documented since Phase 8, ADR-0028). What you get when you run the editor demo is the *real* editor — real ECS scene, real physics bodies, real assets, real GENSSAVE1 autosave, real GPU render — driven through a deterministic scripted session and written to a PNG + JSON evidence pair. The player runs `.genspkg` packages and writes a JSON run report. Nothing here is faked, and nothing here pretends to be a windowed app.
- **The editor needs a GPU adapter** (Vulkan / Metal / DirectX 12 — anything a normal desktop has). On a machine without one, it exits with an adapter error instead of pretending.
- **Package scope.** GENSPKG1 carries scenes + Rhai scripts + data documents; texture/audio streaming inside packages is documented future work (see the report's scorecard below).
- **Versions.** The release tag is `v1.0.0` (the roadmap-completion milestone). Workspace crate versions remain `0.1.0` — renumbering 20 crates' inter-locking version pins is deliberate future hygiene, not an oversight.
- **macOS builds** ride the same CI matrix but are not attached in v1.0.0.

## The full Phase 10 report (verbatim)

