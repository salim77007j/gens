# Phase 0 Report — Risk Management and Safe Development

- **Branch:** `phase0-risk-management`
- **Report commit:** this commit (last entry of the ledger below)
- **Date:** 2026-10-11 (UTC)
- **Phase status:** **COMPLETE LOCALLY — PUSH BLOCKED BY INVALID CREDENTIAL** (details in §9; the moment a valid token is provided, the documented push sequence finishes the phase)

## 1. Objective

Establish the risk-managed development process before any engine code exists, per Phase 0 §0.1–§0.10: environment protection, branch discipline, build safety, change isolation, experiment safety, a 5-minute rollback capability, automated CI quality gates, incremental verification, documented decisions and risks, and a strict no-user-testing policy (CI does the testing).

## 2. Requirement → implementation map

| § | Requirement | Implementation | Evidence |
|---|---|---|---|
| 0.1 | Fresh clone, verified state, evidence in repo | Clone of empty public repo verified via `ls-remote` (0 refs) + clone warning; evidence directory established | `docs/evidence/phase0/clone-verify.txt`, `environment.txt` |
| 0.2 | Branch per phase, tags before phases | Work on `phase0-risk-management`; `main` untouched until phase verified; `pre-phase1-safe` tag defined (pending push, §10) | ADR-0005, `docs/ROLLBACK_PLAN.md` |
| 0.3 | Compile/test/size checks for every change | `scripts/verify.sh` mirrors CI gates; executed green this phase (no Rust yet by design) | `docs/evidence/phase0/verify-run.txt` |
| 0.4 | One logical change per commit; WHAT/WHY/HOW messages | 12 single-purpose commits, all with structured messages | `git log` ledger in §4 |
| 0.5 | Experiments isolated + documented | `experiment/<topic>` policy + log template | `docs/EXPERIMENTS.md` |
| 0.6 | Rollback within 5 minutes | 7-scenario playbook with time budgets; live drill executed (marker → revert) | `docs/ROLLBACK_PLAN.md`, `docs/evidence/phase0/rollback-drill.txt` |
| 0.7 | CI gates on every push; failed gate rejects push | `.github/workflows/ci.yml`: docs-integrity gate, credential secret scan, 3-OS rust matrix (fmt, clippy `-D warnings`, build, test, headless smoke, size gate) — armed, self-activating in Phase 1 | ADR-0004, workflow file |
| 0.8 | Incremental verification after every sub-item | `verify.sh` after every sub-item; one defect found and fixed (worklog timestamp drift, §7) | worklog entries C1–C10 |
| 0.9 | Documented decisions and risks | 8 ADRs; 16-entry risk register with scoring and detail notes | `docs/DECISIONS.md`, `docs/RISKS.md` |
| 0.10 | No user testing; CI-compatible methods | Everything this phase is CI-verifiable by construction; headless strategies documented (xvfb/software-GL paths named for Phases 1–2, 7) | ADR-0003/0004, RISKS R-004/R-014 |

## 3. Environment inventory

- Platform: Linux 5.10 (x86_64), sandbox session
- git 2.47.3; **Rust toolchain NOT installed locally** (Phase 0 needs no compilation; rustup install is risk R-015, scheduled before Phase 1 build work — CI installs its own toolchain regardless)
- Network: github.com reachable (HTTP 200); repository `salim77007j/gens` exists, **public**, **empty** at clone time
- Credential handling: `GH_TOKEN` environment variable read by a git credential helper at push time; nothing secret stored in `.git/config`, files, or commits

## 4. Commit ledger (one logical change per commit)

| # | Commit | Title | Notes |
|---|---|---|---|
| C1 | e194fd0 | chore(repo): bootstrap repository skeleton | .gitignore, .editorconfig, LICENSE-MIT, README, WORKLOG scaffold |
| C2 | 87336db | chore(evidence): environment, clone verification, auth diagnosis | first push attempt failed here (401) |
| C3 | 54b5628 | docs(governance): ADR-0001..0008 + risk register R-001..R-016 | |
| C4 | 2cf7182 | docs(process): rollback plan + experiment policy | |
| C5 | 308f87d | ci: 3-OS quality gates | |
| C6 | 09619a1 | build(safety): local verification gate + toolchain pin | verify.sh green |
| C6b | d157ad8 | docs(worklog): record build-safety tooling evidence | |
| C7 | 562ecc7 | docs(design): archive editor design reference | Phase 7 input preserved in-repo |
| C8 | 62d5ff1 | test(rollback): drill marker | to be reverted — by design |
| C9 | 90096b8 | Revert "test(rollback): drill marker…" | drill executed |
| C9b | 3a35fa3 | docs(worklog): correct timestamps to git times | honesty fix, disclosed |
| C10 | *(this commit)* | docs(report): phase 0 report + finalized drill evidence | |

## 5. CI configuration and current status

The workflow has two jobs: `gate / docs integrity` (always active — fails the push if any governance document is deleted or a credential pattern is committed) and `gates / {ubuntu,windows,macos}` (format, `clippy -D warnings`, build, test, Linux headless smoke, binary-size gate). The rust steps **self-detect** `Cargo.toml`: absent → armed (skip with notice); present → enforced. This means the workflow can be merged now and cannot go red for missing engine code, yet becomes fully enforceable the instant Phase 1 lands the workspace.

**Current run status: no runs have executed.** GitHub Actions only runs workflows after a successful push, and every push failed with `Invalid username or token` (§9). The YAML was validated locally with a real parser, and `verify.sh` (which mirrors the same gates) passed. CI-green is therefore **pending, not claimed** — the phase exit checklist in §8 reflects this honestly.

## 6. Build safety in practice

`scripts/verify.sh` ran after every sub-item this phase (gates 0–1 active, 2–6 correctly reporting "armed, not active"). It caught one process defect: my hand-written worklog timestamps had drifted ahead of the real clock by up to 13 minutes. Fix: all timestamps re-synced to `git log` times (the authoritative source) in commit 3a35fa3, with the correction disclosed in its own worklog entry. This is exactly the kind of drift the incremental-verification requirement exists to catch.

## 7. Rollback drill

Marker commit 62d5ff1 created and instantly reverted by 90096b8; local elapsed time under one second against a 5-minute budget; clean tree after; forward-moving history (no rewrite). Full transcript: `docs/evidence/phase0/rollback-drill.txt`. The push component of the drill publishes automatically with the first successful push since the drill commits are part of the branch.

## 8. Phase exit criteria — honest scorecard

| Criterion | Status |
|---|---|
| Governance docs complete (WORKLOG, DECISIONS, RISKS, ROLLBACK, EXPERIMENTS) | **Met** |
| CI workflow present, gates armed, YAML validated | **Met** |
| verify.sh local gate passing | **Met** |
| Rollback capability demonstrated (drill) | **Met (local)** |
| Design reference archived for Phase 7 | **Met** |
| Decisions + risks documented | **Met** |
| All commits pushed to origin | **NOT met — blocked** (R-001) |
| CI runs green on origin | **NOT met — blocked** (R-001) |
| `pre-phase1-safe` tag on origin | **NOT met — blocked** (R-001) |

The three unmet items have one shared root cause and one shared fix (§10). Phase 0 is not declared "done" until they clear; no Phase 1 work starts before that.

## 9. Blocker: push credential invalid (R-001, Realized)

Facts (full transcript in `docs/evidence/phase0/auth-check.txt`):

1. `GET /user` with `Authorization: Bearer <token>` → HTTP 401 `Bad credentials`.
2. `GET /user` with `Authorization: token <token>` → HTTP 401 `Bad credentials`.
3. `git push -u origin phase0-risk-management` → `remote: Invalid username or token`; authentication failed.

The token format itself is well-formed (classic PAT shape), so the credential has been revoked, expired, or mistyped upstream. It was also shared in plaintext in chat, so it must be treated as burned **regardless**: revoke it and generate a fresh one.

Mitigations applied this phase: env-only credential storage; secret-scan gate; granular local commits so nothing is lost; transparent logging instead of silent push-skipping; the environment-loss window is logged as risk R-002 with the recovery procedure.

## 10. Resume procedure (executes in ~5 minutes once unblocked)

```bash
export GH_TOKEN=<fresh token — classic 'repo' scope, or fine-grained with Contents: Read and write on salim77007j/gens>
cd /home/z/my-project/gens
./scripts/verify.sh                              # local gates green
git push -u origin phase0-risk-management        # publishes all 13 commits; CI runs
# verify branch CI green (docs-gate + armed rust matrix)
git checkout -b main && git push -u origin main  # bootstrap: main = verified Phase 0 head
# verify main CI green
git tag -a pre-phase1-safe -m "Phase 0 complete: process, docs, CI armed" && git push origin pre-phase1-safe
# optional hardening: branch protection via API (no required checks yet — see R-008/R-009)
```

A ready-made script implementing this sequence lives at `/home/z/my-project/scripts/resume_push.sh` (session-side, not part of the repo).

## 11. Decisions and risks delta

- ADRs added: 0001 (workspace), 0002 (bevy_ecs facade), 0003 (wgpu + software fallback), 0004 (CI-first), 0005 (branch/tag discipline), 0006 (licensing), 0007 (governance set), 0008 (public repo).
- Risks added: R-001..R-016; R-001 marked **Realized** with mitigations and user actions; R-002 monitoring the unpushed-work window; R-015 tracks the missing local Rust toolchain.

## 12. Verdict

Phase 0 delivers everything it can deliver without a working credential: the process, the docs, the gates, the drill, and the evidence trail — with the three remaining exit criteria explicitly blocked and a five-minute, fully documented path to clear them. Per the STOP protocol, no Phase 1 work begins until the push succeeds and this phase is declared complete.

## 13. Correction addendum (2026-10-12, recorded on `phase1-core`)

Phase 0 was later completed on origin: branch pushed, CI green (runs [#1](https://github.com/salim77007j/gens/actions/runs/38162440721) and [#2](https://github.com/salim77007j/gens/actions/runs/38162508120)), `main` bootstrapped from the verified head, tag `pre-phase1-safe` pushed. All three "NOT met" items in §8 cleared.

The §9 blocker diagnosis is also corrected. A credential byte-identical to the one diagnosed as "invalid" later authenticated and pushed successfully. Root cause: the sandbox executes each shell command in a **fresh environment** — an `export` performed in one tool call does not exist in the next. Every failing test (both API 401s and the git rejection) ran in a call that did not carry the credential, so GitHub was handed an empty secret. The honest conclusion is therefore: *the Phase 0 credential was never proven invalid; the test harness lost it between calls*. The §9 narrative above is preserved unedited as the record of what was (wrongly) believed at the time; this section and the addendum in `docs/evidence/phase0/auth-check.txt` supersede it. New risk R-017 captures the operational lesson: authenticated commands must carry their credential in the same invocation, always.
