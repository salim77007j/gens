# Phase 9 Report — Cinematics + World Building + Plugins

**Branch:** `phase9-cinematics-world-plugins` · **Base:** `2ac93d2` (main, run #110 green, tag `pre-phase9-safe`)

## 1. What shipped

### 1.1 Cinematics — `gens-cinematics` (workspace member 18) + runtime integration

- **`track`** — typed keyframe tracks (`Float`/`Vec3`/`Quat`) with `Linear`/`Step`/`Smoothstep` (plain-arithmetic easing), clamp-at-ends sampling, and construction-time validation: empty tracks, duplicate times and non-finite values are loud errors. 11 tests.
- **`timeline`** — tracks addressed by `(logical target, Channel{Translation,Rotation,Scale,Fov,Fade})` in a `BTreeMap` (deterministic order), kind-vs-channel validation, every key inside `[0, duration]`, events inside `(0, duration]` (a zero-time event could never fire on a first play — rejected instead of silently dead), stable event ordering, and a `CameraCut` intent (hard cut or declared blend-in). Builder with fail-fast validation.
- **`player`** — the deterministic state machine: `advance(dt)` takes the fixed delta as a parameter (no clock anywhere), events fire on half-open windows `(from, to]` exactly once per cycle, `Loop` wraps tail-then-head, pathological multi-cycle deltas fire every traversed cycle, and negative/non-finite/overflow deltas are typed errors. 21 tests.
- **`gens-runtime::cinematic`** — `CinematicStage` (player + binding map), `CameraPose` staging component, `advance_and_apply` with the order **advance → sample → validate → write** (a frame's writes reflect the time the step ended at); validation failures rewind the clock (seek is exact) so a rejected apply is a true no-op; per-entity field folds in deterministic sample order. `Fov` lands in `CameraPose`, `Fade` returns in the outcome.
- **`cinematic_hello`** — the ADR-0030 proof: straight-through, fresh replay and seek-resume runs over 300 fixed steps; all per-step world hashes byte-identical; post-finish stability over the observed tail; `gens-cinematic-1` report with environment disclosure. Two local artifacts byte-identical.

### 1.2 World building — `gens-worldgen` (workspace member 19)

- **`noise`** — `Noise2D`: SplitMix64-style lattice hashing with an **exact** `[0,1)` extraction (`from_bits(1.0 | mantissa) − 1.0` — no rounding anywhere), smoothstep value noise, FBM with a validated octave ladder.
- **`heightfield`** — grid sampling (non-finite heights are loud construction errors), `to_mesh` with central-difference normals (clamped borders) and CCW indices, FNV-1a content hash over the canonical byte encoding; a local `fnv1a64` keeps the crate dependency-free.
- **`scatter`** — jittered-lattice placement where every point derives from `(seed, cell)` via two independent noise channels; density mask from the same noise; i32 lattice domain validated loudly.
- **`chunk`** — `WorldGen::from_config` (validated once); `generate(coord)` samples terrain at **world** coordinates so neighbors share border heights (pinned by a test), meshes stay **chunk-local** (the origin carries placement — the hash addresses shape and float precision survives far-from-origin chunks), scatter is world-space.
- **`streaming`** — `ChunkStreamer`: Chebyshev disc desired-set over a `BTreeMap`, differential sorted load/unload plans, idempotent re-update.
- **`worldgen_hello`** — the ADR-0031 proof through the live ECS: generation-order independence (coord-keyed maps over three orders), streaming determinism (identical load/unload ledgers), restart determinism (identical per-step world hashes), terrain persistence (a watched prop from a never-unloaded chunk keeps its terrain y). `gens-worldgen-1` report. Two local artifacts byte-identical.

### 1.3 Plugins — `gens-plugin` (workspace member 20) + the out-of-tree proof

- **`registry`** — the `Plugin` trait (name + build), `PluginHost` exposing the composition root's `World` and `EngineSchedules` (`add_update_systems`/`add_post_update_systems` take anything bevy's `add_systems` accepts — real typed systems; intra-plugin order is the author's `.chain()` contract, cross-plugin order is registration order), a duplicate/empty-name-rejecting registry, and `apply()` returning an `AppliedPlugins` evidence record; failed builds name the failure and keep earlier plugins' work (the ADR-0028 never-half-run discipline).
- **`plugins/hello-plugin`** — a crate **outside the workspace** (root `Cargo.toml` excludes it) that path-depends on the engine crates like any downstream user: `HelloPlugin` registers an `Orb` component + `OrbitState` resource (fixed dt) + a real orbit system + startup spawn. `hello-plugin-demo` proves in-process bit-identity across two runs and loud duplicate-name rejection; `gens-plugin-hello-1` report. **If the plugin API only worked inside the workspace, this crate would not compile — that is the proof.**

### 1.4 Editor integration (closes Phase 8's honest empty states)

- **Game View** — renders the scene through the director's `CameraPose` (`render_game_view`: position, rotation-aimed forward/up, pose FOV); the director toolbar shows state/time/fov; no orbit interaction while a cinematic owns the camera.
- **Animator** — the live cinematic's real track table (target/channel/keys), progress bar, fired events, camera-directed badge.
- **Network** — renders a session snapshot (`NetPanelData`); the honest empty state explains why a live session cannot exist inside a byte-hashed replay (packet timing). The snapshot type is unit-tested; the counters' home is documented.
- **`editor_hello`** — the replay now runs a real 0.5 s take (CineCamera dolly + yaw + FOV ease, one event, fixed dt); step 20 switches to Game View so the final CI-rendered frames exercise the director-camera path end to end; evidence JSON records `cinematic_events`.

### 1.5 CI + gates

- Three new smokes on all 3 OS: cinematic, worldgen, out-of-tree plugin (the strongest one — it compiles a downstream crate against the engine and fails the run if the API regresses).
- `verify.sh` renumbered to **18 gates**.

## 2. Run ledger (phase 9 branch)

| Run | Commit | Result | Note |
|---|---|---|---|
| #111 | `1a93ffd` P9-0 | cancelled | superseded by the P9-1 push (content covered by #112's green) |
| #112 | `b340a7b` P9-1 | success | keyframe tracks |
| #113 | `083c896` P9-2a | success | timeline + player |
| #114 | `7cc9923` P9-2b | success | runtime integration + replay proof |
| #115 | `67c69f6` P9-3 | success | noise + heightfield + scatter |
| #116 | `51016dc` P9-4 | success | chunks + streaming + worldgen proof |
| #117 | `5dff6e3` P9-5 | success | plugin registry + out-of-tree proof |
| #118 | `5ecd60c` P9-6 | success | editor tabs |
| #119 | `78a9387` P9-7 | success | 3 new smokes + 18 verify gates |

## 3. Issue log (found → fixed, with evidence)

1. **The seek-semantics lesson (design, caught before it could ship wrong)** — the player's local time is a sequential f32 accumulation; 60 × (1/60) = `0.9999997` (bits `3f7ffffb`), not 1.0. A seek to a *nearby literal* shares no addition history and diverges in low bits. Resolution: seek targets are exact f32 state values (the save format's philosophy, ADR-0021); `cinematic_hello` proves seek-equivalence from the exact internal time and records its bit pattern. The same accumulation signature explains why the naive "240 steps = 4.0 s" post-finish window was wrong — the proof now observes the tail instead of assuming it.
2. **Input-data platform independence** — the first `cinematic_hello` draft computed key rotations with runtime `sin`/`cos`; platform libm could make cross-OS *inputs* differ and undercut the replay claims. Fixed: every timeline key in every proof is an f32 literal.
3. **Order-proof comparison bug (worldgen)** — the first draft compared hash *sequences* across generation orders; different coords legitimately produce different hashes, so it failed spuriously. Fixed: coord-keyed maps per pass, equal maps prove order independence.
4. **`gen` is a reserved keyword (edition 2024)** — test helpers named `gen` failed to compile; renamed to `generator`/`config`.
5. **NaN never equals NaN** — an `assert_eq!` on `InvalidDuration { duration: NaN }` was vacuously true-prone; asserted by pattern with `duration.is_nan()`.
6. **fmt gate drift caught locally (#98 lesson held)** — a clippy-driven `collapsible_if` edit skipped the fmt pass once; `verify.sh` died at gate [2/17] *before push*, `cargo fmt --all` fixed it, zero CI cost. The Phase 8 lesson is now habit.
7. **R-032 disk incidents ×3 this phase** — ENOSPC at the test build (0.14 GB free), at the P9-7 verify (twice). Mitigations that held: `cargo clean` passes between clippy/test/smoke phases, `CARGO_INCREMENTAL=0`, freeing the sandbox's `~/.cache` (2 GB), and the out-of-tree plugin sharing the workspace target via `CARGO_TARGET_DIR`. Zero work lost; every local pass ended `VERIFY: PASS`.
8. **bevy 0.20 facts pinned** — `Schedule::add_systems` takes `impl IntoScheduleConfigs<ScheduleSystem, M>`; `World::insert` needs `Bundle`; `Component` derive requires `bevy_ecs` in the deriving crate's scope; `Query<&mut T>` iteration yields `&mut` only through `&mut query`.

## 4. Honest scorecard

| Criterion (README roadmap: "Cinematics + world building + plugins") | Status |
|---|---|
| Cinematics | **Done** — pure sampled timelines, deterministic playback + events, camera direction with explicit staging, replay + seek-equivalence proven per integration (ADR-0027 pattern) |
| World building | **Done** — seeded noise/FBM, heightfield→mesh with content hashes, deterministic scatter, chunk content as a pure function of `(seed, coord)`, differential streaming; order/streaming/restart proven through the ECS |
| Plugins | **Done** for the CI-provable half — in-process registration of real typed systems with an evidence record, proven from OUTSIDE the workspace; **dynamic-library loading is NOT in Phase 9** (dlopen is not CI-provable on the 3-OS matrix; would need its own ADR) |
| Editor Game View / Animator tabs | **Done** — real cinematic data + director-camera rendering; the tabs' byte-identity rides the replay |
| Network tab | **Snapshot-wired, honest-empty in the replay** — a live session would break byte-identity on packet timing; the type is real and unit-tested, the limitation is stated where the empty state renders |
| Window shell (winit) | Still **deferred** (unchanged from Phase 8; ADR-0028) |
| Packaging texture/audio assets | Still **future work** (unchanged; Phase 8 handoff) |

**Tests:** gens-cinematics 32 lib; gens-worldgen 29 lib; gens-plugin 3 lib; gens-editor 40 lib (3 new cinema); gens-runtime +6 (5 cinematic integration + example doctest pins); workspace **468 passed / 0 failed / 9 ignored (GPU)**, carried by CI on every push.

**Binary sizes:** debug example binaries ~121 MB each (the 350 MiB ADR-0012 gate binds `gpu_hello` and stayed green in CI; release gate enforced by CI runners — local runs loud-skipped at <3 GB free per R-028).

## 5. Decisions & risks delta

- **ADR-0030** — cinematics are pure sampled timelines advanced by the fixed step; no ECS in the leaf crate; determinism proven per integration; camera direction is explicit staging (ADR-0026's rule applied to directors).
- **ADR-0031** — chunk content is a pure function of `(seed, chunk_coord)`; the noise core is integer-lattice with **no transcendentals** (one documented exception class: `sqrt`, correctly rounded per IEEE-754); no external noise crate (R-026 churn kept out of the determinism path).
- **ADR-0032** — plugins register real typed bevy systems in-process; the out-of-tree crate is the proof; dynamic loading deliberately out.
- **RISKS**: new R-033 (f32 time accumulation vs seek semantics — mitigated by exact-bit seek contract + proof), R-034 (transcendental creep into determinism-critical crates — mitigated by the ADR-0031 rule + input-literal discipline), R-035 (editor snapshot tabs drift from live state — mitigated by the ADR-0025 real-vocabulary rule + geometry tests), R-032 incident note (3 more ENOSPC events, playbook held).

## 6. Evidence index

- `docs/evidence/phase9/ci-runs.txt` — the ledger above (#111–#119)
- `docs/evidence/phase9/cinematic_local.json` — the ADR-0030 proof (replay identical, seek equivalent, seek-target bits)
- `docs/evidence/phase9/worldgen_local.json` — the ADR-0031 proof (order/streaming/restart identical)
- `docs/evidence/phase9/plugin_local.json` — the ADR-0032 proof (in-process identity, duplicate rejection)
- `docs/evidence/phase9/verify_18gates.txt` — the local 18-gate PASS transcript
- CI artifacts per run: `smoke_cinematic.json`, `smoke_worldgen.json`, `smoke_plugin.json` join the existing snapshot set

## 7. Phase 10 handoff

Row 10 is "Final testing + benchmarks + validation". Open threads this phase leaves deliberately: the window shell (winit) still awaits a CI-provable story; packaging covers scripts + data documents only; the Network tab's live-session path needs the windowed editor; cinematic clips are authored in code (a serialized cinematic format is natural Phase 10 polish); the bench harness (ADR-0029) should grow a worldgen/cinematic scenario row so the new pillars are measured, not just proven.
