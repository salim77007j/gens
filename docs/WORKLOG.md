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
