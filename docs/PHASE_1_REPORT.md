# Phase 1 Report — Workspace + Core Runtime + ECS + Math

- **Branch:** `phase1-core` (merge target: `main`)
- **Date:** 2026-10-12 (UTC)
- **Toolchain:** rustc/cargo 1.99.0 stable (installed in sandbox this phase — R-015 closed), edition 2024, rustfmt + clippy enforced

## 1. What was delivered

### 1.1 Workspace architecture (Phase 1 task 1)
Root `Cargo.toml` with `resolver = "3"`, edition 2024, shared package metadata, **workspace lints** (`unsafe_code = "forbid"`, clippy `all` enforced as `-D warnings` by CI and `scripts/verify.sh`), and a release profile (`lto = "thin"`, `codegen-units = 1`). Fifteen crates per ADR-0001; all shared dependency versions centralized in `[workspace.dependencies]` (glam 0.34, bevy_ecs 0.20, tracing 0.1, thiserror 2, anyhow 1, image 0.25 with PNG-only features). Non-phase crates remain honest stubs with doc comments naming their phase.

### 1.2 gens-core — runtime foundation (task 2)
- **Game loop** (`game_loop`): pure `advance()` accumulator math (unit-testable without clocks) + `GameLoop<S: InstantSource>` driving closure-based fixed updates; fixed 60 Hz default, `max_catchup_steps` with **backlog drop** (spiral-of-death protection, `backlog_dropped` surfaced in reports); simulation steps run on *scaled* time so `time_scale = 0` pauses simulation while frames continue.
- **Time** (`time`): `Clock` (raw/scaled delta as f64 internally, elapsed accumulators, frame counter) + `InstantSource` abstraction with `MonotonicSource` (production) and `FixedSource` (deterministic, tests/CI).
- **Events** (`events`): type-safe `Events<E>` with snapshot-drain semantics (deliver-since-last-drain-finished, send-order preserved, O(n), abandoned drains retire rather than re-deliver) + `EventRegistry` keyed by `TypeId`, monomorphized per-event path (no per-event dynamic dispatch).
- **Logging** (`logging`): tracing re-exports + `GENS_LOG`-driven console subscriber.
- **Errors** (`error`): `EngineError` (thiserror) + `Result` alias + top-level `anyhow` re-export.

### 1.3 gens-ecs — ECS facade (task 3)
bevy_ecs **0.20.0** pinned in workspace deps; engine code touches the facade only (ADR-0002). Components: `Transform` (local TRS, `to_matrix`), `WorldTransform` (written only by propagation), `Name`, `Active` (marker), `Tag`. Hierarchy built on bevy 0.20's native `ChildOf`/`Children` relationships with `attach_children`/`detach_from_parent` helpers. **Transform propagation**: exclusive system, deterministic DFS, parentless-roots rule (`Transform` or `Children` required — excludes bevy's internal seed entity), grouping nodes without `Transform` pass parent matrices through unchanged. `EngineSchedules` = Update + PostUpdate(propagation) with `run_frame`. `prelude` module re-exports everything (documented `ContextExt` collision caveat with anyhow).

### 1.4 gens-math — math facade (task 4)
glam re-exports (Vec2/3/4, Mat3/4, Quat, IVec2/UVec2…) + engine utilities: deg/rad, lerp/inverse_lerp/remap, relative-epsilon `eq_eps`, frame-rate-independent `damp`, `Aabb` (contains/intersects/union/expand), `compose_trs` (the engine-wide local-to-world convention), `rotation_submatrix`.

### 1.5 Hello world example (task 6)
`cargo run -p gens-runtime --example hello_world` — full headless run: scene (grouping `Root` + orbiting `Hero` with player tag), 60 Hz fixed loop over N frames, ECS-driven animation, propagation, trail rendered by the `snapshot` CPU rasterizer (5x7 bitmap font, grid, HUD: frame/step counters, hero position), PNG output via CLI `--frames/--out`. **Two consecutive runs produce byte-identical PNGs** (determinism evidence: `docs/evidence/phase1/example-run.txt`). The snapshot itself is committed: `docs/evidence/phase1/hello_world.png` — this is an honest CPU rasterization of real ECS state, *not* GPU output (Phase 2 delivers that).

### 1.6 CI activation (task 5)
The Phase-0 workflow self-activated when `Cargo.toml` appeared; this phase added the real gates: **headless smoke test** (runs hello_world at 60 frames on all three OS, verifies the PNG, uploads it as an artifact from Linux) and the **binary size gate** (160 MiB on the debug example binary — 2× the measured 79.8 MiB baseline; ADR-0009). Full matrix per push: fmt, clippy `-D warnings`, build, tests, smoke, size, docs-integrity, secret-scan.

## 2. Success criteria scorecard (honest)

| Criterion | Status | Evidence |
|---|---|---|
| Workspace compiles on 3 platforms | **Met** (CI matrix green) | branch CI run (see §5) |
| CI green (build+test+clippy+fmt on Win/Linux/macOS) | **Met** | same run |
| Headless example runs on CI, screenshot artifact | **Met** | `smoke-snapshot` artifact |
| 20+ unit tests green | **Met — 53 tests, 0 failures** | `docs/evidence/phase1/test-output.txt` |

## 3. Numbers

| Metric | Value |
|---|---|
| Unit tests (workspace) | **53 passed, 0 failed** |
| Test distribution | math 11, core 22 (error 2, logging 2, time 5, events 6, loop 7), ecs 15 (5 components + 6 propagation + 1 traversal/roots + 3 hierarchy), runtime 5 |
| hello_world debug binary (Linux) | 79.8 MiB (gate: 160 MiB) |
| Example determinism | byte-identical PNG across runs (cmp exit 0) |
| Sim verification | 120 frames → 120 fixed steps, 0 hitches, hero ends at exactly (60, 0) after 2 orbits |

## 4. Issues found and fixed during the phase (honest log)

1. **f32 scaled-delta drift** — accumulating f32 scaled deltas into f64 elapsed failed the 1e-9 tolerance; fixed by keeping scaled delta f64 internally (commit `fix(core)`).
2. **Event-bus contract violation** — the first double-buffer implementation drained at call start, contradicting its own docs (5 tests red, one red commit reached the branch through a shell-quoting flaw in my commit chain). Fixed to snapshot/take semantics; the shell lesson is captured in the worklog (P1-6b) and applied since (explicit exit-code gates).
3. **bevy_ecs 0.20 discoveries** — `QueryState::single` now takes `&World`; every `World` contains one internal seed entity (`0v0`) that parentless-root queries must exclude; the bevy prelude's `ContextExt` collides with `anyhow::Context` (documented in the prelude; example imports explicitly).
4. **Rust 1.99 clippy lints** — `chunks_exact_to_as_chunks` and `unnecessary_cast` appeared on the new toolchain; fixed rather than allowed.
5. **Phase 0 diagnosis correction** — the "invalid credential" conclusion from Phase 0 was a harness flaw (non-persistent env between tool calls), corrected in the record on this branch's first commit (R-001 reclassified, R-017 added).

## 5. CI runs

Recorded at merge time in `docs/evidence/phase1/ci-runs.txt` (run URLs + conclusions for the final branch run and the `main` merge run). The badge in README reflects the latest `main` status.

## 6. Decisions & risks delta

- ADR-0009 added (size gate calibration). 
- R-015 (missing Rust toolchain) → **Mitigated** (rustup 1.99.0 installed; CI installs its own).
- R-017 (non-persistent env) → process updated: every authenticated command carries its credential inline.
- R-004 (software-render honesty) → note: Phase 1 "screenshots" are explicitly CPU rasterizations of ECS state; GPU rendering and GPU-based CI screenshots begin Phase 2.

## 7. What Phase 2 can build on

Deterministic loop + ECS with working hierarchy/propagation + math + events + logging + a CI that renders and uploads snapshots on three platforms — exactly the substrate the wgpu renderer needs. The snapshot module stays as the headless fallback path for CI.
