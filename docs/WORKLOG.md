# Gens — Work Log

Append-only engineering log. Every sub-item that changes this repository gets an entry here before its commit is created (Phase 0 §0.1). The entry is committed together with the change it describes.

Entry format:

```
### [<UTC timestamp>] [phase] <short title>
- WHAT: ...
- WHY: ...
- EVIDENCE: commit <sha7>, CI run <url-or-status>, metric/screenshot path
```

Rules:

- Never rewrite or delete past entries (append-only).
- One entry per logical change / commit.
- Every entry references evidence: commit SHA, CI run, or a metric/screenshot path under `docs/evidence/`.
- If a push could not be performed, the entry says so explicitly — pushes are never silently skipped.

---

### [2026-10-11T17:20Z] [phase0] environment protection — fresh clone and branch setup
- WHAT: Cloned `https://github.com/salim77007j/gens.git` (public, empty repository — verified via `git ls-remote` exit 0 with zero refs and clone warning "empty repository"). Configured repo-local identity (salim77007j / noreply email), `commit.gpgsign=false`, and a credential helper that reads `GH_TOKEN` from the process environment at push time (no credential is stored in `.git/config` or any file). Created phase branch `phase0-risk-management`.
- WHY: Phase 0 §0.1 — start from a verified fresh clone; §0.2 — dedicated branch per phase; security — the push credential must never be written to disk or the repo.
- EVIDENCE: `docs/evidence/phase0/clone-verify.txt`, `docs/evidence/phase0/environment.txt` (committed in the evidence sub-item), `docs/evidence/phase0/auth-check.txt` (push credential status).

### [2026-10-11T17:21:00Z] [phase0] bootstrap commit (C1)
- WHAT: First commit on `phase0-risk-management` — `.gitignore` (Cargo.lock intentionally tracked), `.editorconfig`, `LICENSE-MIT`, `README.md` (vision, architecture plan, roadmap, process), `docs/WORKLOG.md` scaffold. Push attempted with the provided credential.
- WHY: Phase 0 §0.4 — single logical change per commit; establishes the repo foundation before any engine content.
- EVIDENCE: commit e194fd0 (root commit). PUSH FAILED: `remote: Invalid username or token` — full transcript in `docs/evidence/phase0/auth-check.txt`.

### [2026-10-11T17:21:26Z] [phase0] environment evidence capture (C2)
- WHAT: Added `docs/evidence/phase0/` — `environment.txt` (platform, toolchain, network, clock), `clone-verify.txt` (remote URL credential-free, empty-repo verification, branch state), `auth-check.txt` (401 diagnosis of the supplied token + mitigation + security note). Second push attempt failed identically; further per-commit push attempts are suspended as futile with the current credential and will be retried once a valid token is provided.
- WHY: Phase 0 §0.1 — "Save ALL evidence to the repo"; transparency about the credential blocker instead of silent push skipping.
- EVIDENCE: this commit (C2); auth failure transcripts in `docs/evidence/phase0/auth-check.txt`; commits e194fd0..HEAD are local until credentials arrive.

### [2026-10-11T17:22:07Z] [phase0] governance docs — DECISIONS + RISKS (C3)
- WHAT: `docs/DECISIONS.md` with ADR-0001..0008 (workspace layout, bevy_ecs facade, wgpu, CI-first, branch/tag discipline, licensing, governance set, public repo). `docs/RISKS.md` with register R-001..R-016 including detail notes; R-001 marked **Realized** (invalid push credential) with mitigations and user actions.
- WHY: Phase 0 §0.9 — every architectural decision and risk documented with alternatives and mitigations; §0.4 — the credential blocker is a fact to record, not hide.
- EVIDENCE: this commit; R-001 evidence at `docs/evidence/phase0/auth-check.txt`.

### [2026-10-11T17:22:25Z] [phase0] rollback plan + experiment policy (C4)
- WHAT: `docs/ROLLBACK_PLAN.md` (7-scenario playbook with time budgets, pre-change checklist, environment-loss recovery, secret-in-history procedure, drill definition) and `docs/EXPERIMENTS.md` (experiment policy + log template).
- WHY: Phase 0 §0.6 (5-minute rollback guarantee) and §0.5 (isolated, documented experiments).
- EVIDENCE: this commit; drill executed later in this phase records to `docs/evidence/phase0/rollback-drill.txt`.

### [2026-10-11T17:22:38Z] [phase0] CI quality gates (C5)
- WHAT: `.github/workflows/ci.yml` — docs-integrity gate (governance files present), secret-scan gate (GitHub token patterns), and a 3-OS matrix (ubuntu/windows/macos) with fmt, `clippy -D warnings`, build, test, headless-smoke and size-gate steps that self-detect the (not yet existing) workspace and skip cleanly. Concurrency cancellation, least-privilege `permissions: contents: read`.
- WHY: Phase 0 §0.7 — automated quality gates on every push; gates are ARMED in Phase 0 and become ACTIVE in Phase 1 when `Cargo.toml` exists, so the workflow never blocks the process bootstrap.
- EVIDENCE: this commit; first real workflow runs will occur on the first successful push (credential blocker recorded in R-001).

### [2026-10-11T17:22:51Z] [phase0] build-safety tooling (C6)
- WHAT: `scripts/verify.sh` (executable local pre-flight mirroring every CI gate; exits PASS on the current tree) and `rust-toolchain.toml` (stable channel + rustfmt + clippy, exact pin deferred to Phase 1 per ADR-0001).
- WHY: Phase 0 §0.3/§0.8 — compile/test/size checks before every change; local and CI gates must be the same gates.
- EVIDENCE: `docs/evidence/phase0/verify-run.txt` (exit 0, full output); commit 09619a1.

### [2026-10-11T17:23:01Z] [phase0] design reference archived (C7)
- WHAT: `docs/design/REFERENCE.png` — the editor design reference supplied for Phase 7 (dark-theme game-editor UI: scene hierarchy, 3D viewport, inspector, asset browser, developer console).
- WHY: Phase 0 §0.1 — all inputs preserved in-repo so Phase 7's design analysis is possible after any environment reset.
- EVIDENCE: commit 562ecc7; detailed design-language extraction deliberately deferred to Phase 7 (R-016).

### [2026-10-11T17:23:07Z] [phase0] rollback drill (C8/C9)
- WHAT: Scenario #1 of `docs/ROLLBACK_PLAN.md` rehearsed for real: marker commit 62d5ff1 created, then instantly reverted by commit 90096b8 (`git revert --no-edit`). Local wall-clock elapsed: 0 seconds.
- WHY: Phase 0 §0.6 — the 5-minute rollback guarantee must be demonstrated, not asserted.
- EVIDENCE: commits 62d5ff1 + 90096b8; timings recorded in `docs/evidence/phase0/rollback-drill.txt` (finalized in the report commit, including the push component pending credentials).

### [2026-10-11T17:23:30Z] [phase0] worklog timestamp correction
- WHAT: Earlier entries carried hand-written timestamps that drifted ahead of the real clock (up to +13 minutes). All entry timestamps corrected to the exact `git log` commit times, which are the authoritative source.
- WHY: Honesty rule — a process log whose times contradict git history is defective evidence; fixing immediately and disclosing the fix beats silently leaving it.
- EVIDENCE: `git log --format='%h %ad %s' --date=iso` matches every timestamp in this file after this commit.

### [phase0] phase report + finalized drill evidence (C10)
- WHAT: `docs/PHASE_0_REPORT.md` (requirement map §0.1–0.10, commit ledger, CI status, drill results, honest exit-criteria scorecard, credential-blocker diagnosis + 5-minute resume procedure) and finalized `docs/evidence/phase0/rollback-drill.txt`.
- WHY: Phase deliverable — a detailed phase report with before/after evidence; the report deliberately does NOT claim CI-green or pushed state while R-001 blocks them.
- EVIDENCE: this commit; phase status "COMPLETE LOCALLY — PUSH BLOCKED BY INVALID CREDENTIAL" per report §8/§9/§10.

### [phase0→1] correction: credential diagnosis was a harness flaw
- WHAT: Phase 0 completed on origin (branch pushed, CI green runs #1/#2, main bootstrapped, pre-phase1-safe tag). Correction addenda added to docs/evidence/phase0/auth-check.txt, docs/RISKS.md (R-001 reclassified, new R-017) and docs/PHASE_0_REPORT.md §13: the earlier 401s came from non-persistent shell env between tool calls (empty credential presented), not necessarily an invalid token.
- WHY: Honesty rule — the record must not stand with a wrong diagnosis when the truth is known; the operational lesson (same-invocation credentials) is captured as R-017.
- EVIDENCE: CI runs 38162440721 / 38162508120 green; addenda in the three files; first commit of phase1-core.

### [phase1] workspace scaffolding (P1-2)
- WHAT: Root workspace manifest + 15 stub crates per ADR-0001; workspace lints (unsafe forbid, clippy) and release profile.
- WHY: Phase 1 task 1 - the workspace architecture is the backbone every later phase builds on.
- EVIDENCE: cargo build/clippy green locally; verify.sh PASS; this commit.

### [phase1] gens-math implemented (P1-3)
- WHAT: glam facade + engine utilities + 11 unit tests; one test expectation corrected during authoring (documented in commit message).
- WHY: Phase 1 task 4.
- EVIDENCE: cargo test -p gens-math: 11 passed; clippy -D warnings clean; this commit.

### [phase1] gens-core errors + logging (P1-4)
- WHAT: error and logging modules with 4 unit tests; deps (thiserror/anyhow/tracing/tracing-subscriber) centralized in workspace.dependencies.
- WHY: Phase 1 task 2 (error handling, logging).
- EVIDENCE: cargo test -p gens-core green; clippy -D warnings clean; this commit.
### [phase1] event bus semantics fix (P1-6b)
- WHAT: Events<E> rewritten to snapshot/take semantics after 5 red tests exposed the double-buffer contract violation; process flaw (heredoc breaking && gates) identified and corrected.
- WHY: Correctness of the documented event contract; honesty about how the red commit reached the branch.
- EVIDENCE: 15/15 tests green; commit above; CI on branch will confirm.
### [phase1] fixed-timestep game loop (P1-7)
- WHAT: advance() pure math + GameLoop driver, 7 tests (22 total in gens-core); two authoring fixes documented in the commit (nanos truncation, const fn).
- WHY: Phase 1 task 2 (fixed-timestep loop, variable render, catch-up guard).
- EVIDENCE: 22/22 gens-core tests green; clippy/fmt clean; this commit.
### [phase1] gens-ecs facade (P1-8)
- WHAT: components, hierarchy helpers, exclusive propagation, EngineSchedules, prelude; 15 tests total incl. doc test; bevy_ecs 0.20.0 pinned.
- WHY: Phase 1 task 3 (bevy_ecs integration, core components, hierarchy, transform propagation).
- EVIDENCE: tests green; clippy clean; this commit.
### [phase1] snapshot + hello_world example (P1-9)
- WHAT: snapshot module + example with 5 tests; deterministic PNG evidence archived; two clippy-1.99 lints fixed before commit.
- WHY: Phase 1 task 6 (hello world: one entity, loop runs, screenshot on CI).
- EVIDENCE: docs/evidence/phase1/hello_world.png; determinism proof (byte-identical reruns); workspace tests green.
### [phase1] CI activation (P1-10)
- WHAT: smoke test + artifact upload + 160 MiB size gate active on all 3 OS; ADR-0009 records the calibration.
- WHY: Phase 1 task 5 (CI: build 3 OS, tests, clippy, fmt, headless smoke, size gate).
- EVIDENCE: this commit; branch CI run will confirm (polled before merge).
### [phase1] phase docs (P1-11)
- WHAT: PHASE_1_REPORT.md (delivery summary, 53-test scorecard, honest issue log, numbers), README status/roadmap/build updates, RISKS updates (R-015 Mitigated, R-001 Corrected), test+size evidence files.
- WHY: Phase deliverable - detailed report with evidence; roadmap reflects reality.
- EVIDENCE: docs/evidence/phase1/{test-output.txt,example-run.txt,hello_world.png}; CI run URLs recorded at merge.

### [phase2] branch opened; governance defect fixed; headless-first ADRs (P2-0)
- WHAT: Branch `phase2-renderer` opened from verified `main` (f3ac0c4). Fixed a governance defect found in the Phase 2 review: ADR-0009 appeared twice in `docs/DECISIONS.md` (byte-different duplicates from the Phase 1 merge); the second copy removed, dedup disclosed here. Added ADR-0010 (offscreen-first rendering, no surface dependency) and ADR-0011 (CI software-rendering policy: guaranteed lavapipe on Linux, real Metal on macOS, honest best-effort on Windows) to govern the renderer work about to land.
- WHY: Phase 2 entry point; the honesty rule requires defects in the record to be fixed and disclosed, not papered over; the two new ADRs are load-bearing for every Phase 2 commit that follows.
- EVIDENCE: this commit; `git log -p docs/DECISIONS.md` shows exactly one ADR-0009 after the fix.

### [phase2] GPU context + adapter selection policy (P2-1)
- WHAT: `gens-renderer/context.rs`: `AdapterClass` (Discrete > Integrated > Virtual > Unknown > Software), pure `classify`/`pick_best` policy functions, `GpuContext` (instance -> enumerate -> classify -> request device+queue, no surface anywhere — ADR-0010). wgpu 30.0.1 + pollster 1.0.1 pinned in `[workspace.dependencies]`. 6 unit tests (policy matrix, tie-breaks, empty-safe, class ordering) + 1 `#[ignore]` GPU integration test for the CI driver step (ADR-0011). wgpu 30 API facts verified from registry source before coding (InstanceDescriptor::new_without_display_handle, DeviceDescriptor{experimental_features,trace}, PollType::wait_indefinitely era APIs).
- WHY: Phase 2 task 1 — the device/adapter layer is the foundation every later renderer module draws through; ADR-0003 mandates the selection policy.
- EVIDENCE: cargo test -p gens-renderer 6 passed/0 failed/1 ignored; clippy -D warnings clean; sandbox has no sudo so lavapipe could not be installed locally — the ignored test runs for real in CI (Linux lavapipe, macOS Metal); this commit.

### [phase2] offscreen render target with readback (P2-2)
- WHAT: `gens-renderer/target.rs`: `RenderTarget` (Rgba8UnormSrgb color + Depth24Plus depth, RENDER_ATTACHMENT|COPY_SRC) with GPU->staging copy (`copy_color_to_staging`) and blocking `read_pixels` (map callback via channel, padding stripped). Pure `bytes_per_row` (256B WebGPU row alignment, overflow-checked) and `unpack_padded_rgba` helpers. 4 unit tests + 2 #[ignore] GPU tests (clear round-trip on a 17x9 non-aligned width; zero-size rejection). lib.rs registers the module.
- WHY: Phase 2 task 2 — ADR-0010's core primitive: the renderer's primary path must produce CPU-readable pixels with no window system.
- EVIDENCE: cargo test -p gens-renderer 10 passed/0 failed/3 ignored; clippy -D warnings clean; this commit. One API surprise fixed from source reading: BufferSlice::get_mapped_range() takes no range argument in wgpu 30 and returns Result.

### [phase2] vertices, meshes and built-in shapes (P2-3)
- WHAT: `gens-renderer/mesh.rs`: `Vertex` (pos+color+uv, 36B stride, layout derived beside the struct), `Mesh` (u16 indices, validate() for range/multiple-of-3/65k-cap, to-bytes helpers), built-ins `triangle`/`quad`/`cube` (24 verts, 36 indices, 6 distinct face colors). 7 unit tests; winding verified with real cross products, not by convention. One clippy-1.99 lint fixed (manual_is_multiple_of).
- WHY: Phase 2 task 3 — draw-call payload + geometry the pipelines in P2-5 consume.
- EVIDENCE: cargo test -p gens-renderer 17 passed/0 failed/3 ignored; clippy -D warnings clean; this commit.

### [phase2] cameras + camera uniform (P2-4)
- WHAT: `gens-renderer/camera.rs`: `Camera` (view/proj pair) with `perspective` look-at and `orthographic_2d` (centered origin, +Y up), `uniform::bytes` serializing view_proj to the exact 64-byte WGSL `mat4x4<f32>` uniform block. gens-math gained the camera-constructor facade `look_at_rh`/`perspective_rh`/`orthographic_rh` (engine crates never call glam directly; glam 0.34 moved these to `camera::rh::*`). Projections deliberately emit **WebGPU NDC** (Z in [0,1], Y-up; glam's `directx` flavor) — one depth convention on every wgpu backend, asserted by a new math test. 5 camera tests + 1 new math test. One honest test-authoring fix disclosed: my first ortho depth expectation had the near/far plane sign backwards; the matrix was right, the test was fixed.
- WHY: Phase 2 task 4 — cameras feed every draw; the depth convention decision is engine law and now pinned by tests.
- EVIDENCE: gens-math 12/12, gens-renderer 22 passed/3 ignored; clippy -D warnings clean; this commit.

### [phase2] WGSL colored pipeline + GPU resources + Renderer facade (P2-5)
- WHAT: shaders/colored.wgsl (group0 camera, group1 model dynamic-offset uniforms); pipeline.rs ColoredPipeline (depth-tested, back-face culled, alpha-blended, validated WGSL compile as Result); resources.rs GpuMesh (validated upload) + ModelUniforms (256B-slot ring, one bind group + dynamic offsets, 256 slots); renderer.rs Renderer facade (set_camera/set_model stage writes; draw_and_read records pass + copy + submit + blocking readback -> RGBA bytes) with ColoredDraw. 2 unit tests (slot offset bounds) + 2 #[ignore] real-GPU tests: triangle centroid interpolation within an sRGB tolerance band + structural white-clear/corner assertions; two blue quads at different depths rendered in BOTH draw orders must be pixel-identical (depth beats submission order). wgpu 30 API drift absorbed: create_shader_module by value, Option-wrapped bind_group_layouts/depth_ops/depth_write_enabled/depth_compare, PipelineLayoutDescriptor.immediate_size replaces push_constant_ranges.
- WHY: Phase 2 task 5 — the first real GPU draw path; every later visual phase builds on this binding scheme.
- EVIDENCE: gens-renderer 24 passed/5 ignored; clippy -D warnings clean; fmt clean; this commit. GPU tests execute in CI (lavapipe/Metal) once the driver step lands in P2-8.

### [phase2] textures + textured pipeline (P2-6)
- WHAT: shaders/textured.wgsl (group1 = model uniform + sampler + texture; fragment = color * sample); texture.rs GpuTexture::from_rgba8 (Rgba8UnormSrgb, TEXTURE_BINDING|COPY_DST, linear/clamp sampler) with a pure validate_rgba8 contract; TexturedPipeline + create_draw_bind_group coupling the model ring with a texture. Renderer::draw_and_read now takes Draw::{Colored,Textured} (textured bind groups are per-call temporaries — honest cost note at Phase 2 scene sizes). 1 unit test + 1 #[ignore] GPU test: 2x2 checker texel-to-corner sampling (red/green/blue/white endpoints) proving UV orientation + sampler + tint multiply. wgpu 30 renames absorbed: BindingResource::TextureView (was Texture), SamplerDescriptor.mipmap_filter: MipmapFilterMode.
- WHY: Phase 2 task 6 — sprites/UI and everything later build on textured quads.
- EVIDENCE: gens-renderer 25 passed/6 ignored; clippy -D warnings clean; this commit. One real bug caught by my own overflow test (u64 multiply overflow at u32::MAX^2*4) and fixed with checked arithmetic before commit.

### [phase2] gpu_hello: full engine through the GPU (P2-7)
- WHAT: gens-runtime/examples/gpu_hello.rs — ECS scene (Root+Hero) driven by the fixed-timestep loop with FixedSource, transforms propagated, then EVERY frame rendered offscreen: background quad, trail dots (every 8th step, colored), textured checker hero sprite at its propagated WorldTransform. Final frame -> PNG + in-process structural verification (border == clear color; hero pixel != background/trail; trail present). --allow-no-gpu prints GPU-SKIP and exits 0 for adapter-less machines (Windows CI leg; ADR-0011); without it the run fails with actionable guidance. gens-renderer gained clear_color_rgba8 so engine code never constructs wgpu::Color (facade rule); gens-runtime now depends on gens-renderer. Two example unit tests (checker corners). Depth layering documented: background -0.4 < trail -0.2 < hero +0.1 (larger view z = nearer under WebGPU depth).
- WHY: Phase 2 task 7 — the proof that loop+ECS+renderer compose, and the CI smoke binary for GPU rendering.
- EVIDENCE: workspace 80 passed/0 failed/6 ignored; clippy -D warnings clean; sandbox probe shows both honest failure and GPU-SKIP paths working; this commit. GPU frames execute in CI (lavapipe Linux / Metal macOS).

### [phase2] CI GPU activation + size gate recalibration (P2-8)
- WHAT: ci.yml — Linux installs mesa-vulkan-drivers + vulkan-tools with a vulkaninfo probe logged; new "Tests (GPU integration)" step runs `cargo test --workspace -- --ignored` on Linux (lavapipe) + macOS (Metal); GPU smoke runs gpu_hello WITHOUT the skip flag on Linux/macOS (must render) and WITH --allow-no-gpu on Windows (best effort, prints GPU-SKIP evidence); artifacts renamed smoke-snapshots {smoke_cpu.png, smoke_gpu.png}; size gate recalibrated to gpu_hello @ 350 MiB (ADR-0012, measured 171.3 MiB; hello_world now 80.0 MiB). verify.sh mirrors: gates 2-7 with the same GPU smoke (local --allow-no-gpu) and the same size gate.
- WHY: Phase 2 task 8 — CI must exercise the real GPU path; a gate that fails on the engine's own smoke binary is a stale threshold, and ADR-0012 moves it with evidence instead of silently.
- EVIDENCE: this commit; branch CI run validates lavapipe rendering + the 6 ignored GPU tests on Linux and macOS runners.

### [phase2] CI-caught fixes: test geometry + sRGB clear semantics (P2-8b)
- WHAT: The first CI run with real GPU adapters (run #26) failed 3 renderer GPU tests on lavapipe AND Metal with clear-color-only pixels, and the Windows GPU smoke failed a border assertion. Two real defects found and fixed: (1) TEST GEOMETRY — the GPU tests used set_camera_2d (1 world unit = 1 pixel) but drew unit-sized shapes (span = 1 pixel) and sampled pixels designed for a 2x2 ortho camera; fixed with a set_unit_camera helper (ortho(2,2), 1 unit = 32 px). The renderer itself was correct. (2) sRGB CLEAR SEMANTICS — WebGPU treats LoadOp::Clear values on *unorm-srgb targets as LINEAR that the hardware encodes on store; clear_color_rgba8 passed encoded bytes u8/255. Evidence: Windows DX12 software adapter rendered border [86,90,101] = srgb_encode(24/255,26/255,33/255) for a requested [24,26,33]. clear_color_rgba8 now sRGB-decodes bytes to linear (srgb_byte_to_linear, round-trip unit test added); gpu_hello's border verification relaxed to structural (backend-independent) because mid-range stored bytes depend on the backend's clear conversion.
- WHY: Honesty rule + CI-first: the failures are the GPU gate doing its job; the record shows the exact evidence chain (Windows adapter log -> sRGB scaling math).
- EVIDENCE: CI run #26 logs (3 OS); workspace 81 passed/0 failed/6 ignored; clippy clean; this commit. Positive discovery: Windows CI runners DO have a usable software adapter ("Microsoft Basic Render Driver (Software, Dx12)") — Windows rendered GPU frames for real.

### [phase2] cross-backend precision evidence + centered-camera fix (P2-8c)
- WHAT: Run #29 diagnostics converted into fixes and evidence. (1) The triangle interpolation test now computes the EXPECTED color from the actual pixel-center sample point via barycentric weights -> linear interpolation -> sRGB encode, tolerance 2/255. CI evidence: llvmpipe/Vulkan (Linux) and Apple Paravirtual/Metal (macOS) rendered byte-identical [135,143,185] — the renderer's interpolation+encoding is provably correct and cross-backend deterministic; my old expectation wrongly assumed the pixel center was the triangle centroid. (2) gpu_hello added cx/cy offsets inside an ALREADY centered camera, pushing the hero to ortho x=220 > 160 (outside the window); models now use pure world coordinates and verification maps world->pixels explicitly (px = x + W/2, py = H/2 - y). (3) Positive: Windows DX12 software adapter now stores the background EXACTLY [24,26,33] — the sRGB clear decode fix from P2-8b is confirmed end-to-end.
- WHY: The GPU gate is doing exactly what it exists for; each red run produced a precise, evidence-backed fix.
- EVIDENCE: run #29 logs (3 OS); workspace 80 passed/0 failed/6 ignored; clippy clean; this commit.

### [phase2] phase docs + evidence (P2-9)
- WHAT: PHASE_2_REPORT.md (delivery summary, honest scorecard, 8-issue log with evidence chain, numbers incl. cross-backend determinism), README status/roadmap/build-docs update (GPU example + --ignored tests documented), RISKS reconciliation (R-004 Phase 2 note, R-002 window disclosure), evidence phase2/{ci-runs.txt ledger with exact run URLs, smoke_cpu.png, smoke_gpu.png}.
- WHY: Phase deliverable - the report must reconcile WORKLOG/DECISIONS/RISKS and let a reader verify every claim from committed evidence.
- EVIDENCE: docs/evidence/phase2/*; run #31; this commit.

### [phase2] merge to main + rollback anchor (P2-10)
- WHAT: phase2-renderer merged to main (--no-ff, merge commit 0f3a7ca); main CI run #33 SUCCESS; annotated tag pre-phase3-safe pushed at the merge. Evidence file records the merge run URL (verified via API).
- WHY: ADR-0005 - main receives only verified phase results; the pre-phase3-safe tag is the five-minute rollback anchor for Phase 3.
- EVIDENCE: docs/evidence/phase2/ci-runs.txt MERGE section; tags pre-phase1-safe/pre-phase2-safe/pre-phase3-safe all on origin.

### [phase3] dependency pins + branch start (P3-0)
- WHAT: branch phase3-physics-audio-input; rapier3d 0.36.1 + kira 0.12.5 (default-features OFF - cpal needs ALSA headers CI cannot install) pinned in [workspace.dependencies]; gens-physics/gens-input/gens-audio registered as workspace path deps; shells wired. Both APIs verified against vendored registry source BEFORE any code: rapier 0.36 has a PhysicsWorld convenience wrapper + glam-based math (glamx, glam 0.33.12 tree vs gens' 0.34.1 - converters required), kira has a MockBackend with manual process() and public StaticSoundData fields.
- WHY: Phase 3 pillar deps (README roadmap row 3); the no-guessing discipline caught the dual-glam-tree fact early and shaped the facade.
- EVIDENCE: crates.io max-stable queries; `cargo tree` unification check; this commit.

### [phase3] gens-physics: rapier facade + ECS glue (P3-1)
- WHAT: convert.rs (glam 0.34 <-> rapier glam 0.33 value converters, round-trip + f32-width guards); world.rs (PhysicsWorld: fixed 1/60 stepping, BodySpec/ColliderSpec value types, dynamic/fixed/kinematic bodies, ball/cuboid colliders, position/rotation/velocity readback, impulses, kinematic teleport, body_kind readback); ecs.rs (PhysicsResource newtype resource, PhysicsBody component, step_physics_system, sync_transforms_from_physics - dynamic/kinematic synced, fixed keep authored transforms). 13 unit tests.
- WHY: Phase 3 physics pillar; facade keeps rapier out of engine code (ADR-0013).
- EVIDENCE: 13 passed; clippy clean. Two honest discoveries: bevy_ecs 0.20 requires Resource: Component (hence the newtype), and rapier's RigidBodyType has a SoftFrame variant (non-exhaustive match fixed).

### [phase3] determinism + integrator honesty fixes (P3-1b)
- WHAT: My first free-fall test pinned semi-implicit Euler coefficients; a probe showed rapier 0.36 integrates differently (drop = (n-3/8)*g*dt^2, velocity exactly linear). Rewrote the contract to physics, not internals: velocity == g*t exact, position within 5 cm of closed-form 1/2*g*t^2, resting contact at floor top, impulse/velocity = 1/m, kinematic teleport, 600-step two-world bit-identical replay with periodic impulses. ECS test thresholds fixed to the measured resting height (0.494) instead of my wrong 4 m drop assumption.
- WHY: Pinning upstream integration internals would break on every rapier upgrade and was unverifiable by construction; the kinematic contract is the actual engine guarantee.
- EVIDENCE: 13 passed; probe outputs recorded in this worklog's commit history (probe removed after measurement).

### [phase3] gens-input: pure action-mapping model (P3-2)
- WHAT: keys.rs (engine-owned KeyCode/MouseButton/DeviceEvent, #[non_exhaustive]); state.rs (InputMap add/unbind/clear + OR-combined bindings; InputState DeviceEvent feed, end-of-frame update() edge rotation, hold/just queries per key/mouse/action, cursor pos+delta, wheel, clear_all focus loss; axis/axis_pair helpers). 9 unit tests. Deliberately bevy-free (ADR-0015): the generic Resource:Component constraint cannot be satisfied without polluting user action enums; ECS wrapper lands with windowing.
- WHY: gameplay must bind to actions, not raw controls; the DeviceEvent stream is the future winit/gilrs backend contract.
- EVIDENCE: 9 passed; clippy clean; one wrong test expectation fixed (mouse delta is total movement since update, (13,2), not the last hop).

### [phase3] gens-audio: kira mock-backend engine (P3-3)
- WHAT: AudioEngine over MockBackend (44.1 kHz default; process() advances the real mixer exactly 128 frames/tick - asserted; process_n helper); SoundHandle (state/position/pause/resume/stop/set_loop_region); tone.rs (sine/silence from raw Frames + rms_left metric - no assets, no decoder features). 10 unit tests incl. measured semantics: 1-tick command lag, frozen position while paused, wrapped loop positions, one-shot natural completion, independent handles, per-sound volume/panning accepted.
- WHY: Phase 3 audio pillar, headless-first (ADR-0014); realtime cpal output deferred as a feature flag (R-018).
- EVIDENCE: 10 passed; clippy clean. Honest finding: kira's default Tween is 10 ms (NOT instant) - tests use an 8-tick settle helper measured against source.

### [phase3] physics_hello: scripted replay integration proof (P3-4)
- WHAT: gens-runtime example: input_script() DeviceEvent timeline feeds InputState; per fixed step: script feed -> action queries -> input.update() -> solver step -> transform sync + propagation; two 600-step runs asserted BIT-IDENTICAL; behavioral gates (strafe +x, MoveLeft pulls below pre-phase x, jump peak > pre-jump y, never below floor, rests at ~0.5); trajectory JSON evidence output. 3 unit tests. gens-runtime gains gens-physics + gens-input deps.
- WHY: the integration proof across input+physics+ECS+loop (R-013 replay architecture), and the CI physics smoke binary.
- EVIDENCE: local run green (final_x=7.905011177062988, final_y=0.49992, peak_y=1.22952); clippy clean. Three real bugs caught by my own gates and fixed with evidence: (1) physics step was missing from the loop (x stayed 0) - and a patch script silently no-op'd once, re-applied with asserts; (2) input.update() before queries swallowed the jump edge (documented ordering contract proven necessary); (3) per-step impulse magnitude error -> 47 m/s ball rolled off the 60 m floor (min_y -325): fixed with realistic 0.05 Ns impulses, linear_damping on BodySpec (rapier has NO rolling resistance), CCD on the player, 100 m floor.

### [phase3] CI physics smoke on 3 OS + verify.sh 8 gates (P3-5)
- WHAT: ci.yml new step runs physics_hello on EVERY OS (CPU-only: no adapter caveats, unlike the GPU smoke); smoke_physics.json joins the artifact; verify.sh gains gate [6/8] mirroring it.
- WHY: every push must exercise the new pillars on all platforms (ADR-0004); the replay makes the 3-OS run a cross-platform consistency witness.
- EVIDENCE: local verify.sh PASS (112 tests, 29290 B trajectory, GPU-SKIP honest in sandbox, size gate 179.5 MiB vs 350 MiB); branch run #40 SUCCESS.

### [phase3] phase docs + evidence (P3-6)
- WHAT: PHASE_3_REPORT.md (scorecard, issue log, cross-OS determinism table), ADR-0013/0014/0015, RISKS (R-013 narrowed with per-OS evidence, R-018/R-019 added), README status/roadmap/build docs, evidence ledger docs/evidence/phase3/ci-runs.txt with run URLs.
- WHY: governance reconciliation at phase end (ADR-0007).
- EVIDENCE: run #40 logs (physics smoke 29290/29290/29253 bytes on Windows/Linux/macOS; x86_64 pair bit-identical, ARM64 macOS low-bit variance); this commit.

### [phase4] dependency pins + branch start (P4-0)
- WHAT: branch `phase4-animation-ui-ai` from verified main (477b36c, run #43 green). egui 0.36.2 pinned in `[workspace.dependencies]`; gens-animation/gens-ui/gens-ai registered as workspace path deps; egui wired into the gens-ui shell. APIs verified against vendored source BEFORE coding: MSRV 1.95 ≤ 1.99; default features exactly `[default_fonts]` (embedded Ubuntu-Light/NotoEmoji/Hack via epaint_default_fonts — zero system-font dependency); 0.36 frame model `ctx.run_ui(RawInput, |ui|) -> FullOutput` (plain `run()` removed); `ctx.tessellate` → `Vec<ClippedPrimitive>` (Mesh | Callback); Mesh indices `Vec<u32>`, premultiplied-sRGB vertex colors; synthetic input via `RawInput{screen_rect,time,events}` + `Event::PointerButton/PointerMoved/PointerGone`. No egui-wgpu: it pairs with its own wgpu minor vs gens' 30.0.1 — rendering egui's tessellation through gens' own pipeline keeps ONE wgpu in the tree.
- WHY: Phase 4 (animation + UI + AI) per README roadmap; the no-guessing discipline caught the frame-model and blending API drift before any code.
- EVIDENCE: crates.io max-stable queries; vendored source reads; this commit; run #44. Environment incident disclosed: sandbox target/ exhausted the 9.9 GB disk (ENOSPC during egui compile) — purged, `CARGO_INCREMENTAL=0` adopted, zero code impact.

### [phase4] gens-animation: skeleton + clips + forward kinematics (P4-1)
- WHAT: skeleton.rs (flat cycle-free Skeleton — parents strictly earlier; JointPose TRS; LocalPose with blend + FK via global_matrix/global_matrices); clip.rs (VecTrack/QuatTrack — Linear/Step/shortest-arc slerp, sorted+validated keys, partition_point sampling; JointTrack 3 channels; Clip = duration-bounded BTreeMap joint→track with sample_clamped + sample_looped). 26 tests. One honest test fix: `from_rotation_z(-270°)` canonicalizes to the +90° quaternion (unit quat double cover) — endpoint replaced with a true −90° and the fact documented.
- WHY: Phase 4 pillar 1 — the sampling layer blending, the controller, and the assets-phase skinning/glTF import build on; pure (state, time), no clocks (R-013).
- EVIDENCE: 26 passed; clippy clean; run #45.

### [phase4] gens-animation: blending + state-machine controller (P4-2)
- WHAT: blend.rs (blend_into in-place lerp/slerp; crossfade timed clamped); controller.rs (AnimationController: Arc<Clip> states, declared parameter table (Number/Flag), data-only Condition/NumberOp transitions, crossfade bookkeeping with the outgoing clip still playing, insertion-order first-match scan, looping play_time wrap; slide_clip test helper). 44 crate tests. My own tests caught play_time advancing TWICE per update (duplicated line — probe example showed the oscillation); looping play_time now wraps at update so queries see sampler phase.
- WHY: controllers are how gameplay drives animation; data conditions (not closures) keep the controller serializable and bit-for-bit replay-testable.
- EVIDENCE: 44 passed; clippy clean; run #46.

### [phase4] gens-ai: behavior trees with blackboard + memory composites (P4-3)
- WHAT: bt.rs — NodeStatus, Blackboard (BTreeMap typed memory: Number/Flag/Text), Behavior trait (tick+reset), nodes: Sequence/Selector (memory cursors — finished children NOT re-ticked, pinned by a tick-counting test), Parallel (All/Any), Inverter, Repeat, Wait, Action/FnMut, Condition/Fn. 13 tests incl. a guard-patrol scenario and a bit-exact replay trace.
- WHY: the decision layer the runtime example drives agents with; determinism from declaration-order iteration + ordered maps, no RNG, no clocks.
- EVIDENCE: 13 passed; clippy clean; run #47. Authoring fixes disclosed: one test expected a third Running tick the correct memory semantics never produce (rewritten with a child tick-counter proving no re-tick); Rc<Cell> counters cloned before moving into leaf closures.

### [phase4] gens-ai: grid nav + deterministic A* + steering (P4-4)
- WHAT: nav.rs — Grid (walkable/cost cells on XZ, cell_center/world_to_cell), A* with Connectivity::Four/Eight, corner-cut prevention, FIXED neighbor order (N/E/S/W then NE/SE/SW/NW), integer-quantized priorities (cost·1024→u32; total order (f,h,index) — no float-comparator ambiguity), Manhattan/octile heuristics, PathError diagnostics; Steering seek/flee/arrive + stateless follow_path (documented overshoot limitation) + stateful forward-only PathFollower. 31 crate tests.
- WHY: agents need paths; the runtime example steers along them. Quantized integer priorities make equal-priority pops a TOTAL order: same grid, same path, always.
- EVIDENCE: 31 passed; clippy clean; run #48. Fixes disclosed: broken exploratory code in a reconstruct draft (caught by compile); the stateless follow_path turn-back behavior documented and given its own test, PathFollower added as the correct stateful primitive; clippy unwrap-after-is_some in neighbor expansion fixed.

### [phase4] gens-ui: egui-backed headless core with replayable frames (P4-5)
- WHAT: frame.rs (pure data: UiVertex pixel-space premultiplied, UiMesh + clip rect, TextureRef/Upload/Free, UiFrame, neutral UiEvent stream, UiInput with the engine clock); context.rs (UiContext: RawInput with ROOT viewport info + deterministic time, 1:1 event translation, one run_ui pass, tessellate, CPU texture mirror so partial font-atlas deltas still yield FULL uploads, textures_delta.clear() acknowledgement — epaint 0.36 Drop-asserts unapplied deltas). 7 tests: font atlas arrives as Managed(0) pass 1; label tessellates; synthetic press+release at a captured button rect CLICKS; pointer-away does not; partial deltas stay whole-texture; byte-identical frame replays; small wheel deltas observable same-pass. ppp pinned 1.0; uploads/frees sorted (egui delta maps are HashMaps).
- WHY: runtime UI + text rendering pillar; immediate mode fits the replay-tested simulation style; no egui-wgpu (wgpu unification, ADR-0017).
- EVIDENCE: run #49 FAILED (3 clippy lints on 3 OS — cargo test green; `clippy | tail -1` masked the failing status locally: lesson recorded), P4-5b run #50 SUCCESS. 0.36 API drift absorbed from source: CentralPanel::show now takes the pass &mut Ui; MouseWheel carries TouchPhase; ImageData is Color-only.

### [phase4] gens-renderer + gens-ui: wgpu UI backend (P4-6)
- WHAT: renderer ui.rs (UiVertex 32 B NDC+uv+premult color; ndc_from_pixel pure tested on 4 corners+center; UiBatch with scissor; UiPipeline — shaders/ui.wgsl, NO uniforms, premultiplied over-blend, no depth; validate_batch; Renderer::draw_ui_and_read — color-only pass, temp buffers documented); gens-ui backend.rs (UiRenderer: core + BTreeMap texture cache; uploads → GpuTexture (CPU mirror guarantees whole textures); pixel→NDC; clip→clamped scissor (pure tested); unknown textures SKIP+count). 2 #[ignore] GPU tests: opaque panel fill byte-exact (20,24,33) + glyphs/button paint over fill (rects captured from layout); hover changes button-center pixels.
- WHY: the "text rendering" half of gens-ui's contract becomes real pixels through ONE wgpu tree, reusing the Phase 2 offscreen-first readback architecture end to end.
- EVIDENCE: runs #51/#52 FAILED (CI GPU tests doing their job), #53 SUCCESS. Issue chain: (1) panel fill stored [79,87,101] = encode(20/255) — raw byte/255 vertex colors treated as linear, double-brightened by the sRGB target; fixed by decoding premultiplied-sRGB vertex colors at the render boundary (the Phase 2 P2-8 convention), round-trip unit-tested WITHOUT a GPU; (2) my text scan probed a hardcoded row above the glyphs — renderer correct, test fixed by capturing widget rects (the P2-8c lesson again).

### [phase4] runtime: animate_hello integration proof (P4-7)
- WHAT: 12×10 grid with central obstacle; BT Selector[dwell, steer] over Rc<RefCell<AgentRuntime>> leaves; PathFollower at 2 u/s; moving flag drives AnimationController (walk 1 s loop + sway, idle 2 s breathing, 0.2 s crossfades) on a 2-joint skeleton; FK + agent pos → ECS Transform → EngineSchedules.run_frame → WorldTransform (Phase 1 pipeline); headless egui HUD per step, tessellated frame FNV-hashed into evidence. Samples every 20 steps. Two 900-step runs BIT-IDENTICAL; gates: x-span > 8, both states observed, walked > 16. Measured: 1.5→10.5 span, 24.57 walked, 45 samples, 4100 B evidence.
- WHY: the Phase 4 integration proof — three new pillars compose with the Phase 1 loop/ECS exactly as physics_hello composed input+physics in Phase 3.
- EVIDENCE: run #54; first execution green after three compile-caught shape fixes (RefMut guard overlap → steer takes owned snapshots; BT leaves own clones of shared state; Box::new on every composite).

### [phase4] CI smoke on 3 OS + verify.sh 9 gates (P4-8)
- WHAT: ci.yml 'Headless animation+AI+UI smoke' runs animate_hello on EVERY OS; smoke_animate.json joins artifacts; verify.sh gains gate [7/9] and renumbers to 9 gates.
- WHY: every push exercises the new pillars on all platforms (ADR-0004); the replay makes each 3-OS run a cross-platform consistency witness.
- EVIDENCE: local verify.sh PASS (9 gates); run #55: animate smoke 4100/4100/4100 B on Windows/Linux/macOS — byte-identical cross-OS evidence recorded under R-013/R-021 without claiming a guarantee.

### [phase4] phase docs + evidence (P4-9)
- WHAT: PHASE_4_REPORT.md (scorecard, issue log, cross-OS table), ADR-0016/0017/0018, RISKS (R-013 Phase 4 note, R-017 per-call granularity note, R-020 egui churn, R-021 UI raster bit-identity), README status/roadmap/build docs, evidence ledger docs/evidence/phase4/ with run URLs.
- WHY: governance reconciliation at phase end (ADR-0007).
- EVIDENCE: runs #44–#55 ledger; workspace 199 passed / 0 failed / 8 ignored (GPU).

### [phase5] dependency pins + branch start (P5-0)
- WHAT: branch `phase5-scripting-assets-save` from verified main (275d5b1, run #57 green). Pinned: serde 1.0.229 (derive), serde_json 1.0.154, rhai 1.26.1 (`default-features = false, features = ["std"]` — kills ahash runtime-rng so script-visible behavior keeps fixed compile-time-rng hashing), gltf 1.4.1 (default import/utils/names; image 0.25 optional dep matches workspace minor). glam gains the `serde` feature (save-system serialization through gens-math re-exports). gens-scripting/gens-assets registered in [workspace.dependencies] and given their phase deps. MSRVs verified from vendored source: rhai 1.66, gltf 1.61, serde 1.56, serde_json 1.71 — all ≤ 1.99.
- WHY: Phase 5 (scripting + assets + save) per README roadmap; P4-0's no-guessing discipline: every version queried on crates.io, every API fact read from vendored source BEFORE coding.
- EVIDENCE: crates.io max-stable queries; vendored-source reads (rhai Map = BTreeMap; ahash forced compile-time-rng via `cargo tree -e features`; gltf Document/Animation/Skin/Node/Mesh/Primitive structs; rhai limits.rs set_max_* sandbox knobs); `cargo fetch` clean; this commit; run #58.

### [phase5] gens-assets: registry core — ids, content hashing, versioning (P5-1)
- WHAT: hash.rs (FNV-1a 64, published-vector tested — the engine's evidence-hash primitive reused for content identity); registry.rs (AssetId = FNV of canonical key — reproducible across machines/processes; AssetRegistry BTreeMap with by-id + by-key indexes, import/import_file, version bumps ONLY when content hash changes, kind-change + id-collision + empty-content + invalid-key errors; deterministic id-ordered iteration); mesh.rs (MeshData positions/normals/uvs/joints/weights/indices + bounds + validate — joints/weights parsed now so the skinned GPU path can adopt them without re-import); error.rs (AssetError + UnsupportedGlTF vocabulary). 19 tests. import_file reads files at the boundary only; everything below is pure.
- WHY: Phase 5 pillar 1 — every later importer (glTF, layouts, scripts) registers through this; asset identity as a pure function of (key, bytes) keeps saves/manifests byte-identical.
- EVIDENCE: 19 passed; clippy -D warnings clean; fmt clean; run #61.

### [phase5] gens-assets: glTF 2.0 import — meshes, skins, clips (P5-2)
- WHAT: gltf.rs (import_gltf_bytes for data-URI/GLB fixtures, import_gltf_file with full buffer resolution; nodes as TRS; meshes with JOINTS_0/WEIGHTS_0 parsed now for the future skinned path; skins as Skeleton + inverse-bind matrices + joint_nodes; animations as Clips bound to the skin they drive, LINEAR/STEP only, one skin per clip). Explicit UnsupportedGlTF vocabulary: CUBICSPLINE, morph targets, matrix nodes, morph-weight channels, mixed-skin clips, multi-primitive meshes — all fail loudly (no silent partial import). Fixture walker.gltf generated by committed scripts/gen_gltf_fixture.py (self-contained data-URI buffer, 420 B, 2-joint skin + skinned quad + 3-channel walk anim); 10 integration tests incl. determinism (two imports equal) and 5 negative paths; 29 crate tests total.
- WHY: Phase 5 pillar 2 — "load real content onto animation clips" (P4 report §7); ADR-0016 reserved clip import for this phase; every rejection documented instead of guessed.
- EVIDENCE: gltf 1.4.1 API verified from vendored source before coding (ReadNormals = Iter<[f32;3]>, enum-as-iterator for indices/joints/weights/texcoords, morph_targets name, Fn+Clone reader bound); two honest test-expectation fixes disclosed: STEP semantics hold the earlier key across [key, next) (second key of a 2-key track visible only at t=1.0), and gltf-rs parse-validation rejects POSITION-less primitives before my reader (test asserts that layer); clippy -D warnings clean (flat_map_identity); fmt clean; run #62.

### [phase5] gens-scripting: sandboxed Rhai engine + serializable script state (P5-3)
- WHAT: engine.rs (ScriptEngine on Engine::new_raw — NO packages: no print/file/network, no module resolver so imports parse but never resolve; hard limits MAX_OPERATIONS=100k, call levels 32, expr depth 32, map/array 1024, string 8 KiB; Script = named AST; call_fn converts through Dynamic and type-checks at the wrapper edge — no internals feature); vars.rs (ScriptVars = BTreeMap<String, ScriptValue> Number/Flag/Text, kind-checked writes (type-changing writes are errors), sorted iteration, serde round-trip stable). rhai feature set finalized: std + sync (Engine/AST Send+Sync for the bevy-Resource future) — runtime-rng already killed in P5-0. 15 tests incl.: runaway loop/recursion hit the caps, print absent, import unresolvable, script maps iterate sorted, 50-call deterministic replay traces equal.
- WHY: Phase 5 pillar 3 — game logic in scripts without breaking the replay contracts (R-013): no clock, no RNG, no ahash runtime-rng, BTreeMap ordering, closed variable vocabulary.
- EVIDENCE: rhai 1.26 APIs verified from vendored source before coding (call_fn signature, Scope::push/get_value, limits.rs setters, new_raw package-free engine, Variant root-export gated behind internals — avoided via Dynamic+try_cast); honest test-expectation fixes: `go` is a Rhai reserved keyword, import fails at EVAL not compile, EvalAltResult Display reads "Too many operations"; 15 passed; clippy -D warnings clean; fmt clean; run #63.

### [phase5] gens-scripting: ECS binding — host, entity API, deterministic runner (P5-4)
- WHAT: host.rs (ScriptHost resource = engine + BTreeMap scripts + current-frame slot; ScriptComponent {script, entry, vars, requests}; run_scripts exclusive system: snapshot ticks in ENTITY-INDEX order (ADR-0019), per-tick ScriptFrame {time, dt, pos, rot, vars, out_pos, requests} published through Arc<Mutex<...>> (rhai sync requires Send+Sync natives; use is sequential so the mutex never contends), entry fn called per entity, writes applied ONLY on success — a failed tick leaves the entity untouched; animation requests are DATA (BTreeMap params + play option), keeping scripting->animation a one-way data street). Entity API = the ONLY non-pure surface: now/dt/pos_x/y/z/set_pos, var_num/set_var_num/var_flag/set_var_flag/var_text/set_var_text (kind-checked), anim_set_param/anim_play. 6 host tests: movement+var persistence, 120-step two-run determinism, spawn-order independence, anim requests recorded, failing script no-ops, missing script skips.
- WHY: "wrap the AI/animation surface in a scripting API without breaking the replay contracts" (P4 §7) — the frame-slot pattern means scripts see a value-snapshot per tick, never live ECS references, so ordering + write-back are the only nondeterminism channels and both are pinned.
- EVIDENCE: bevy_ecs 0.20 Entity::index() returns EntityIndex (not u32 — verified in vendored source), Resource-only derive (the PhysicsResource precedent; Resource:Component is implied in bevy 0.16+); rhai sync natives need Send+Sync closures; 21 crate tests passed; clippy -D warnings clean; fmt clean; run #64.

### [phase5] gens-runtime: versioned world save/load (P5-5)
- WHAT: save.rs — self-contained versioned document (GENSSAVE1 magic line + JSON body + GENSHASH fnv1a64 trailer), MIRROR types (SavedTransform as [f32;3]/[f32;4], SavedScript {script, entry, vars, requests}) decoupling the file format from live ECS/glam internals; capture_world iterates read-only (world.iter_entities) then sorts by entity index — the canonical order; apply_save spawns in file order into a FRESH world and re-derives WorldTransform via EngineSchedules (never saved); atomic write (tmp + rename) so a crash mid-write keeps the previous save; hash + magic + version all verified on load. 9 tests: byte-determinism, full roundtrip (all components incl. vars/requests), index-sorted order, framing, corrupt-body hash rejection, bad magic, future version rejection, atomic write leaves no tmp, WorldTransform never saved.
- WHY: Phase 5 pillar "save" (README roadmap); ADR-0021 scope decision: engine-standard component vocabulary only — game state that must survive save/load travels through ScriptComponent::vars (serialized), and a reflection/registry layer for arbitrary components is future work, stated honestly.
- EVIDENCE: bevy_ecs 0.20 world.query needs &mut World — capture uses iter_entities + per-component get (read-only) with the index sort providing determinism; 14 gens-runtime lib tests passed; clippy -D warnings clean; fmt clean; run #65.

### [phase5] runtime: asset_script_hello integration proof (P5-6)
- WHAT: the phase's centerpiece example — a patrol agent where the ENTIRE decision layer is content: walker.gltf imported (Skeleton + walk Clip + skinned MeshData, registry identities logged into evidence), patrol.rhai (embedded, registry-registered) drives waypoints/movement/phase/dwell through the sandboxed entity API (pos_x/z/set_pos, var_*/set_var_*, anim_play, dt, math vocabulary incl. to_int — rhai has NO cast operator and to_int lives in packages the sandbox does not load), run_scripts ticks in entity order, the host samples the imported clip at the script-driven phase and writes FK offsets to Transform, propagation computes world transforms. Run A straight 1200 steps; Run B saves at step 600 (atomic, hash-verified) -> FRESH stage -> apply_save -> RE-SAMPLES the pose (derived state) -> continues; all 60 samples bit-identical across both runs. Gates: x span 7.997 (max-min over the full excursion), max waypoint 3, both states observed, phase wrapped.
- WHY: Phase 4 §7's promise verbatim — content loads onto clips, a script wraps the surface, nothing breaks the replay contracts.
- EVIDENCE: two honest example bugs disclosed and fixed: (1) bevy 0.16+ resources are ENTITIES — build_world(false) + apply_save duplicated the agent (initial entity kept patrolling) and iter_entities().next() picked the ScriptHost resource entity; agent now found by Name; (2) the pose is derived state — after load it must be re-sampled from restored vars or the swap-step hash diverges (which is exactly the documented save contract). verify.sh 10/10 PASS locally (GPU-SKIP honest); run #66.

### [phase5] CI assets+scripting+save smoke on 3 OS + verify.sh 10 gates (P5-7)
- WHAT: ci.yml new step runs asset_script_hello on EVERY OS (CPU-only); smoke_asset_script.json joins the artifacts; verify.sh renumbered to 10 gates with [8/10] mirroring the smoke.
- WHY: every push exercises the new pillars on all platforms (ADR-0004); the bit-identical save/load replay makes each 3-OS run a cross-platform consistency witness.
- EVIDENCE: local verify.sh PASS (10 gates, 30064 B evidence); run #66.

### [phase6] dependency pins + branch start + default-branch hygiene fix (P6-0)
- WHAT: branch `phase6-networking-data-assets` from verified main (7db9eb0, run #68 green, tag pre-phase6-safe). Pinned (crates.io max_stable queried 2026-10-12): tokio 1.53.2 (`default-features = false, features = ["rt","net","time","sync","macros"]`), quinn 0.11.12 (`default-features = false, features = ["runtime-tokio","rustls"]` — the `rustls` feature maps to `rustls-ring`, so the crypto provider is ring, avoiding aws-lc-rs's cmake/NASM build burden on Windows CI), rcgen 0.14.10 (`pem`, `ring` — provider parity with quinn). gens-networking registered in [workspace.dependencies] and given its phase deps (gens-assets for the FNV-1a content-hash primitive). MSRVs verified from vendored source: quinn 1.85, rcgen 1.88, rustls 1.71, tokio 1.71 — all ≤ 1.99. Repository hygiene: default branch was still `phase0-risk-management` (GitHub kept the first-pushed branch since Phase 0; fresh clones got a stub workspace) — fixed to `main` via API, disclosed here.
- WHY: Phase 6 (networking + data assets) per README roadmap; P4-0's no-guessing discipline: every version queried on crates.io, every API fact read from vendored source BEFORE coding (quinn::rustls re-export gated on rustls-ring/rustls-aws-lc-rs; ServerConfig::with_single_cert(Vec<CertificateDer>, PrivateKeyDer); ClientConfig::with_root_certificates(Arc<RootCertStore>); RootCertStore::empty+add_parsable_certificates; rcgen CertifiedKey { cert, signing_key } with cert.der() -> &CertificateDer<'static>, signing_key.serialize_der() -> Vec<u8>).
- EVIDENCE: cargo fetch clean; sandbox toolchain reinstalled after environment reset (rustc 1.99.0, same pin as CI); MSRV table from vendored Cargo.tomls; API signatures quoted from quinn-proto 0.11.19/src/config/mod.rs and rcgen 0.14.10/src/lib.rs; this commit; branch CI run.

### [phase6] gens-assets: data documents — versioned scene/prefab format (P6-1)
- WHAT: data.rs (DataDocument {format_version, name, entities: Vec<DataEntity> in authored order}; DataEntity {name (unique), translation [f32;3], rotation [f32;4] xyzw unit-quat, scale [f32;3] finite non-zero, script: Option<DataScript>}; DataScript {script key, entry fn, vars: BTreeMap<String, DataVar>}; DataVar = mirror vocabulary Number(f64)/Flag/Text — deliberately NOT gens-scripting's ScriptValue so gens-assets stays rhai-free, bridge converts with kind checks like save.rs does (ADR-0021 decoupling, R-023 scope note)). Loud validation: version!=1, empty document/entity names, duplicate names, non-finite TRS, quat norm |1|>1e-4, zero scale, empty script key/entry — all DataError variants; serde_json parse errors surfaced through AssetError::Data. import_data_document validates BEFORE registering (invalid content never enters the registry). canonical_json() byte-stable (struct field order + BTreeMap). AssetKind::Data added; .json extension re-mapped UiLayout -> Data (UiLayout never had an importer; it keeps .uilayout; registry test updated). 14 tests.
- WHY: Phase 6 pillar "data assets" (README roadmap); PHASE_5_REPORT §7 — data-driven content formats on the same registry. Authored-order entities + sorted vars keep the document identity a pure function of content; the runtime bridge (P6-2) and the network snapshot channel (P6-4) build on this vocabulary.
- EVIDENCE: 33 lib + 10 gltf integration tests passed; clippy -D warnings clean (manual_contains fixed); fmt clean; local verify.sh PASS (10 gates; animate smoke 4100 B and asset_script smoke 30064 B — byte-identical with the Phase 5 ledger); this commit; branch CI run.

### [phase6] gens-assets data format gains active/tag; gens-runtime bridge — document -> world (P6-2)
- WHAT: DataEntity extended with `active: bool` (serde default true) and `tag: Option<String>` — the document now covers the FULL save vocabulary (name/active/tag/transform/script), making it a complete world-bootstrap format; DataEntity::named constructor + tag/active round-trip test (15 data tests). gens-runtime::data.rs: apply_data_document re-validates (loud for programmatically-built docs too), spawns in authored order (Name, Transform from TRS verbatim, Active when active, Tag, ScriptComponent with DataVar->ScriptValue conversion + ScriptRequests::default()); DataSpawnReport returns ids in spawn order; data_var_to_script_value is the single total conversion point (vocabularies identical by construction). 6 bridge tests: component presence/values/order, two-fresh-worlds capture byte-identically, data->world->save->world->capture == data->world->capture (the data and save paths agree exactly), hand-built invalid doc refused, empty doc no-op, vars sorted into the save.
- WHY: PHASE_5_REPORT §7 — data-driven content on the same registry discipline; the bridge is the mirror of save.rs (capture = world->document, data = document->world) and its byte-equality test proves the two paths share one canonical form, which the network snapshot channel (P6-4) transfers verbatim.
- EVIDENCE: honest test-expectation fix disclosed: a fresh bevy 0.16+ World's first spawned entity does NOT get index 0 (low indices reserved for resource entities — the P5-5 lesson again); the bridge contract is RELATIVE consecutiveness (spawn order == authored order) and the test now asserts windows(w[1]==w[0]+1) instead of absolute base. 20 runtime lib tests passed; clippy -D warnings clean; fmt clean; local verify.sh PASS (10 gates, smoke sizes ledger-matching); this commit; branch CI run.

### [phase6] gens-networking: QUIC loopback transport behind a sync facade (P6-3)
- WHAT: cert.rs (LoopbackCert — rcgen self-signed for "localhost", DER in memory, quinn server/client configs built from it; client trusts EXACTLY the operator-provided cert — no system roots, no TOFU); transport.rs (QuicEndpoint {server(addr, cert), client(cert)} — each endpoint owns a DEDICATED I/O THREAD whose tokio current_thread runtime is driven for the endpoint's whole life; QuicLink {send_frame, recv_frame, close} — one bidirectional stream per link, framed [u32 len][u64 FNV(payload)][payload], hash-verified on receipt, 16 MiB pre-allocation guard; TransportError vocabulary incl. CorruptFrame/FrameTooLarge/Timeout). Link establishment ends with an EMPTY ANNOUNCE FRAME the client sends and the server consumes during accept. Graceful teardown: QuicEndpoint::drop sends Shutdown{grace 250ms} and JOINS the I/O thread; link drops drain 30ms. 14 tests (3 cert + 11 transport).
- WHY: Phase 6 pillar "networking" (README roadmap); async confined behind a sync facade (ADR-0022 preview) — the engine calls send/recv and simulates, it never sees tokio. The FNV-verified frame extends the asset/save integrity discipline to the wire (PHASE_5 §7: move state over the wire with the same hash discipline).
- WHY-THIS-SHAPE (three empirical findings, all reproduced before being fixed, all load-bearing): (1) quinn transmits packets ONLY while its runtime is being block_on-driven — a facade that block_ons per call freezes outbound frames when its main future completes (a handshake completes while the peer's reply sits untransmitted; three tests deadlocked exactly there); the permanently-driven I/O thread removes that class. (2) QUIC announces a stream on the wire only when its FIRST BYTE is written — open_bi alone leaves the server's accept_bi waiting forever; the announce frame makes link establishment data-driven and deterministic. (3) Runtime drop cancels queued teardown frames (CONNECTION_CLOSE never reached the peer, flaky 2/5); graceful Shutdown+join fixes it. Honest debug debris: four scratch test binaries were written and DELETED during the bisection; the committed evidence is the three findings above plus the fix.
- EVIDENCE: 14 tests green; 8 consecutive PARALLEL full-crate runs green (the flake hunt: sequential-pass/parallel-fail races eliminated by the announce protocol + deterministic test endings + graceful shutdown); clippy -D warnings clean (unused_mut/unused_imports/collapsible_if/manual_contains fixed along the way); fmt clean; verify.sh 10/10 PASS (physics 29290 B, animate 4100 B, asset_script 30064 B — ledger-matching); this commit; branch CI run. MSRV note: quinn resolves its async runtime at endpoint creation (inside block_on); quinn 0.11.12's Endpoint::client takes only a SocketAddr with set_default_client_config afterwards (API verified from vendored source).

### [phase6] gens-networking: sessions — versioned handshake + hash-checked envelopes (P6-4)
- WHAT: session.rs — strict message vocabulary MsgKind {Snapshot=1, Input=2, Bye=3} (unknown kinds rejected loudly, no Custom escape hatch — two disagreeing builds must fail, not "work"); envelope codec encode_envelope/decode_envelope (tick u64 | kind u8 | payload_len u32 | payload_hash u64 FNV | payload; pure functions, byte-exact layout tests) with MAX_SESSION_PAYLOAD = frame max − 21; handshake HELLO "GENH"+version → WELCOME "GENW"+version+session_id / REJECT "GENR"+reason, version mismatch = loud VersionMismatch + REJECT to the peer; Session::host(link, session_id, t) / ::client(link, t) / send(kind, tick, payload, t) / recv(t) / close. 7 tests (4 pure envelope + 2 loopback handshake/message flows + version-reject flow).
- WHY: PHASE_5 §7 — "move save state over the wire with the same hash discipline": the envelope's payload hash is INDEPENDENT of the transport's per-frame hash (belt and suspenders, deliberate — a future transport swap cannot silently weaken integrity). Ticks are caller-assigned simulation state, never wall-clock. Session identity is host-assigned and passed in (this layer never invents ids).
- EVIDENCE: 18 crate tests green (3 cert + 8 transport + 7 session); 3 consecutive parallel runs green; clippy -D warnings clean; fmt clean; verify.sh 10/10 PASS (smoke sizes ledger-matching); this commit; branch CI run.

### [phase6] CI net smoke on 3 OS + verify.sh 11 gates (P6-6)
- WHAT: ci.yml new step runs net_hello (--steps 600 --replicate-at 300) on EVERY OS (loopback QUIC, no external network; smoke_net.json joins the artifacts); verify.sh renumbered to 11 gates with [9/11] mirroring the smoke.
- WHY: every push exercises the networking pillar on all platforms (ADR-0004); the wire-continuation replay makes each 3-OS run a cross-platform consistency witness.
- EVIDENCE: local verify.sh PASS (11 gates, net smoke 1571 B replication evidence); this commit; branch CI run.

### [phase6] phase docs + evidence (P6-7)
- WHAT: PHASE_6_REPORT.md (delivery sections, honest scorecard incl. NOT-claimed scope, numbers, 8-item issue log, run ledger, decisions/risks delta, Phase 7 handoff); ADR-0022 (quinn loopback-first behind a sync facade with a permanently-driven I/O thread), ADR-0023 (versioned JSON scene documents on the registry), ADR-0024 (strict session vocabulary + version-pinned handshake + transport-independent envelope hash); RISKS R-013 Phase 6 note + new R-025 (loopback-only scope), R-026 (quinn/tokio churn), R-027 (replication scope); README status banner (fixed the stale Phase-3 text) + roadmap row 6 + net_hello build docs; evidence ledger docs/evidence/phase6/ci-runs.txt.
- WHY: governance reconciliation at phase end (ADR-0007).
- EVIDENCE: runs #69–#76 all green; workspace 300 passed / 0 failed / 8 ignored (GPU); this commit; branch CI run.

## [phase7] Visual editor — hierarchy, viewport, inspector, assets, console (P7-0..P7-8b)

- WHAT: gens-editor realized against docs/design/REFERENCE.png: dark theme (deterministic style application, ADR-0025), docked shell (menu bar with real command intents, hierarchy, 3D viewport, inspector, asset browser, tracing console, status bar), editor components (MeshSlot/ShapeKind, AuthoredBody mirroring BodySpec+ColliderSpec), two-renderer composition (dedicated viewport renderer feeds an egui managed texture; window UiRenderer paints the chrome; final RGBA is the artifact of record), editor_hello replaying a real click/drag/edit/duplicate/save sequence over the reference scene with REAL physics (fixed ship collider, falling CCD crate), REAL assets (rhai source + encoded PNG), REAL save output (GENSSAVE1 autosave), 24-frame FNV ledger, two-run byte-identity; CI editor smoke on 3 OS + artifacts; verify.sh 12 gates.
- WHY: Phase 7 = visual editor per README roadmap; panels reflect real engine vocabulary only (ADR-0025) — no display-only fields; determinism kept (ppp 1.0, sim clock, scripted replay).
- EVIDENCE: 24 unit + 1 GPU-ignored tests green; clippy -D warnings exit 0; verify.sh PASS; CI runs #82-#92 (two reds diagnosed and fixed with evidence: clippy drift #82, single-run_frame bug #90); run #92 snapshot inspected pixel-by-pixel — viewport renders the scene (terrain, guards, ship at the edited position with gizmo, falling crate), console shows the live trace lines, dock columns correct; docs/evidence/phase7/editor_hello.{png,json}; determinism verified (two runs byte-identical).
- LESSONS: (1) hash gates are blind to layout — snapshot inspection + geometry tests now mandatory (ADR-0026); (2) Renderer camera staging is explicit state — its omission produced a hash-stable empty viewport; (3) egui 0.36 facts (Panel::show, ui.columns, TextureHandle::set &mut, Margin i8, style.spacing/Visuals split, Theme-pinned styles) verified from vendored source before each use; (4) bevy 0.20 facts (iter_entities -> EntityRef, entity ids unordered, seed 0v0 excluded by content filter); (5) Windows best-effort smoke branches must mark skips auditable (R-028).

## [phase8] Profiling + debugging + build pipeline (P8-0..P8-8)

- WHAT: gens-profiler (workspace member 16) — handle-based nested span capture (begin_frame/begin_span/end_span/end_frame + time() closures; disabled fast path; robust auto-complete/orphaned-flag contract; 4096-span/frame cap), aggregation report (per-name count/present_frames/quartiles/share; byte-stable gens-profile-1 JSON), tracking global allocator (current/peak/counts, gens-mem-1); gens-runtime/src/profile.rs — instrumented_frame + canonical world_state_hash; the ADR-0027 proof example (profile_hello: profiler-off vs profiler-on replays byte-identical over 600 steps, evidence JSON with phase quartiles + memory); gens-runtime/src/debug.rs — debug_dump (gens-debug-1) + validate_world (typed ViolationKind vocabulary: non-finite transforms, non-unit rotation, zero scale, broken propagation, dangling links, duplicate names, stale physics handles via new PhysicsWorld::body_exists, unregistered scripts); gens-build (workspace member 17) — GENSPKG1 package container (save-format framing, per-file FNV hashes, build-time scene validation, shared registry key/kind rules), gens-pkg + gens-player CLIs, headless player with gens-run-report-1; editor integration — Telemetry rolling-window aggregation, real Profiler tab (phase table), Performance tab (frame time + memory), status-bar CPU/Mem (only when data exists), console command row + executor (help/entities/validate/stats/clear) with ADR-0026 geometry pins; bench_step (3 scenarios, fixed sampling methodology, environment disclosure); CI: profiler/package/bench smokes on 3 OS + debug/release binary size gates; verify.sh 15 gates; [profile.dev] debug=1.
- WHY: README row 8 "Profiling + debugging + build pipeline"; R-004 debt closed honestly; ADR-0027 (observation-only profiling, proven per integration), ADR-0028 (package container), ADR-0029 (bench methodology + release gate).
- EVIDENCE: runs #97-#105 (#98 fmt drift red, #101 flaky memory test red on Windows — both fixed with evidence); profile_hello: 600/600 hashes identical, physics_step median ~232us share 78%, final_hash stable across runs; bench (debug/linux): physics_10 23.8ms, physics_50 175.0ms, scripts_20+physics_20 92.7ms per 60 steps, stddev <1%; package pipeline end-to-end (build -> verify -> 300 steps -> 0 violations); editor_hello gains validated_clean gate + profile_sidecar; gpu_hello sizes: debug 170.1 MiB (gate 350), release 8.3 MiB (gate 20).
- LESSONS: (1) the fmt gate is [2/15] for a reason — run cargo fmt --all --check BEFORE every commit, never cargo fmt -p after edits (#98); (2) assertions about process-global counters must hold under arbitrary interleaving — monotonic-only families, or they flake on Windows timing (#101); (3) bevy_ecs 0.20 binds systems to their initializing World — fresh worlds need fresh schedules; (4) bevy 0.20 rejects relationship inserts to unspawned entities (dangling ChildOf unreachable via public API — checker stays as defense-in-depth); (5) MessageVisitor captures only the message field — command output must be formatted strings, not structured fields; (6) iter_entities order is nondeterministic — lookups by Name query, always; (7) the 10 GB sandbox cannot hold debug+release trees — release gate carries a loud disk-aware skip and CI enforces it (R-032).

## [phase9] Cinematics + world building + plugins (P9-0..P9-7)

- WHAT: gens-cinematics (member 18) — typed keyframe tracks (Float/Vec3/Quat, Linear/Step/Smoothstep, loud empty/non-finite rejection), validated timelines ((target, Channel) BTreeMap, events in (0,duration], CameraCut intent), deterministic player (advance(dt) takes the fixed delta; half-open exactly-once event windows; loop wrap tail-then-head; multi-cycle deltas fire every cycle; typed delta errors); gens-runtime/src/cinematic — CinematicStage + CameraPose + advance_and_apply (advance->sample->validate->write, rewind-on-failure, per-entity field folds); cinematic_hello (ADR-0030 proof: replay + seek-equivalence + observed post-finish stability; gens-cinematic-1). gens-worldgen (member 19) — Noise2D (SplitMix64 lattice, exact [0,1) extraction, smoothstep value noise, validated FBM), HeightField (loud non-finite rejection, central-difference normals, FNV-1a content hash), scatter ((seed,cell)-derived jittered lattice, two independent channels), WorldGen (world-coord terrain sampling so chunks join seamlessly; chunk-LOCAL meshes so hashes address shape), ChunkStreamer (Chebyshev disc, differential sorted plans); worldgen_hello (ADR-0031 proof: order/streaming/restart through the ECS; gens-worldgen-1). gens-plugin (member 20) — Plugin/PluginHost (real typed bevy systems, registration-order application), loud duplicate/empty names, AppliedPlugins evidence record; plugins/hello-plugin OUTSIDE the workspace (root excludes it) proving the API compiles and runs for a downstream user (ADR-0032; gens-plugin-hello-1). Editor — Game View renders through the director's CameraPose (render_with_camera refactor), Animator shows the live track table + progress, Network tab takes a session snapshot (live sessions would break byte-identity on packet timing — the empty state says so); editor_hello runs a real 0.5s take and switches to Game View at step 20. CI — cinematic/worldgen/plugin smokes on 3 OS (the plugin smoke compiles a downstream crate: a gate on the API); verify.sh 18 gates.
- WHY: README row 9 "Cinematics + world building + plugins"; Phase 8 handoff open threads (Game View/Animator empty states, Network counters stranded in gens-networking) closed where CI-provable.
- EVIDENCE: 468 workspace tests green (gens-cinematics 32, gens-worldgen 29, gens-plugin 3, editor 40); runs #111-#119 (#111 cancelled superseded, #112-#119 green); local verify.sh PASS 18/18; two-run artifacts byte-identical for all three proof reports; proof values pinned (seek target 0.9999997 bits 3f7ffffb; plugin orb angle 3ffffff6 = 120x(1/60) — the same honest f32 signature).
- LESSONS: (1) f32 clocks accumulate — seek targets are exact state bits, never nearby literals, and stability windows are observed not assumed; (2) proof INPUT data must be platform-independent too — f32 literals only, no runtime sin/cos; (3) edition 2024 reserves `gen`; (4) NaN never equals NaN — assert errors by pattern; (5) the fmt gate [2/18] caught a drift locally BEFORE push (the #98 lesson is now habit); (6) R-032 ENOSPC x3 — clean-passes + CARGO_INCREMENTAL=0 + ~/.cache purge + shared CARGO_TARGET_DIR for the out-of-tree crate; zero work lost.

## [phase10] Final testing + benchmarks + validation (P10-0..P10-5)

- WHAT: P10-1 repo hygiene — P9-7 had committed plugins/hello-plugin/target/ (697 files, ~44 MB binaries; the root /target rule does not match plugin-local target dirs and no gate checks hygiene); untracked index-only (no history rewrite per the rollback plan), .gitignore gains plugins/*/target/. P10-2 gens-cinematics/src/document.rs — the gens-cine-1 serialized cinematic format (ADR-0033): every f32 travels as 8-digit IEEE-754 hex bits (decimal text is the corruption channel for determinism-critical floats), loading rebuilds through the real constructors so zero validation code can drift, canonical byte form + FNV-1a content_hash (the heightfield convention), core types stay serde-free; 9 tests (41 lib total). P10-3 bench_step 3→5 scenarios (ADR-0029 methodology unchanged): worldgen_stream_r2 (radius-2 streamer, focus flips 2 chunks/10 steps, real load/unload churn, mesh hashes folded) + cinematic_camera_hero (advance→sample→validate→write through the live ECS, looped 2s take, f32-literal keys); gens-bench-1 format backward compatible. P10-4 gens-runtime/tests/final_integration.rs — the cross-pillar suite: timeline LOADED from a gens-cine-1 document, terrain/scatter/floor height from gens-worldgen, physics ball rest ASSERTED from its own position-bits ledger, save/restore at step 90 with the director seeked to the checkpoint's EXACT accumulated f32 time (R-033), restored tail == original tail byte-for-byte, fresh replay == run A end to end; the save format's no-velocity boundary is documented and modeled honestly (restored ball fixed at its asserted-rest pose). P10-5 gens-build/examples/release_manifest.rs — gens-release-1 (ADR-0034): members/ADRs/risk-ids/roadmap/wire-format-tags derived from the repo, SELF-VALIDATES its shape (>=20 members, >=30 ADRs, >=25 risks, 11 roadmap rows in phase order, >=5 formats) and exits non-zero on drift; verify.sh 19 gates (gate [18/18]), CI step on 3 OS, manifest joins artifacts.
- WHY: README row 10 "Final testing + benchmarks + validation" — the last roadmap row; Phase 9 handoff threads closed where CI-provable (serialized cinematic format, worldgen/cinematic bench rows); the P9 hygiene defect found by the pre-flight audit.
- EVIDENCE: runs #123-#127 all green on 3 OS; local verify.sh PASS 19/19 (transcript docs/evidence/phase10/verify_19gates.txt, R-032 linker bus-error at 347 MB free captured mid-file, cargo clean → PASS); manifest inventories 20 members / 34 ADRs / 35 risks / 10 wire formats / 11 roadmap rows; final integration green in CI (log excerpt evidence).
- LESSONS: (1) a repo-hygiene gate is a real gap — add-ons that change WHERE output goes (out-of-tree targets) silently defeat path-scoped ignore rules; (2) world-hash assertions must watch WHAT can actually move — the director animating means the world hash moves even when physics is asleep, so rest claims need entity-scoped ledgers; (3) the sandbox kills long background processes between tool calls — run gates in the foreground with resume-from-cargo-cache, or chunk them; (4) rustup reinstall after R-002 takes 3 minutes and the pinned-channel playbook worked as written; (5) byte scanners over docs need char-boundary guards — punctuation will find you.
