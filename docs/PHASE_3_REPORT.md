# Phase 3 Report — Physics + Audio + Input (headless, 3-OS CI)

- **Branch:** `phase3-physics-audio-input` (merge target: `main`)
- **Date:** 2026-10-12 (UTC)
- **Toolchain:** rustc/cargo 1.99.0 stable, edition 2024; **rapier3d 0.36.1** + **kira 0.12.5** (pinned in `[workspace.dependencies]`, every API verified against registry source before use)

## 1. What was delivered

### 1.1 Physics — rapier3d behind a facade (P3-1, ADR-0013)
`gens-physics` wraps the pinned rapier engine: `PhysicsWorld` with explicit fixed-timestep stepping (`step()` only, default 1/60), value-type `BodySpec`/`ColliderSpec` (dynamic/fixed/kinematic-position bodies; ball/cuboid colliders with friction/restitution/density; `linear_damping`, CCD flag), full pose/velocity readback, impulses, and kinematic teleporting. Every value crosses the rapier boundary through tested converters (`convert.rs`) because rapier 0.36 resolves to parry's glamx (glam 0.33.12) while gens standardizes on glam 0.34.1. ECS glue: `PhysicsResource` newtype resource (bevy_ecs 0.20 requires `Resource: Component`, which a distant impl cannot satisfy), `PhysicsBody` component, `step_physics_system`, and `sync_transforms_from_physics` writing poses into the standard `Transform` (fixed bodies keep authored transforms) so Phase 2 rendering consumes physics results unchanged. Raw rapier stays reachable as `gens_physics::backend`.

### 1.2 Input — pure action-mapping model (P3-2, ADR-0015)
`gens-input` is deliberately bevy-free: engine-owned `KeyCode`/`MouseButton`/`DeviceEvent` vocabulary, `InputMap` (actions → bindings, OR-combined, runtime add/unbind/clear), `InputState` fed by a `DeviceEvent` stream with an explicit end-of-frame `update()` rotating just-pressed/released edges, axis/axis-pair helpers, and focus-loss `clear_all`. Real device backends (winit/gilrs) plug into the same event stream in the windowing phase; ECS resource glue lands then as a newtype wrapper (the `Resource: Component` constraint documented in ADR-0015).

### 1.3 Audio — kira headless-first (P3-3, ADR-0014)
`gens-audio` runs kira 0.12's **MockBackend**: the real mixer/renderer advances deterministically — one `process()` tick renders exactly 128 frames (asserted by test). `AudioEngine`/`SoundHandle` expose play/pause/resume/stop/loop/position with state-machine verification (frozen positions while paused, wrapped loop regions, one-shot natural completion, independent handles). `tone.rs` synthesizes sine/silence from raw `Frame`s — no binary assets, no decoder features. cpal realtime output is a deferred cargo feature (ALSA headers unavailable in CI/sandbox; R-018).

### 1.4 Integration proof — `physics_hello` (P3-4)
A scripted `DeviceEvent` replay drives strafing impulses and a ground-checked jump on a damped, CCD-enabled player ball over a fixed floor. Per fixed step: script feed → action queries → `input.update()` → solver step → transform sync → `WorldTransform` propagation (the full Phase 1 pipeline). The simulation runs twice and must be **bit-identical** over 600 samples; behavioral gates verify strafe, pull-back, jump peak, floor integrity, and rest height; the trajectory is written as JSON evidence.

### 1.5 CI — physics smoke on every OS (P3-5)
New CI step runs `physics_hello` on **all three OS including Windows** — it is CPU-only, so unlike the GPU smoke it has no per-OS adapter caveats. `smoke_physics.json` joins the artifacts. `verify.sh` mirrors it as gate [6/8] (8 local gates total).

## 2. Success criteria scorecard (honest)

| Criterion | Status | Evidence |
|---|---|---|
| Rigid bodies + colliders + fixed-step stepping | **Met** | 13 physics tests incl. free-fall kinematics, resting contact, impulse/mass, kinematic teleport |
| Deterministic simulation (replay-ready) | **Met (in-process)** | 600-step two-world replay asserted bit-identical; `physics_hello` two-run replay asserted bit-identical on every push |
| Cross-platform determinism | **Honestly scoped** — x86_64 Windows == x86_64 Linux bit-identical; ARM64 macOS differs in low bits | run #40 logs: final_x 7.905011177062988 (Win/Linux) vs 7.905004501342773 (macOS) — recorded under R-013; networking must not assume cross-arch bit-identity |
| Audio playback state machine verified headless | **Met** | 10 audio tests; no audio device anywhere in CI; measured 128-frame tick contract |
| Action-mapped, rebindable input with frame edges | **Met** | 9 input tests; the demo's jump edge proved the documented query-before-update contract in practice |
| Three pillars compose with the Phase 1/2 engine | **Met** | `physics_hello`: input → actions → physics → ECS → `WorldTransform` |
| 20+ new unit tests | **Met — 32 new (13+9+10); workspace 112 tests 0 failures** | run #40 |
| CI green on 3 OS | **Met** | run #40 SUCCESS (all jobs) |

## 3. Numbers

| Metric | Value |
|---|---|
| Workspace tests | **112 passed, 0 failed** (+4 example unit tests inside that count) |
| New tests this phase | 32 (physics 13, input 9, audio 10) |
| Physics smoke (CI) | Windows 29290 B, Linux 29290 B, macOS 29253 B trajectory JSON |
| Cross-OS final_x (600 steps) | 7.905011177062988 (Win x86_64) / 7.905011177062988 (Ubuntu x86_64) / 7.905004501342773 (macOS ARM64) |
| `gpu_hello` debug binary (size gate) | 179.5 MiB vs 350 MiB gate (rapier+kira add ~5 MiB; ADR-0012 unchanged) |
| API verification | rapier3d 0.36.1, kira 0.12.5 — every call checked against vendored source |

## 4. Issues found and fixed during the phase (honest log)

1. **bevy_ecs 0.20 `Resource: Component`** — `impl Resource for PhysicsWorld` fails from a distance; discovered at first compile. Fixed with deliberate newtype resources (`PhysicsResource`), documented in ADR-0015 as the shape of the future input wrapper.
2. **Dual glam trees** — rapier 0.36 → parry glamx (glam 0.33.12) vs gens glam 0.34.1: distinct types despite identical layout. Fixed with a one-module, unit-tested conversion boundary (`convert.rs`) instead of a silent transmute.
3. **rapier 0.36's integrator is NOT semi-implicit Euler** — my first free-fall test pinned internal coefficients (measured drop = (n−3/8)·g·dt²). Rewrote the contract to Newtonian kinematics (velocity = g·t exact; position within 5 cm of ½g·t²), so upgrades cannot silently break tests that never tested engine truth.
4. **`RigidBodyType::SoftFrame`** — rapier 0.36 added soft-body frames; the facade's `body_kind` match was non-exhaustive at first compile. Mapped honestly (never solver-driven gameplay bodies).
5. **Per-step impulse magnitude error** — 0.35 Ns applied *every step* accelerated the ball to 47 m/s; it rolled off the 60 m floor and fell to y = −325 (min_y gate caught it). Fixed with realistic 0.05 Ns impulses, `linear_damping` on `BodySpec` (rapier has no rolling resistance), CCD on the player, and a 100 m floor.
6. **Input edge-ordering bug** — `input.update()` ran *before* action queries, so the jump's just-pressed edge was always cleared: the jump never fired. The fix (query first, rotate edges after) is exactly the frame contract documented in the crate docs — proven necessary by the demo's own gates.
7. **kira's default tween is 10 ms, not instant** — pause assertions failed at 2 ticks (5.8 ms of audio). Measured the tick semantics against source (128-frame buffers, 1-tick command lag) and wrote tests with an 8-tick settle helper.
8. **A patch script silently no-op'd** — a Python `str.replace` without an assertion left the physics step out of the example loop (x stayed 0). Re-applied with hard asserts; all subsequent patches assert their replacements.

## 5. CI runs

| Run | SHA | Result |
|---|---|---|
| [#40](https://github.com/salim77007j/gens/actions/runs/38176575188) | 62c7d79 | **success — full matrix, physics smoke on 3 OS, GPU smoke Linux/macOS** |

Full ledger with per-OS evidence: `docs/evidence/phase3/ci-runs.txt`. Physics trajectory artifacts (`smoke_physics.json`) upload with the `smoke-snapshots` artifact.

## 6. Decisions & risks delta

- **ADR-0013** (rapier3d facade + determinism scope), **ADR-0014** (kira headless-first, realtime deferred), **ADR-0015** (pure input model, ECS glue deferred with the `Resource: Component` rationale) added.
- **R-013** narrowed with measured cross-OS evidence (x86_64 pair bit-identical; ARM64 macOS low-bit variance).
- **R-018** (realtime audio unproven until windowing) and **R-019** (rapier/kira churn) added.
- No prior risk status changed otherwise; R-002 stayed Monitoring (every sub-item pushed the day it was written).

## 7. What Phase 4 can build on

A deterministic simulation core (physics + actions) wired into the ECS and the fixed-timestep loop, headless audio scheduling, 112 green tests across three operating systems — and honest, measured knowledge of where cross-platform float identity ends. Animation + UI + AI land on top of a simulation that replays bit-identically.
