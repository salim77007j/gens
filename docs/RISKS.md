# Gens — Risk Register

Likelihood and Impact are scored Low / Medium / High. Status is one of `Open`, `Monitoring`, `Mitigated`, `Realized` (risk materialized — link to the incident evidence). This register is updated in the same commit that changes the relevant code or process. Owners: the engineering team (automated agents) unless stated otherwise.

## Register

| ID | Risk | Likelihood | Impact | Mitigation (summary) | Status |
|---|---|---|---|---|---|
| R-001 | Push credential leaked or invalid | High | High | Env-only storage, secret-scan gate, rotation guidance, local-commit buffer | Corrected (process error, not credential failure - see detail note) |
| R-002 | Sandbox environment reset loses local state | Medium | High | Push after every sub-item; re-clone procedure in ROLLBACK_PLAN §4. Phase 2 note: the session start lost the credential again (env reset) and pushed work accumulated locally until the user re-exported the token mid-phase; window closed with zero loss | Monitoring |
| R-003 | CI unavailable / quota exhausted | Low | Medium | Public repo = free minutes; local `scripts/verify.sh` mirrors every gate | Monitoring |
| R-004 | Software rendering (lavapipe) unrepresentative of GPU perf | High | Medium | Report relative + absolute numbers with environment disclosure; 60 FPS criterion measured honestly, not fabricated. Phase 2 note: CI now renders REAL GPU frames on 3 adapters (llvmpipe/Vulkan, Metal, DX12 WARP-class); perf claims still wait for Phase 8 benchmarks | Open |
| R-005 | CI runner flakiness (Windows/macOS variance, network hiccups) | Medium | Medium | Retry policy (max 1 retry per job), fail-fast disabled, flaky-gate quarantine documented | Open |
| R-006 | bevy_ecs upstream breaking changes | Medium | Medium | Pin minor version; facade crate confines churn | Open |
| R-007 | Scope overrun / phase compression temptation | High | High | Hard phase gates, STOP-after-phase protocol, per-phase success criteria in phase reports | Monitoring |
| R-008 | Branch protection not enforceable via API token permissions | Medium | Low | Procedural discipline + phase-boundary CI verification before merge; revisit enforcement in Phase 1 | Open |
| R-009 | Required-status-checks would block direct main pushes of merge commits | Medium | Low | Keep protection advisory in Phase 0; adopt PR-flow with required checks only if friction justifies it | Open |
| R-010 | macOS runner/toolchain variance (Xcode noise) | Medium | Low | Pin runner image when artifacts depend on it; treat warnings as errors only in our code | Open |
| R-011 | Binary-size gate threshold unknown before Phase 1 | Low | Low | Placeholder in Phase 0; calibrate with real binaries in Phase 1 and record in DECISIONS | Open |
| R-012 | Upstream dependency regression blocks a phase | Medium | Medium | Pin versions; patch locally or send upstream PR (Global Rule 10); ROLLBACK_PLAN §5 | Open |
| R-013 | Cross-platform float determinism (physics/networking) | Medium | High | Fixed timestep, deterministic math discipline, cross-platform vector tests in Phase 3. **Phase 3 note:** in-process determinism delivered and CI-pinned (physics_hello replays 600 steps bit-identically, asserted on every push); cross-OS evidence: x86_64 Windows == x86_64 Linux bit-identical, ARM64 macOS differs in low bits (final_x 7.9050045 vs 7.9050112) — cross-platform bit-identity is NOT claimed and networking must not assume it. **Phase 4 note:** the replay pin now covers AI + animation + UI (`animate_hello`: positions, animation states AND egui UI frame hashes bit-identical across two runs every push); run #55 additionally produced byte-identical smoke JSON on all three OS (4100/4100/4100 B) — a strong data point that stays formally unclaimed as a guarantee | Open (narrowed) |
| R-014 | Headless editor screenshots degrade fidelity (Phase 7) | Medium | Low | Software GL path + egui screenshot capture; document limitations honestly | Open |
| R-015 | Rust toolchain absent in the current sandbox | High | Medium | rustup 1.99.0 installed in Phase 1 (local gates run); CI installs its own toolchain regardless | Mitigated |
| R-016 | Design reference misread → editor "copies" instead of "inspires" | Low | Medium | `docs/EDITOR_DESIGN.md` in Phase 7 must document extracted language AND Gens' divergences | Open |
| R-017 | Non-persistent shell environment between tool calls silently breaks authenticated commands | High | Medium | Every command that authenticates must carry its credential in the SAME invocation; never rely on cross-call state (discovered Phase 0→1, caused the false R-001 diagnosis). **Phase 4 note:** verified the reset granularity is EVERY tool call, not per session — env vars set in one Bash call are empty in the next; discipline holds (zero auth failures across 10+ pushes this phase) | Monitoring |
| R-018 | Realtime audio output unproven (cpal backend deferred; CI/sandbox lack ALSA headers) | Medium | Low | Mock backend proves the full scheduling/state machine headless (ADR-0014); realtime output is a feature-gated addition with build docs when windowing lands | Open |
| R-019 | rapier 0.36 / kira 0.12 upstream churn (fast-moving majors) | Medium | Medium | Exact version pins in workspace.dependencies; facade crates confine churn with a `backend` escape hatch; determinism + kinematics tests gate every upgrade | Open |
| R-020 | egui 0.36 upstream churn (fast-moving 0.x, API broke twice inside one minor: `run_ui`, `CentralPanel::show(&mut Ui)`, `MouseWheel.phase`) | Medium | Medium | Pinned 0.36.2; every API fact verified from vendored source BEFORE coding; egui confined to `gens-ui` (backend consumes pure `UiFrame`s, so the wgpu layer never sees egui); replay tests hash frames byte-for-byte and gate upgrades | Open |
| R-021 | Cross-platform UI rasterization bit-identity unproven (ab_glyph rasterization + tessellation are in-process deterministic only) | Low | Low | `animate_hello` asserts UI frame-hash equality in-process per run; cross-OS run #55 emitted identical hashes (4100 B JSON on all 3 OS) but no guarantee is claimed; networking/save systems must not assume it | Open |

## Detail notes for top-severity risks

### R-001 — Push credential leaked or invalid — **status correction: process error, not credential failure**

The token supplied for this project was shared in plaintext in the chat transcript and was found to be **invalid** at Phase 0 start: `GET /user` returns `401 Bad credentials` under both `Bearer` and `token` schemes, and `git push` returns `remote: Invalid username or token`. Evidence: `docs/evidence/phase0/auth-check.txt`.

**Correction (see the addendum in that file and the Phase 0 report §13):** a credential byte-identical to the one diagnosed later authenticated and pushed successfully. The 401s came from the test harness — the sandbox runs each shell command in a fresh environment, and every failing test ran in a call that did not carry the credential (empty secret presented). The original "invalid credential" conclusion was unproven. R-001 is reclassified from *Realized* to *process error, corrected*. The credential-hygiene mitigations (env-only storage, secret-scan gate, granular commits) remain in force regardless.

### R-002 — Sandbox environment reset loses local state

Until the first successful push, all work exists only in the local clone at `/home/z/my-project/gens`. This is the one window where the "never leave uncommitted work" rule cannot be fully honored (commits exist, pushes do not). Mitigations: every change is already a discrete local commit with a complete message (WHAT/WHY/HOW), so re-applying after a reset is mechanical; the full content is documented in this repository's own docs; the window is intentionally kept short by finishing Phase 0 locally in one session.

### R-007 — Scope overrun / phase compression temptation

Ten phases of engine development is a multi-session effort. The failure mode observed in prior projects (per the user's brief) is compressing phases and claiming success. Mitigations are structural, not aspirational: each phase has written success criteria that CI can verify; each phase report must reconcile evidence; and the STOP protocol (no Phase N+1 until the user says so) is enforced by the agent, not by memory. Honest failure reporting (`IF BLOCKED: STOP`) is preferred over green-washing — demonstrated by the credential blocker handling in Phase 0.
