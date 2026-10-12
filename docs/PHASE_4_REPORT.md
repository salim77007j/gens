# Phase 4 Report — Animation + UI + AI (deterministic cores, egui rendered through the engine's own wgpu)

- **Branch:** `phase4-animation-ui-ai` (merge target: `main`)
- **Date:** 2026-10-12 (UTC)
- **Toolchain:** rustc/cargo 1.99.0 stable, edition 2024; **egui 0.36.2** (pinned; every API verified against vendored source before use); animation and AI are dependency-free beyond gens-math/thiserror

## 1. What was delivered

### 1.1 Animation — pure clip/blend/controller core (P4-1, P4-2; ADR-0016)
`gens-animation` is a pure `(state, time)` sampling layer: a flat cycle-free `Skeleton` (parents strictly earlier indices) with TRS `JointPose` and forward kinematics; keyframe `VecTrack`/`QuatTrack` (linear, step, shortest-arc slerp; sorted + validated at construction; `partition_point` sampling); duration-bounded `Clip`s writing only the joints they own; in-place `blend_into` and clamped `crossfade`; and an `AnimationController` state machine — named states over `Arc<Clip>`, a declared parameter table (`Number`/`Flag`), data-only transition conditions (serializable, replayable), crossfades where the outgoing clip keeps playing, insertion-order first-match transitions, and looping `play_time` wrap. No wall clocks, no hidden state; controllers are replay-tested bit-for-bit. Skinning and glTF import land with the assets phase on top of this layer.

### 1.2 UI — egui headless core + engine-owned wgpu backend (P4-5, P4-6; ADR-0017)
`gens-ui` wraps egui 0.36 in two layers. The **pure core** turns a neutral `UiEvent` stream into fully tessellated `UiFrame`s (pixel-space meshes + texture uploads): one `ctx.run_ui` pass, `ctx.tessellate` to clipped primitives, a CPU texture mirror so partial font-atlas deltas still produce whole-texture uploads, deltas sorted and acknowledged (epaint 0.36 Drop-asserts unapplied deltas), pixels-per-point pinned to 1.0, engine simulation time only. The **wgpu backend** lives in `gens-renderer::ui`: vertices convert to NDC on the CPU (the pipeline has no uniforms at all), premultiplied over-blending (one / one-minus-src-alpha), per-batch scissor, color-only pass through `Renderer::draw_ui_and_read`. egui's premultiplied-sRGB vertex colors decode to linear at the render boundary — the Phase 2 convention — so opaque colors round-trip byte-exactly, pinned by GPU tests on lavapipe and Metal. Notably, egui's text glyphs and buttons render through gens' own pipeline: the HUD proof in `animate_hello` runs entirely on the CPU (tessellated frame hashed into evidence), while the pixel proof runs in CI with real adapters.

### 1.3 AI — behavior trees, grid A*, steering (P4-3, P4-4; ADR-0018)
`gens-ai` hand-rolls the decision and navigation layers with zero external dependencies: `NodeStatus`/`Blackboard`/`Behavior` with memory-based Sequence/Selector (finished children are not re-ticked — proven by tick-counting tests), Parallel (All/Any), Inverter, Repeat, Wait, Action/Condition leaves; a `Grid` with walkable/cost cells and A* whose priorities are **integer-quantized** (cost·1024 → u32, total order `(f, h, index)`) with fixed neighbor expansion order and corner-cut prevention — the same grid always yields the same path; `Steering` (seek/flee/arrive) plus a stateful forward-only `PathFollower` with arrival semantics; the stateless `follow_path` helper keeps a documented overshoot limitation.

### 1.4 Integration proof — `animate_hello` (P4-7)
A patrol agent on a 12×10 grid (central obstacle) is driven by a behavior tree (dwell at waypoints / steer along A* paths), whose movement flag drives the animation state machine (walk/idle with 0.2 s crossfades, 2-joint skeleton: bob + sway), whose FK output writes the ECS `Transform` and propagates through the Phase 1 pipeline; a headless egui HUD pass runs every step and its tessellated frame is FNV-hashed into the evidence. Two 900-step runs must be **bit-identical** — positions, animation states and UI frame hashes alike. Behavioral gates: patrol spans the map (x 1.5 → 10.5 measured), both animation states observed, walked distance > 16 (measured 24.6).

### 1.5 CI — animate smoke on every OS + 9-gate verify (P4-8)
`animate_hello` runs on **all three OS** every push (CPU-only, no adapter caveats); `smoke_animate.json` joins the artifacts; `verify.sh` mirrors everything as 9 local gates. Run #55 produced **byte-identical smoke evidence on Windows, Linux and macOS (4100 B each)** — recorded as evidence under R-013/R-021, not claimed as a cross-platform guarantee.

## 2. Success criteria scorecard (honest)

| Criterion | Status | Evidence |
|---|---|---|
| Skeletal animation: clips, blending, state machines | **Met** | 44 animation tests: FK vs hand-composed matrices, slerp shortest arc, blend weights, crossfade continuity, transition ordering, replay bit-identity |
| Runtime UI + text rendering | **Met** | egui core: 7 tests (synthetic-pointer click, frame replay); backend: 2 GPU tests on lavapipe+Metal — opaque fill byte-exact, glyphs/button pixels verified, hover repaints |
| Deterministic AI (BT + pathfinding + steering) | **Met** | 31 AI tests: memory semantics via tick counters, A* optimality shapes, corner cuts, deterministic tie-breaks, steering contracts |
| Three pillars compose with the Phase 1 loop/ECS | **Met** | `animate_hello`: BT → steering → controller → FK → ECS → `WorldTransform` → UI hash |
| Deterministic replay (in-process) | **Met** | two 900-step runs bit-identical incl. UI hashes, asserted on every push (R-013 scope) |
| CI green on 3 OS | **Met** | runs #53/#54/#55 SUCCESS (all jobs); GPU integration renders for real on Linux/macOS |
| UI renders through ONE wgpu tree | **Met** | no egui-wgpu in the lockfile; `ui.wgsl` + `UiPipeline` in gens-renderer (ADR-0017) |

## 3. Numbers

| Metric | Value |
|---|---|
| Workspace tests | **199 passed, 0 failed, 8 ignored** (GPU: 6 renderer + 2 UI, rendered in CI on Linux/macOS) |
| New tests this phase | 87 (animation 44, AI 31, UI core 7 + GPU 2, renderer UI unit 4, example 2 − overlaps in counting) |
| animate smoke (CI, run #55) | Windows 4100 B, Linux 4100 B, macOS 4100 B — byte-identical |
| physics smoke (unchanged) | 29290 / 29290 / 29253 B |
| gpu_hello PNG | 2088 / 2084 / 2090 B (backend-dependent, structural assertions) |
| Binary size gate | 174056344 B (166 MiB) vs 350 MiB gate (ADR-0012 unchanged; egui added ~2 MiB) |
| CI runs consumed | 12 branch runs (#44–#55), 3 red — all three documented below with root causes |

## 4. Issues found and fixed during the phase (honest log)

1. **egui 0.36 API drift (twice inside one minor)** — `Context::run` is gone (`run_ui` frame model); `CentralPanel::show` now takes the pass `&mut Ui` (not `&Context`); `MouseWheel` carries a `TouchPhase`; `ImageData` is `Color`-only; mesh indices are `Vec<u32>`. All absorbed by reading vendored source BEFORE coding; the closure contract of `UiContext::run` passes the pass-`Ui` accordingly.
2. **epaint Drop-asserts unapplied texture deltas** — the first UI test run panicked inside epaint: deltas must be consumed *or* `clear()`ed. Extraction is the handling; `clear()` is the acknowledgement (documented in `UiContext::run`).
3. **CI-caught: UI vertex colors double-brightened (run #51)** — panel fill stored `[79, 87, 101]` instead of `[20, 24, 33]`. The math was unambiguous: 79 = encode(20/255) — raw byte/255 fragments treated as linear, re-encoded by the sRGB target. Fixed by decoding premultiplied-sRGB vertex colors at the render boundary (the P2-8 convention), with a GPU-free round-trip unit test.
4. **CI-caught: my test probed the wrong pixels (run #52)** — after the decode fix the fill round-trip passed, but the text scan sat on a hardcoded row above the glyphs. The renderer was correct; the test now captures widget rects from the layout (the P2-8c lesson repeated: fix the test's geometry, not the code).
5. **play_time advanced twice per update (P4-2)** — a duplicated `+= dt` line made looping oscillate (0.5 → 0 → 0.5); found by my own wrap test and a probe example before commit.
6. **Slerp test endpoint was mathematically impossible (P4-1)** — `from_rotation_z(-270°)` canonicalizes to the +90° quaternion (unit quaternions double-cover rotations); the "long way" is not representable. Endpoint replaced with a true −90°, fact documented in the test.
7. **Stateless `follow_path` turns back on overshoot** — "first waypoint beyond radius" re-targets an early waypoint once the agent passes it. Documented as a limitation; `PathFollower` (forward-only cursor, radius-based advance, final-waypoint arrival) is the correct primitive, with its own test narrative.
8. **clippy gate parity (run #49)** — three lints (`field_reassign_with_default`, `infallible_destructuring_match`, `clone_on_copy`) failed CI while `cargo test` was green because the local `clippy | tail -1` swallowed the failing exit status. Fixed the lints AND the process: clippy results are now checked fresh (`touch`/`cargo clean -p`) and by exit code, not output tail.
9. **Sandbox disk exhaustion (twice)** — `target/` hit the 9.9 GB disk ceiling during the egui build and again before the final test count (ENOSPC mid-link). `cargo clean` + `CARGO_INCREMENTAL=0` adopted; disclosed as environment incidents, zero code impact; one verify run was repeated after the cleanup to keep the evidence honest.
10. **A patch script silently no-op'd (again)** — one Python `str.replace` missed fmt-reworded text and left the GPU test unchanged ("nothing to commit"); re-applied with hard asserts on every replacement (the Phase 3 lesson, still paying rent).

## 5. CI runs

See `docs/evidence/phase4/ci-runs.txt` for the full ledger with links: #44 P4-0, #45 P4-1, #46 P4-2, #47 P4-3, #48 P4-4, #49 ✗ → #50 P4-5b, #51 ✗ → #52 ✗ → #53 P4-6c, #54 P4-7, #55 P4-8. The merge run and tag anchor are recorded there after the merge.

## 6. Decisions & risks delta

- **ADR-0016** animation core (skinning/glTF deferred), **ADR-0017** egui headless core + engine-owned wgpu backend (no egui-wgpu: wgpu unification), **ADR-0018** hand-rolled BT/A* with integer-quantized determinism.
- **RISKS:** R-013 narrowed (replay pin now covers AI + animation + UI hashes; cross-OS byte-identical animate smoke recorded as evidence, not guarantee); R-017 refined (env resets are per tool call — discipline held across 10+ pushes); **R-020** new (egui 0.x churn — pinned, source-verified, confined to gens-ui, replay-gated); **R-021** new (UI rasterization cross-OS bit-identity unproven — in-process only).

## 7. What Phase 5 can build on

A deterministic simulation core whose replay pin now spans input → physics → **AI → animation → UI**, a renderer that draws both 3D geometry and egui UI through one wgpu tree with offscreen readback, 199 green tests across three operating systems, and honest, measured knowledge of where cross-platform bit-identity ends. Scripting + assets + save (Phase 5) can load real content onto animation clips and UI layouts, and wrap the AI/animation surface in a scripting API without breaking the replay contracts.
