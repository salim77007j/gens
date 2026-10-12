# Gens — Architecture Decision Records

Format per entry: **Status / Context / Decision / Alternatives considered / Risks & mitigations**.
This file is append-only: decisions are superseded by new ADRs, never rewritten. Every ADR that introduces risk must have a matching entry (or update) in `docs/RISKS.md`.

---

## ADR-0001 — Rust + Cargo workspace with per-system crates

**Status:** Accepted (Phase 0)

**Context.** Gens must compete on performance with Unity, Unreal, and Godot while remaining maintainable and verifiable by a small team and CI. The language and repository layout are the most expensive decisions to reverse later.

**Decision.** Rust on the stable channel, organized as a Cargo **workspace** with fifteen crates: `gens-core`, `gens-ecs`, `gens-math`, `gens-renderer`, `gens-physics`, `gens-audio`, `gens-input`, `gens-animation`, `gens-ui`, `gens-ai`, `gens-scripting`, `gens-assets`, `gens-networking`, `gens-editor`, `gens-runtime`. All shared dependency versions are inherited from `[workspace.dependencies]`; profiles and workspace lints (`warnings = "deny"`) are defined once at the root. Until a crate's phase arrives, it remains a thin stub so the workspace graph is real but cheap.

**Alternatives considered.**
- *Single monolithic crate* — rejected: compile times grow superlinearly, module boundaries are unenforced, CI parallelism is impossible.
- *A few mega-crates (engine / editor / runtime)* — rejected: boundary discipline decays; the per-system crates also document the architecture.
- *C++ core with Rust shell* — rejected: build-system complexity and memory-safety burden contradict the project's goals.

**Risks & mitigations.** Fifteen-crate overhead before Phase 1 → stubs are near-empty; dependency-version drift → all versions inherited from the root; compile-time creep → workspace lint gate + CI timing tracked from Phase 1.

---

## ADR-0002 — ECS: bevy_ecs behind a `gens-ecs` facade

**Status:** Accepted (Phase 0)

**Context.** The ECS is the backbone of the engine. Writing a competitive archetype ECS from scratch is a multi-year research effort with poor return on investment, while 2026-era Rust offers production-grade off-the-shelf options.

**Decision.** Integrate `bevy_ecs` (the standalone ECS crate, not the Bevy engine) as the storage/scheduler core. `gens-ecs` exposes a deliberate facade: engine components (`Transform`, `Name`, `Active`, `Tag`), hierarchy and transform-propagation systems, and engine-owned schedules. Engine code never touches raw bevy_ecs APIs except inside `gens-ecs`.

**Alternatives considered.**
- *hecs* — lighter, but no built-in scheduler; we would rebuild scheduling ourselves.
- *flecs (C++) via FFI* — excellent tech, but FFI + C++ toolchain on three CI platforms is a persistent liability.
- *Custom ECS* — rejected: time-to-competence is measured in years; zero product differentiation for our goals.

**Risks & mitigations.** Upstream breaking changes → pin the exact minor version in `[workspace.dependencies]`; facade contains churn to one crate. Determinism across platforms → validated in Phase 3 with fixed-step ordering tests (see `docs/RISKS.md` R-013).

---

## ADR-0003 — Renderer: wgpu (Vulkan / DX12 / Metal / WebGPU) with software fallback

**Status:** Accepted (Phase 0)

**Context.** The renderer must target three desktop graphics stacks plus WebGPU, run headless on CI machines that have no GPU, and stay safe-by-default.

**Decision.** Build `gens-renderer` on `wgpu`. Adapter selection detects NVIDIA / AMD / Intel (dedicated and integrated) and prefers dedicated GPUs; when no hardware adapter exists (CI), fall back to software adapters (lavapipe/llvmpipe on Linux, documented equivalents elsewhere). All shaders in WGSL.

**Alternatives considered.**
- *Raw Vulkan (ash)* — maximum control, but three-platform safety and CI cost are prohibitive for a small team.
- *OpenGL (glow)* — legacy, deprecated on macOS, dead end for WebGPU.
- *Direct3D-only or Metal-only* — contradicts the cross-platform mission.

**Risks & mitigations.** Software-render performance on CI is unrepresentative of real GPUs → benchmarks report both relative ratios and absolute numbers with environment disclosure (`docs/RISKS.md` R-004); wgpu driver quirks on vendor GPUs → zero-crash gate in Phase 2 plus adapter allow/deny list; upstream wgpu regressions → pin versions, patch locally or send upstream PRs (Global Rule 10).

---

## ADR-0004 — CI-first development: 3-OS matrix, headless verification, evidence artifacts

**Status:** Accepted (Phase 0)

**Context.** The user will not run or test anything until the whole project is declared complete (Phase 0 §0.10). Every claim of success must therefore be machine-verifiable.

**Decision.** `.github/workflows/ci.yml` runs on every push: a docs-integrity gate, a credential secret-scan, and a three-OS matrix (ubuntu-latest, windows-latest, macos-latest) that executes format check, `clippy -D warnings`, build, tests, a Linux headless smoke test (xvfb where needed), and a binary-size gate. Gates written in Phase 0 are **armed but inactive** until the Cargo workspace exists in Phase 1 (they detect the workspace and skip cleanly rather than failing). Evidence (screenshots, metrics, logs) is uploaded as artifacts and committed under `docs/evidence/`.

**Alternatives considered.**
- *Local-only verification* — rejected: unverifiable claims, violates §0.10.
- *Linux-only CI* — rejected: the engine claims Windows/macOS support; untested claims are forbidden by the honesty rule.

**Risks & mitigations.** CI minutes → free for public repositories; runner flakiness → retry policy documented in `docs/RISKS.md` R-005; slow software rendering → smoke tests render tiny scenes with explicit expectations (R-004).

---

## ADR-0005 — Branch-per-phase + rollback tags

**Status:** Accepted (Phase 0)

**Context.** Phases are milestones; `main` must always be in a verified, shippable state, and any regression must be reversible within five minutes (§0.2, §0.6).

**Decision.** Each phase works on `phase<N>-<slug>` (Phase 0: `phase0-risk-management`). `main` receives only verified phase results. Immediately before starting phase N, a tag `pre-phase<N>-safe` is placed on `main` as the rollback anchor. Phase 0 note: the repository was empty, so `main` is initialized from the verified head of `phase0-risk-management` (bootstrap, no merge needed) and tagged `pre-phase1-safe` before Phase 1 begins.

**Alternatives considered.**
- *Trunk-based (commit to main directly)* — rejected for this project: a broken main defeats the rollback guarantee.
- *Long-lived `develop` branch* — rejected: merge-hell risk with one active phase at a time adds no value.

**Risks & mitigations.** Stale branches at project end → all phase branches are kept (they are the granular history); tag mistakes → tags are local+remote deletable and re-creatable in minutes (procedure in `docs/ROLLBACK_PLAN.md`).

---

## ADR-0006 — Licensing: MIT now; dual MIT | Apache-2.0 finalized in Phase 1

**Status:** Accepted (Phase 0)

**Context.** A public engine repository needs a license from its first commit. The Rust ecosystem convention is dual MIT | Apache-2.0 (the Apache grant additionally covers patent claims), and most of our dependency tree will be MIT/Apache-dual.

**Decision.** Ship `LICENSE-MIT` in the bootstrap commit so the repository is unambiguously open source from day one. When the workspace crates land in Phase 1, add `LICENSE-APACHE` and set `license = "MIT OR Apache-2.0"` in all crate manifests — unless the user directs otherwise.

**Alternatives considered.** *Proprietary/all-rights-reserved* — rejected: contradicts the showcase intent and the Rust ecosystem norms; *GPL-family* — rejected: would constrain adopters and most likely the user's intent for an engine competing with permissively licensed peers.

**Risks & mitigations.** User may prefer a different license → single ADR change plus one commit swaps it before Phase 1 code exists; cost of change is near zero now and grows later, which is exactly why it is raised in Phase 0.

---

## ADR-0007 — Governance docs and evidence-in-repo

**Status:** Accepted (Phase 0)

**Context.** Global rules require documentation after every sub-item, honesty backed by evidence, and survival across environment resets.

**Decision.** The repository carries a fixed governance set: `docs/WORKLOG.md` (append-only per-change log), `docs/DECISIONS.md` (this file), `docs/RISKS.md` (risk register), `docs/ROLLBACK_PLAN.md`, `docs/EXPERIMENTS.md`, plus `docs/evidence/<phase>/` for raw proof (logs, metrics, screenshots) and `docs/design/` for reference material. The CI docs-gate fails any push that deletes a governance file. Evidence referenced by a worklog entry must exist in the same commit that entry lands in.

**Alternatives considered.** *Wiki/external docs* — rejected: external state is not protected by CI and not atomic with code; *evidence as CI artifacts only* — rejected: artifacts expire, repository files do not.

**Risks & mitigations.** Docs rot → each phase report explicitly reconciles WORKLOG/DECISIONS/RISKS at phase end; repo size growth from screenshots → PNGs are bounded per phase and reviewed in the phase report.

---

## ADR-0008 — Repository visibility: public

**Status:** Accepted (Phase 0)

**Context.** The push target is a GitHub repository under the user's account. CI minutes are free and unlimited for public repositories, and Phase 10 requires a public showcase (README, screenshots, benchmarks).

**Decision.** The repository is public. Secrets policy: no credential ever enters the repository (enforced by the secret-scan gate); the push token lives only in the local process environment for the session.

**Alternatives considered.** *Private repository* — rejected: Actions minutes would be consumed against a quota across the three-OS matrix, and the Phase 10 showcase would be invisible.

**Risks & mitigations.** Accidental secret commit → secret-scan gate + `.gitignore` for `.env*`; if a secret ever lands, `git-filter-repo` + token revocation procedure is documented in `docs/ROLLBACK_PLAN.md` §5.

---

## ADR-0009 — Binary size gate: 160 MiB on debug example binaries (Phase 1 baseline)

**Status:** Accepted (Phase 1)

**Context.** Phase 0 §0.7 requires a binary-size gate on every push. No threshold existed before real binaries did; the hello-world example (gens-core + gens-ecs + bevy_ecs + glam + image, debug profile) measured **79.8 MiB** on Linux x86_64 (rustc 1.99.0, debug with assertions and debug symbols).

**Decision.** Gate = 160 MiB (2x measured baseline) on the debug `hello_world` example binary, checked in CI on all three platforms. Release-profile gates with tighter budgets (LTO on, symbols stripped) are introduced with the renderer phase when real release binaries exist; the threshold moves by ADR, never silently.

**Alternatives considered.** Gating on `target/` total (rejected: dominated by build intermediates, noisy); gating only release (deferred: no release artifacts yet).

**Risks & mitigations.** Debug sizes vary across platforms (Windows PDBs live beside the binary, not in it — fine; macOS dSYM extraction not enabled) → gate watches the executable file itself, documented honestly; a dependency bump that explodes size trips the gate → that is the gate working as intended, revert-first per `docs/ROLLBACK_PLAN.md`.

---

## ADR-0010 — Headless-first rendering: offscreen render targets as the primary path

**Status:** Accepted (Phase 2)

**Context.** The renderer must run on CI machines with no GPU, no window system and no display server, while still producing visual evidence (screenshots) on every push (Global Rule 2, ADR-0004). Window-surface presentation is not needed until the Phase 7 editor.

**Decision.** `gens-renderer` is designed **offscreen-first**: the primary render path draws into an engine-owned color+depth texture and reads pixels back to CPU memory (PNG-encodable) with no window, no surface and no display server dependency. Window-surface presentation is layered on top in a later phase via `raw-window-handle`; nothing in the core render API assumes a surface exists. Adapter selection never requires a `compatible_surface`.

**Alternatives considered.**
- *xvfb + GL surface on CI* — rejected: fragile display-server emulation, hidden state, slower, and does not map to Vulkan/Metal/DX12.
- *Surface-first with headless mode bolted on* — rejected: two code paths to maintain; the surfaceless path is the strictly more general one.

**Risks & mitigations.** Readback cost (GPU→CPU copy) is irrelevant at CI scene sizes; presentation path stays untested until Phase 7 → flagged in RISKS as deferred surface work; SRGB/framebuffer format choices fixed now (`Rgba8UnormSrgb` + `Depth24Plus`) are documented in code and can be revisited by ADR if a need appears.

---

## ADR-0011 — Software-rendering policy for CI: guaranteed lavapipe on Linux, graceful degradation elsewhere

**Status:** Accepted (Phase 2)

**Context.** ADR-0003 commits to a software fallback when no hardware adapter exists, but CI has three different graphics stories: GitHub `ubuntu-latest` has no Vulkan ICD by default, `macos-latest` has real Metal (Apple silicon), and `windows-latest` has only the Microsoft Basic Render Driver whose DX12 feature level is not guaranteed.

**Decision.** The CI workflow **installs Mesa's lavapipe** (`mesa-vulkan-drivers`) on Linux and exports `VK_ICD_FILENAMES` (plus `vk_swiftshader`-style explicit ICD selection) so a real software Vulkan device always exists there. GPU integration tests are marked `#[ignore]` in the normal suite and executed by a dedicated CI step on Linux and macOS (both have guaranteed adapters: lavapipe / Metal). Windows runs the same step best-effort: if no adapter is found the step prints the adapter probe report and is skipped honestly (documented in the run log) rather than failing — the 3-OS guarantee applies to build/clippy/unit tests, while GPU-render guarantees are Linux+macOS until a Windows software adapter is proven stable.

**Alternatives considered.**
- *SwiftShader on all platforms* — rejected: not maintained for Windows runners; WARP is the Windows-native equivalent but unstable to configure via wgpu.
- *Fail CI when Windows has no adapter* — rejected: a platform's missing software driver is not a code regression; failing would train the team to ignore red.

**Risks & mitigations.** "Skipped honestly" can drift into "skipped silently" → the Windows step must print its probe evidence every run, and the phase report states exactly which OS rendered GPU frames; lavapipe behavior drift across Mesa versions → smoke expectations are structural (pixel classes, not exact bytes) on CI, with byte-exact determinism asserted only within a single machine run.

---

## ADR-0012 — Binary size gate recalibration: 350 MiB on the wgpu-linked debug example (Phase 2 baseline)

**Status:** Accepted (Phase 2)

**Context.** ADR-0009's 160 MiB gate watched the Phase 1 `hello_world` debug binary (79.8 MiB). Phase 2 added the wgpu renderer; the GPU smoke example `gpu_hello` (gens-core + gens-ecs + gens-renderer + wgpu 30, debug profile) measures **171.3 MiB** on Linux x86_64 (rustc 1.99.0) — the 160 MiB gate would now fail on the engine's own smoke binary, which is the gate working, but on a threshold that no longer reflects a regression: wgpu alone costs ~90 MiB in debug.

**Decision.** The gate target moves to the heaviest example (`gpu_hello`) with **350 MiB** (2x the new measured baseline), checked on all three platforms. `hello_world` (80.0 MiB) remains built and run as the CPU smoke but is no longer the gated binary. Future thresholds move by ADR only.

**Alternatives considered.**
- Gating release binaries instead — deferred again: release LTO sizes become meaningful in Phase 8 (build pipeline); debug gates catch accidental dependency explosions earlier.
- Excluding wgpu from the gate — rejected: hiding the renderer's weight defeats the gate's purpose.

**Risks & mitigations.** Debug sizes vary across platforms (Windows PDBs live beside the binary; macOS dSYM not extracted) → the gate watches the executable file itself; a future dependency that doubles the binary trips the gate → revert-first per `docs/ROLLBACK_PLAN.md`, then recalibrate with evidence.

---

## ADR-0013 — Physics: rapier3d behind a `gens-physics` facade

**Status:** Accepted (Phase 3)

**Context.** The engine needs rigid-body dynamics (bodies, colliders, fixed-step integration) that is pure Rust (no FFI toolchain on three CI platforms), headless-testable, and deterministic enough for replays.

**Decision.** Integrate `rapier3d` **0.36.1** (pinned) behind `gens-physics`: a `PhysicsWorld` facade with value-type `BodySpec`/`ColliderSpec`, explicit fixed-timestep stepping (`step()` only — never wall time), and ECS glue (`PhysicsResource` newtype resource, `PhysicsBody` component, step + transform-sync systems writing the standard `Transform`). All values cross the rapier boundary through tested converters in one module: rapier 0.36 resolves to parry's `glamx` (glam 0.33.12) while gens standardizes on glam 0.34.1, so the types are distinct despite identical layout. The raw crate stays reachable as `gens_physics::backend` (the `gens-ecs::ecs` convention). The determinism contract is **in-process bit-identical replay** (asserted over 600 steps with periodic impulses); cross-platform bit-identity is NOT claimed (R-013), and CI logs per-OS final positions as evidence.

**Alternatives considered.**
- *PhysX / other C++ physics via FFI* — rejected: build-system and safety burden on three platforms contradicts ADR-0001.
- *Custom solver* — rejected: years of work, zero product differentiation for our goals.
- *Avian/bevy_xpbd (bevy-ecosystem physics)* — rejected: deep bevy-app coupling; gens uses standalone bevy_ecs behind a facade (ADR-0002).

**Risks & mitigations.** rapier version churn → exact pin + facade confines churn (R-019); rapier's integrator is a version-internal detail → tests pin Newtonian kinematics (v = g·t exact, y within 5 cm of ½g·t²), not solver coefficients; fast movers tunnel → CCD flag on `BodySpec`, exposed and used by the demo; no rolling resistance by default → `linear_damping` exposed on `BodySpec`.

---

## ADR-0014 — Audio: kira with headless-first mock processing; realtime output deferred

**Status:** Accepted (Phase 3)

**Context.** The audio pillar must be verifiable in CI, which has no audio devices and (in our sandbox) no ALSA headers for building cpal. Device output is only useful once a windowing/runtime shell exists.

**Decision.** Integrate `kira` **0.12.5 with `default-features = false`** behind `gens-audio`: an `AudioEngine` over kira's **MockBackend** whose real mixer/renderer advances deterministically — one `process()` tick = exactly 128 frames (asserted). Sounds are synthesized from raw `Frame`s (`tone` module) so no decoder features and no binary assets are needed. The realtime backend (kira's cpal feature) is a deliberate later addition as a cargo feature on `gens-audio`; nothing in the facade changes that day. Sample-level output capture is not exposed by the mock backend (its output buffer is private), so the honest Phase 3 verification surface is the scheduling/state machine (play/pause/resume/stop/loop/positions) plus synthesized-data contracts.

**Alternatives considered.**
- *rodio* — rejected: playback-sink oriented, no mixer graph or state introspection to verify.
- *cpal alone* — rejected: too low-level (device I/O only; we would rebuild mixing, tweens, scheduling).
- *Enabling cpal now* — rejected: ALSA headers unavailable in CI/sandbox; a build that needs system packages we cannot install contradicts the CI-first rule.

**Risks & mitigations.** Realtime output unproven until the windowing phase → recorded as R-018 with the feature-flag path; kira churn → exact pin + `backend` escape hatch (R-019); mock `sample_rate` defaults to 1 Hz if unset → facade asserts 44.1 kHz default and threads the rate through one constructor.

---

## ADR-0015 — Input: pure action-mapping core; device backends and ECS glue land with windowing

**Status:** Accepted (Phase 3)

**Context.** Gameplay must be written against rebinding-able actions, not raw keys, and the input model must be CI-testable without devices. But no window exists in Phase 3, and bevy_ecs 0.20 (like bevy 0.16+) requires `Resource: Component`, which a generic `InputState<A>` cannot satisfy without forcing user action enums to derive `Component`.

**Decision.** `gens-input` is a **pure, bevy-free crate**: engine-owned `KeyCode`/`MouseButton`/`DeviceEvent` vocabulary (`#[non_exhaustive]`), `InputMap` (actions → bindings, OR-combined, add/unbind/clear), `InputState` fed by a `DeviceEvent` stream with explicit end-of-frame `update()` rotating just-pressed/released edges, axis helpers, and focus-loss `clear_all`. Device backends (winit keyboard/mouse, gilrs gamepads) will translate native events into `DeviceEvent`s when the windowing phase lands — the action model does not change. ECS integration arrives then too, as a newtype resource wrapper (the same pattern `gens-physics::PhysicsResource` already uses), precisely because of the `Resource: Component` constraint.

**Alternatives considered.**
- *Adopting winit event enums now* — rejected: couples the action model to a windowing library before any window exists.
- *gilrs gamepads now* — rejected: CI has no gamepad devices; nothing honest to assert.
- *Generic `Resource` impl via derive on user enums* — rejected: forces `Component` bounds on every game's action enum (leaks bevy into gameplay code).

**Risks & mitigations.** Key vocabulary growth (F-keys, numpad, IME) → `#[non_exhaustive]` + backend translations stay in one module; edge-ordering bugs ("update before query" swallows just-pressed) → documented frame contract in the crate docs, and the physics demo's behavioral tests caught exactly this bug class in practice.

## ADR-0016 — Animation: pure deterministic clip/blend/controller core; skinning and glTF import deferred

- **Status:** Accepted (Phase 4)
- **Context:** Phase 4 needs skeletal animation, blending and state machines (README roadmap) that compose with the replay-tested simulation discipline. Mesh-space skinning needs a skinned render path, and clip import needs an asset pipeline — neither exists before Phase 5/8.
- **Decision:** `gens-animation` is a pure `(state, time)` core: flat cycle-free `Skeleton` (parents strictly earlier indices), TRS `JointPose` + forward kinematics, keyframe tracks (linear/step/shortest-arc slerp, sorted + validated), `Clip` per-joint channels, pose blending (`blend_into`, `crossfade`), and an `AnimationController` state machine whose conditions are **data** (declared parameter comparisons), not closures. No wall clocks, no hidden state; looping wraps `time % duration`. Controllers are replay-tested bit-for-bit.
- **Consequences:** glTF/mesh skinning arrive with the assets phase on top of this sampling layer; determinism carries the R-013 in-process scope; slerp is shortest-arc via glam (unit quaternions double-cover rotations — endpoints canonicalize, documented in tests).

## ADR-0017 — UI: egui 0.36 immediate mode behind a pure headless core; gens-renderer backend instead of egui-wgpu

- **Status:** Accepted (Phase 4)
- **Context:** The delegation shortlists egui/Slint. The engine pins wgpu 30.0.1; `egui-wgpu` pins its own wgpu minor, which would put two wgpu trees in the lockfile (the Phase 3 rapier dual-glam lesson). CI needs UI without windows or system fonts.
- **Decision:** `gens-ui` wraps egui 0.36.2 in two layers: (1) a **pure headless core** — neutral `UiEvent` streams in, fully tessellated `UiFrame`s out (`ctx.run_ui` → `ctx.tessellate`), pixels-per-point pinned to 1.0, engine simulation time only, texture deltas emitted sorted (egui's delta maps are `HashMap`s) and acknowledged (`clear()`, epaint 0.36 asserts); (2) a **wgpu backend in gens-renderer** (`ui.rs`): vertices converted to NDC on the CPU (no uniforms in the pipeline), premultiplied over-blending (one / one-minus-src-alpha), per-batch scissor, color-only pass. egui's premultiplied-sRGB vertex colors are decoded to linear at the render boundary — the Phase 2 convention (P2-8) — pinning opaque-color byte round-trips, proven by GPU tests on lavapipe/Metal.
- **Consequences:** egui types surface in `gens-ui`'s public closures (the facade confines the dependency); exact antialiased-edge compositing matches egui-backend convention rather than strictly linear-light premultiplied math (opaque colors are exact; documented honestly); embedded fonts (`epaint_default_fonts`) remove the system-font CI dependency; windowing/event backends later feed the same `UiEvent` stream.

## ADR-0018 — AI: hand-rolled behavior trees + grid A* with integer-quantized determinism; no external deps

- **Status:** Accepted (Phase 4)
- **Context:** The stub promises "navigation, pathfinding and behavior trees". Mature crates exist, but the engine's defining constraint is replayable determinism (R-013), and the needed scope (classic BT node set + grid A* + steering) is small enough to own.
- **Decision:** `gens-ai` hand-rolls: `NodeStatus`/`Blackboard`/`Behavior` with memory-based Sequence/Selector (finished children are not re-ticked — pinned by tick-counting tests), Parallel (All/Any), Inverter, Repeat, Wait, Action/Condition leaves; a `Grid` (walkable/cost cells) with A* using **integer-quantized priorities** (cost·1024 → u32; total order `(f, h, index)`) and fixed neighbor expansion order (N/E/S/W then diagonals, corner-cut prevention); stateless `Steering` (seek/flee/arrive/follow_path with a documented overshoot limitation) plus a stateful forward-only `PathFollower`.
- **Consequences:** no upstream churn risk for AI; navmesh and hierarchical pathing are future work on the same contracts; identical grids always yield identical paths (asserted in tests); in-process determinism scope per R-013.

## ADR-0019 — Scripting: sandboxed Rhai behind a value-snapshot entity API; entity-index execution order

- **Status:** Accepted (Phase 5)
- **Context:** Phase 5 needs scripting that composes with the replay-tested simulation discipline (R-013). Rhai is pure Rust (no C boundary), MSRV 1.66, and its default features carry `ahash/runtime-rng` — runtime-seeded hashing. `Engine::new()` also installs a `FileModuleResolver`, and `to_int`/`sqrt`/`floor` live in packages. bevy resources require `Send + Sync` native functions (rhai's `sync` feature).
- **Decision:** pin rhai 1.26.1 with `default-features = false, features = ["std", "sync"]` (kills runtime-rng, verified via `cargo tree -e features`; `Map` is a `BTreeMap` regardless). `ScriptEngine` builds on `Engine::new_raw()` — no packages, no module resolver — plus hard limits (100 k operations, 32 call levels/expr depth, 1024 map/array, 8 KiB strings). The host registers the ONLY non-pure surface: a closed math vocabulary (`sqrt/floor/ceil/abs/min/max/to_int/to_float` — rhai has no cast operator) and the per-tick entity API reading a value-snapshot `ScriptFrame` (`time/dt/pos/rot/vars` in; `out_pos`, `ScriptRequests` out) published through an `Arc<Mutex>` slot; use is strictly sequential. `run_scripts` snapshots entities and ticks them in **entity-index order**; writes apply only on success. `ScriptVars` is a `BTreeMap<String, ScriptValue>` (Number/Flag/Text) with kind-checked writes.
- **Consequences:** scripts see no clocks, no RNG, no IO — identical frames replay bit-for-bit (pinned in-process per R-013); the API is deliberately narrow (requests are DATA; the game applies them); upstream churn is contained by the raw-engine surface being tiny; `internals` stays off — results cross the boundary as `Dynamic` and are type-checked at the wrapper edge.

## ADR-0020 — Assets: identity = FNV(canonical key) + content hash; glTF subset with loud rejection

- **Status:** Accepted (Phase 5)
- **Context:** The engine needs content loading whose identity is reproducible across machines and restarts (saves, manifests, evidence). glTF 2.0 is large; skins/clips must land in gens-animation's Phase 4 data model without changes; CI fixtures must be self-contained and auditable.
- **Decision:** `AssetId` = FNV-1a 64 of the canonical key (the engine's evidence-hash primitive); `AssetRegistry` is an id-ordered `BTreeMap` whose version bumps ONLY when the content hash changes; kind-change, id-collision, empty-content and malformed keys are errors. glTF import accepts an explicit subset — TRS nodes, single-primitive triangle meshes (JOINTS_0/WEIGHTS_0 parsed and stored for the future skinned path), skins as `Skeleton` + inverse-bind matrices + joint-node mapping, animations as `Clip`s bound to one skin with LINEAR/STEP samplers — and rejects CUBICSPLINE, morph targets, matrix nodes, morph-weight channels and multi-primitive meshes with `UnsupportedGlTF` diagnostics. The committed fixture is generated by a committed Python script (byte-stable float formatting).
- **Consequences:** same bytes → same ids/hashes/objects, always; unsupported content fails loudly instead of importing wrong data; textures/materials/MorphTargets stay unimported until a consuming phase needs them; the skinned GPU render path (ADR-0016's reservation) can adopt the stored joints/weights without re-import.

## ADR-0021 — Save: mirror-type versioned document, canonical entity order, atomic + hash-verified

- **Status:** Accepted (Phase 5)
- **Context:** World persistence must be byte-deterministic (saves as evidence), crash-safe, and decoupled from engine internals so the format can evolve. bevy 0.16+ stores resources AS entities (they occupy entity slots); `WorldTransform` is derived state.
- **Decision:** a save is `GENSSAVE1\n` + JSON body + `GENSHASH fnv1a64(body)\n`, all verified on load, written atomically (temp + rename). The body uses MIRROR types (`SavedTransform` arrays, `SavedScript {script, entry, vars, requests}`) — not live ECS types. `capture_world` sorts entities by index (archetype order is not deterministic); `apply_save` spawns into a FRESH world; derived state (`WorldTransform`, animation poses) is never saved and re-derived after load. Scope: the engine-standard component vocabulary (name/active/tag/transform/script) — game state that must survive save/load travels through `ScriptComponent::vars`; a reflection/registry layer for arbitrary components is future work, stated here rather than faked.
- **Consequences:** identical world state → byte-identical files; the save/load continuation property (fresh world + save + remaining steps == uninterrupted run) is pinned by `asset_script_hello` on every push across three OS; resource entities carry no saveable components and are excluded by construction; format evolution starts at `GENSSAVE1` versioning with a loud `BadVersion` rejection.

## ADR-0022 — Networking: quinn/QUIC loopback-first behind a synchronous facade

**Status:** Accepted (Phase 6)

**Context.** The README roadmap gives Phase 6 "Networking + data assets"; the engine itself is synchronous (pollster-style blocking at the wgpu boundary) and CI runs on three operating systems with no external network contract. Real-socket verification is required — a mock-only transport would prove nothing about QUIC on Windows/macOS/Linux runners.

**Decision.** Transport = quinn 0.11 (QUIC over UDP) pinned to the `rustls-ring` provider, paired with rcgen `ring` for in-memory self-signed loopback certificates. Every endpoint owns a dedicated I/O thread whose tokio `current_thread` runtime is driven for the endpoint's whole lifetime; the public API is synchronous (`send_frame`/`recv_frame`/`close`) with explicit timeouts. Three constraints are load-bearing and documented in the module: quinn transmits only while its runtime is driven; QUIC announces streams on the first written byte (hence the link's announce frame); runtime drop cancels queued teardown frames (hence graceful `Shutdown` + join).

**Consequences.** The engine never sees async; tokio is confined to gens-networking internals. Loopback-only is a stated scope (R-025): NAT traversal, congestion profiles and hardening are future work. Frames are FNV-verified independent of TLS (integrity discipline identical to assets/save). One caller per endpoint at a time; per-link FIFO managers preserve ordering.

## ADR-0023 — Data assets: versioned JSON scene documents on the registry

**Status:** Accepted (Phase 6)

**Context.** Phase 5's report pointed data-driven content at the registry identity discipline (FNV key + content hash + version). A scene format needs to cover the engine-standard component vocabulary without becoming a reflection system (R-023), and without coupling the asset crate to the scripting engine.

**Decision.** `DataDocument` (JSON, `format_version` 1): entities in authored order — name (unique), `active`, optional `tag`, TRS (unit-quat tolerance 1e-4, non-zero finite scale), optional script binding with a mirror variable vocabulary (`Number(f64)`/`Flag`/`Text`). Validation is loud and happens BEFORE registry registration; canonical JSON is byte-stable (struct order + `BTreeMap`). The `.json` extension maps to `Data` (the previously unused `UiLayout` kind keeps `.uilayout`). Instantiation is a runtime bridge (`gens-runtime::data`) symmetric with the save format, byte-equal to it by construction.

**Consequences.** Scenes are pure functions of content (hashable, replicable, evidence-friendly). Game-specific components still travel through script variables — the reflection/registry layer remains future work (R-023), stated honestly rather than faked.

## ADR-0024 — Session protocol: strict vocabulary, version-pinned handshake, independent hash

**Status:** Accepted (Phase 6)

**Context.** The transport moves verified frames; sessions need identity (who am I talking to, which protocol), semantics (what is in a frame) and end-to-end integrity that survives a transport swap.

**Decision.** Handshake: client `HELLO "GENH"+version` → host `WELCOME "GENW"+version+session_id` or `REJECT "GENR"+reason`; version mismatches are loud on both sides. Envelopes: `tick u64 | kind u8 | payload_len u32 | payload_hash u64 | payload` with a STRICT kind vocabulary (Snapshot/Input/Bye) — unknown kinds are protocol errors, deliberately without a `Custom(u8)` escape. The payload hash is computed by the session layer independently of the transport's per-frame hash.

**Consequences.** Two builds that disagree about the protocol fail at handshake or at the first unknown kind instead of corrupting each other. The double hash costs 8 bytes per message and a recompute per receive — negligible at engine message rates and deliberate (belt and suspenders). Ticks and session ids are caller/host-assigned: the session layer invents no identity and reads no clock.

## ADR-0025 — Visual editor: egui offscreen-first, two-renderer composition, real vocabulary only

**Status:** Accepted (Phase 7)

**Context.** The README roadmap gives Phase 7 "Visual editor" and the archived reference design (`docs/design/REFERENCE.png`) shows a full docked-panel editor (hierarchy, 3D viewport with gizmo, inspector, asset browser, console, status bar). CI runs on three OSes with no display; the engine has no windowing layer yet; the reference mockup also shows fields (physics mass/friction/restitution, shadow toggles) the engine's real component vocabulary does not have.

**Decision.** The editor is an egui 0.36 application composed entirely offscreen, per frame: (1) a dedicated viewport `Renderer` renders the 3D scene from an orbit camera into its own offscreen target and reads the pixels back; (2) those pixels enter egui as a managed texture (`load_texture` + `ui.image`) inside the center dock; (3) a dedicated window `Renderer` + `UiRenderer::render` draws the whole editor chrome over it and returns the final RGBA — the artifact of record (PNG + FNV hash). Panels mirror real engine state only: hierarchy = actual ECS tree; inspector = actual components (Name/Active/Transform/PhysicsBody+specs); asset browser = actual `AssetRegistry` entries; console = actual `tracing` events captured by a ring-buffer layer. Where the mockup shows vocabulary the engine lacks (mass/friction/restitution), the panel omits it and the omission is documented — no invented fields. Determinism contract: ppp pinned to 1.0, deterministic clock, scripted input replay; two runs must be byte-identical.

**Consequences.** CI can exercise and snapshot the real UI on all three OSes (lavapipe/Metal/DX12) exactly like `gpu_hello`; the native window shell (winit) is explicitly future work — when it lands it must only *feed input events and present pixels*, not restructure the UI. The two-renderer split costs one extra adapter instance in CI (software) and keeps viewport resolution independent of window size. The inspector's edit surface is limited to the real component vocabulary; arbitrary-component reflection remains R-023.

## ADR-0026 — UI acceptance: hash gates alone are blind, snapshots must be inspected and geometry pinned

**Status:** Accepted (Phase 7)

**Context.** Phase 7's CI was green on run #89 while the committed snapshot had an empty 3D viewport (the camera matrix was never staged) and an empty, mis-placed console (a manual `allocate_ui` split put panels in the wrong half and ~110 px too low). Frame-hash determinism — the discipline since Phase 2 — proves a run equals *itself*, not that the pixels are *right*. Only pixel-level inspection of the artifact revealed both defects; a third bug (#90, a single `run_frame` call) was masked on Windows by a best-effort skip branch.

**Decision.** For every phase whose deliverable renders UI: (1) the CI snapshot is a *reviewed artifact* — it is downloaded and inspected before a phase may merge, not just hashed; (2) panel geometry is pinned by headless tessellation tests that assert *where* content lands (column, y-range, bounds), not merely that frames hash equal; (3) best-effort platform branches must mark skips with a distinctive log marker and must never swallow a failing example as "skip" without that marker being auditable; (4) renderer state that shaders consume (camera, lights) is staged explicitly and listed in the render-path checklist alongside draws.

**Consequences.** Merging a UI phase now requires human (or agent) eyes on the PNG — an explicit, cheap step that caught two silent defects this phase. Geometry tests are headless (no adapter) so they gate every push on every OS. The determinism contract (ADR-0025) is unchanged; it simply stops being the *only* line of defense.
