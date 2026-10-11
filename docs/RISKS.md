# Gens — Risk Register

Likelihood and Impact are scored Low / Medium / High. Status is one of `Open`, `Monitoring`, `Mitigated`, `Realized` (risk materialized — link to the incident evidence). This register is updated in the same commit that changes the relevant code or process. Owners: the engineering team (automated agents) unless stated otherwise.

## Register

| ID | Risk | Likelihood | Impact | Mitigation (summary) | Status |
|---|---|---|---|---|---|
| R-001 | Push credential leaked or invalid | High | High | Env-only storage, secret-scan gate, rotation guidance, local-commit buffer | **Realized** (invalid credential at Phase 0 start) |
| R-002 | Sandbox environment reset loses local state | Medium | High | Push after every sub-item (once credentials work); re-clone procedure in ROLLBACK_PLAN §4 | Monitoring |
| R-003 | CI unavailable / quota exhausted | Low | Medium | Public repo = free minutes; local `scripts/verify.sh` mirrors every gate | Monitoring |
| R-004 | Software rendering (lavapipe) unrepresentative of GPU perf | High | Medium | Report relative + absolute numbers with environment disclosure; 60 FPS criterion measured honestly, not fabricated | Open |
| R-005 | CI runner flakiness (Windows/macOS variance, network hiccups) | Medium | Medium | Retry policy (max 1 retry per job), fail-fast disabled, flaky-gate quarantine documented | Open |
| R-006 | bevy_ecs upstream breaking changes | Medium | Medium | Pin minor version; facade crate confines churn | Open |
| R-007 | Scope overrun / phase compression temptation | High | High | Hard phase gates, STOP-after-phase protocol, per-phase success criteria in phase reports | Monitoring |
| R-008 | Branch protection not enforceable via API token permissions | Medium | Low | Procedural discipline + phase-boundary CI verification before merge; revisit enforcement in Phase 1 | Open |
| R-009 | Required-status-checks would block direct main pushes of merge commits | Medium | Low | Keep protection advisory in Phase 0; adopt PR-flow with required checks only if friction justifies it | Open |
| R-010 | macOS runner/toolchain variance (Xcode noise) | Medium | Low | Pin runner image when artifacts depend on it; treat warnings as errors only in our code | Open |
| R-011 | Binary-size gate threshold unknown before Phase 1 | Low | Low | Placeholder in Phase 0; calibrate with real binaries in Phase 1 and record in DECISIONS | Open |
| R-012 | Upstream dependency regression blocks a phase | Medium | Medium | Pin versions; patch locally or send upstream PR (Global Rule 10); ROLLBACK_PLAN §5 | Open |
| R-013 | Cross-platform float determinism (physics/networking) | Medium | High | Fixed timestep, deterministic math discipline, cross-platform vector tests in Phase 3 | Open |
| R-014 | Headless editor screenshots degrade fidelity (Phase 7) | Medium | Low | Software GL path + egui screenshot capture; document limitations honestly | Open |
| R-015 | Rust toolchain absent in the current sandbox | High | Medium | Install rustup with pinned stable toolchain before Phase 1 build work; CI installs its own toolchain regardless | Open |
| R-016 | Design reference misread → editor "copies" instead of "inspires" | Low | Medium | `docs/EDITOR_DESIGN.md` in Phase 7 must document extracted language AND Gens' divergences | Open |

## Detail notes for top-severity risks

### R-001 — Push credential leaked or invalid — **Realized**

The token supplied for this project was shared in plaintext in the chat transcript and was found to be **invalid** at Phase 0 start: `GET /user` returns `401 Bad credentials` under both `Bearer` and `token` schemes, and `git push` returns `remote: Invalid username or token`. Evidence: `docs/evidence/phase0/auth-check.txt`.

Immediate mitigations applied: (1) no credential is written to disk, `.git/config`, or the repository — the credential helper reads `GH_TOKEN` from the process environment at push time; (2) a CI secret-scan gate blocks any commit containing a GitHub token pattern; (3) all Phase 0 work is preserved as granular local commits on `phase0-risk-management`, so a single push completes the phase once a valid credential arrives.

Required user actions: create a fresh token (classic with `repo` scope, or fine-grained with **Contents: Read and write** on this repository only), export it as `GH_TOKEN`, and revoke the exposed/invalid token regardless — it was shared in plaintext and must be treated as burned. Residual risk after fix: none beyond normal token hygiene.

### R-002 — Sandbox environment reset loses local state

Until the first successful push, all work exists only in the local clone at `/home/z/my-project/gens`. This is the one window where the "never leave uncommitted work" rule cannot be fully honored (commits exist, pushes do not). Mitigations: every change is already a discrete local commit with a complete message (WHAT/WHY/HOW), so re-applying after a reset is mechanical; the full content is documented in this repository's own docs; the window is intentionally kept short by finishing Phase 0 locally in one session.

### R-007 — Scope overrun / phase compression temptation

Ten phases of engine development is a multi-session effort. The failure mode observed in prior projects (per the user's brief) is compressing phases and claiming success. Mitigations are structural, not aspirational: each phase has written success criteria that CI can verify; each phase report must reconcile evidence; and the STOP protocol (no Phase N+1 until the user says so) is enforced by the agent, not by memory. Honest failure reporting (`IF BLOCKED: STOP`) is preferred over green-washing — demonstrated by the credential blocker handling in Phase 0.
