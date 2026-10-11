# Gens — Rollback Plan

Objective (Phase 0 §0.6): any regression is reversible within **five minutes**, using only commands and artifacts documented here. Rollback always precedes debugging (§0.3): a broken tree is reverted first, the failure is reproduced in isolation, and only then is a fix re-attempted.

## 1. Principles

- **Tags before risk.** Before any major or experimental change, create an anchor: `git tag rollback/<description> && git push origin rollback/<description>` (branch protection permitting; on phase branches always possible). Phase boundaries get `pre-phase<N>-safe` tags on `main`.
- **Revert-first.** `git revert` (forward-moving history) is preferred over history rewriting. History rewriting (`reset` + `push --force-with-lease`) is permitted **only** on phase branches, never on `main`.
- **One escape hatch per change.** Every worklog entry for a major change names its rollback command explicitly.

## 2. Scenario playbook

| # | Scenario | Detection | Action | Budget |
|---|---|---|---|---|
| 1 | Bad commit on a phase branch | CI red / `verify.sh` fails | `git revert <sha>` on the branch; push | < 2 min |
| 2 | Phase branch badly broken (multiple failed experiments) | repeated red CI | `git reset --hard <last-good>`; `git push --force-with-lease origin <branch>` | < 2 min |
| 3 | `main` broken after a phase merge | `main` CI red | `git revert -m 1 <merge-sha>` on `main`; push; investigate on the phase branch | < 3 min |
| 4 | Local environment lost/reset | tool calls fail; clone missing | Re-clone from GitHub (§4), checkout the phase branch or tag, run `scripts/verify.sh` | < 5 min |
| 5 | Bad or mistaken tag | noticed at tag creation | `git tag -d <tag> && git push origin :refs/tags/<tag>`; re-tag correctly | < 1 min |
| 6 | Dependency upgrade breaks CI | CI red right after manifest change | Re-pin the previous version in `[workspace.dependencies]`; file the upstream issue; patch locally or send an upstream PR (Global Rule 10) | < 5 min to restore |
| 7 | Secret accidentally committed | secret-scan gate red | Revoke the credential **immediately**; `git filter-repo --replace-text` on the phase branch; force-with-lease push; rotate credentials | < 5 min (repo side) |

## 3. Pre-change checklist (major changes)

1. Working tree clean, `git status --short` empty, everything pushed.
2. Anchor tag created and pushed: `git tag rollback/<desc> && git push origin rollback/<desc>`.
3. The rollback command for *this specific change* is written into the WORKLOG entry that introduces it.
4. `./scripts/verify.sh` passes on the starting state.

## 4. Environment-loss recovery (Phase 0 §0.1)

```bash
git clone https://github.com/salim77007j/gens.git
cd gens
git checkout phase<N>-<slug>          # or a tag: pre-phase<N>-safe
./scripts/verify.sh                   # must pass — proves the restored state is consistent
```

Work resumes from the last pushed commit. Anything unpushed is considered lost **by design** — the process forbids leaving unpushed work behind (Phase 0 §0.1). Exception window: the credential-blocked interval at Phase 0 (see `docs/RISKS.md` R-002), recorded explicitly rather than hidden.

## 5. Secret-in-history procedure (referenced by ADR-0008)

1. Revoke the exposed token at GitHub (Settings → Developer settings → Tokens).
2. Create `replacements.txt` containing the literal secret and a placeholder.
3. `git filter-repo --replace-text replacements.txt --force` on an isolated copy of the branch.
4. Verify with `git grep` that the pattern is gone from all history.
5. `git push --force-with-lease`; re-clone fresh locally; rotate to a new credential.

## 6. Phase 0 rollback drill

A marker commit is created and then reverted, with wall-clock timings recorded, as concrete evidence that scenario #1 works end-to-end. Results: `docs/evidence/phase0/rollback-drill.txt` and `docs/PHASE_0_REPORT.md` §7. The drill ran locally due to the credential blocker; the push component of the drill is executed as part of the first successful push sequence.
