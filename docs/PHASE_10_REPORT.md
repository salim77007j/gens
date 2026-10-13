# Phase 10 Report — Final Testing + Benchmarks + Validation

**Branch:** `phase10-final-validation` · **Base:** `65dfe01` (main, run #122 green, tag `pre-phase10-safe`)

## 1. What shipped

### 1.1 Repository hygiene (P10-1)

The pre-flight audit found that P9-7 (`78a9387`) had committed the out-of-tree plugin's build directory — 697 files under `plugins/hello-plugin/target/`, including a 43.9 MB debug binary. The root `/target` ignore rule does not match a plugin-local `target/`, and none of the 18 gates check repository hygiene, so the defect was invisible to every gate (GitHub's 100 MB per-file limit happened to hold, CI stayed green). Fixed index-only (`git rm -r --cached`, no history rewrite per the rollback plan), `.gitignore` gains `plugins/*/target/` with the incident documented inline. The repo stops carrying ~44 MB of build output from this commit forward.

### 1.2 The `gens-cine-1` cinematic document format (P10-2, ADR-0033)

Cinematic timelines were authored in code only — the Phase 9 handoff named the serialized format as natural polish, and row 10's "validation" demanded the wire story be closed. `gens-cinematics::document` defines the format under the two rules this repo has paid for:

- **Exact f32 fidelity** — every float travels as its IEEE-754 bit pattern in 8-digit hex, so parse yields the exact bits back (decimal text is exactly the corruption channel that would break the `3f7ffffb` seek signatures).
- **Validation is the constructors'** — loading rebuilds the timeline through `FloatTrack`/`Vec3Track`/`QuatTrack`/`TimelineBuilder` themselves: zero new validation code exists to drift, and a document is either a valid `Timeline` or a typed `DocumentError` (Parse/FormatTag/Channel/Interpolation/FloatBits/TrackData/TrackKind/Timeline).

Canonical form is byte-stable for equal timelines (tracks in `(target, channel)` order, events in firing order), so the FNV-1a `content_hash` over the canonical bytes pins content identity. Core timeline types stay serde-free. 9 new tests pin bit-exact roundtrips, canonical+hash stability, format-tag enforcement and loud rejection of event-window/duplicate-time/kind-mismatch/+inf/malformed-bits documents.

### 1.3 Benchmarks: the new pillars are measured, not just proven (P10-3)

`bench_step` grows from 3 to 5 scenarios — the Phase 9 handoff's explicit ask — under the unchanged ADR-0029 methodology (fixed sampling, environment disclosure, advisory-only):

- **`worldgen_stream_r2`** — a Chebyshev-radius-2 streamer whose focus flips 2 chunks every 10 steps, so every sample carries real differential load/unload traffic (noise + heightfield + mesh + content hash per loaded chunk); checksum folds mesh hashes and unload coords.
- **`cinematic_camera_hero`** — the director composition through the live ECS (`advance → sample → validate → write` per fixed step, looped 2 s take with fov/translation/rotation tracks + one event); checksum folds fired events and the hero's written transform bits. All key values are f32 literals (the platform-independence rule).

Format `gens-bench-1` is backward compatible; no new dependency. Local smoke (debug/linux): `worldgen_stream_r2` median 9.16 ms, `cinematic_camera_hero` 0.39 ms per 20 steps (vs `physics_50` 44.3 ms).

### 1.4 Final testing: the cross-pillar integration suite (P10-4)

Every existing smoke proves a pillar pair; row 10's "final testing" demanded proof that the pillars **compose end to end**. `gens-runtime/tests/final_integration.rs` runs one deterministic CPU-only scenario where:

- the director's timeline is **loaded from a `gens-cine-1` document** (the P10-2 format feeding the real pipeline, `content_hash` asserted stable);
- terrain placement, prop scatter and the physics floor height come from **`gens-worldgen`** (chunk at the origin, `height_at` under the floor);
- **`gens-physics`** drops a ball whose rest is **asserted from its own position-bits ledger** (the world hash keeps moving while the director animates — rest is asserted against the ball ledger, not the world hash);
- at step 90 the **save format** captures the world; a fresh world restores from the JSON and the director seeks to the checkpoint's **exact accumulated f32 time** captured from the live player (the R-033 contract — never a nearby literal);
- the proofs: restored tail == original tail **byte-for-byte**, and a fully fresh replay == run A **end to end**.

Honest boundary (documented in the file header): the save format carries no rigid-body velocity (ADR-0021 scope). The original run asserts the ball was at rest before the checkpoint; the restored run re-creates it FIXED at its saved pose — at rest by construction, indistinguishable from the sleeping original — so the tail equality proves the save/restore+seek path itself, and any drift in any pillar fails loudly. Both tests pass on the local runner and ride `cargo test --workspace` on every push, on all 3 OS.

### 1.5 Validation: the `gens-release-1` self-validating manifest (P10-5, ADR-0034)

`gens-build/examples/release_manifest.rs` derives the release inventory from the repository itself — workspace members (parsed from the root `Cargo.toml`), ADR count, unique `R-###` risk ids, README roadmap rows with their honest statuses, and the `gens-*-1` wire-format tags discovered by scanning the workspace sources — then **validates its own shape** (≥20 members, ≥30 ADRs, ≥25 risk ids, exactly 11 roadmap rows in phase order, ≥5 wire formats) and exits non-zero on any discrepancy. It is gate [18/18] of `verify.sh` (now **19 gates**) and its own CI step on all three OSes; the manifest joins the smoke artifacts. A hand-written summary would rot; this cannot.

## 2. Run ledger (phase 10 branch)

| Run | Commit | Result | Note |
|---|---|---|---|
| #123 | `39d1285` P10-1 | success | repo hygiene (697 artifacts untracked) |
| #124 | `a7f2941` P10-2 | success | gens-cine-1 document format |
| #125 | `c1a3e05` P10-3 | success | worldgen + cinematic bench rows |
| #126 | `2a936b1` P10-4 | success | cross-pillar final integration |
| #127 | `4323e80` P10-5 | success | release manifest + gate 18 + CI step |

## 3. Issue log (found → fixed, with evidence)

1. **The P9 hygiene defect (found by audit, invisible to every gate)** — 697 committed build artifacts (~44 MB) from P9-7's `git add -A`. Fixed in P10-1; `.gitignore` now covers out-of-tree plugin targets; the gap ("no gate checks repo hygiene") is documented in the ignore rule itself.
2. **Toolchain lost mid-phase (R-002)** — the sandbox reset wiped `~/.cargo` entirely between sessions; the repo survived. Re-installed rustup + stable 1.99.0 with the pinned components (rustfmt, clippy) and re-ran the gates. The rollback playbook's re-install path worked as written.
3. **Env-var loss kept striking (R-017)** — `GH_TOKEN` and `PATH` reset between tool calls multiple times per session; the env-only credential helper + silent re-export pattern held, zero token material ever hit disk or output.
4. **`build_track` error-flow rework** — the first draft of the document loader used `?` across `TrackError`/`DocumentError` boundaries that had no `From` impl, and one `serde_json::Error::custom` call that isn't an inherent method. Reworked into two dedicated variants (`TrackData`, `TrackKind`) with the wrapping done where the target/channel context exists.
5. **The settle assertion watched the wrong ledger** — the first draft asserted the ball was settled from the WORLD hash; the hash keeps moving while the director animates (camera/hero are still playing at 1.5 s), so the assertion failed correctly. Fixed: rest is asserted against the ball's own position-bits ledger; the world hash remains the replay-comparison ledger.
6. **`Mode::Full` never captures** — run A was initially `Full` while the test expected a checkpoint capture from it; switched to `Checkpoint` (same 120 steps, plus the in-flight capture). The failure mode was a clean `expect` panic, not silent behavior.
7. **clippy held the line twice** — unused imports and a `collapsible_if` were caught locally before push (the #98 lesson is now habit); the `if let … && let` let-chain form replaced the nested if.
8. **R-032 disk incidents ×2** — the linker died with a bus error at 347 MB free during the evidence-capture verify pass; a second verify attempt at low disk failed the same way. `cargo clean` (7.8 GB recovered) → full 19-gate **VERIFY: PASS** transcript saved. Release size gate loud-skipped per the R-028 disk-aware rule; CI enforces it.
9. **Em-dash slicing panic** — the risk-id scanner indexed into a multi-byte character (`—`) and panicked; fixed with `is_char_boundary` guards and byte-wise scanning. The manifest's byte-stability guarantee now survives punctuation.

## 4. Honest scorecard

| Criterion (README roadmap: "Final testing + benchmarks + validation") | Status |
|---|---|
| Final testing | **Done** — cross-pillar integration suite (worldgen + physics + cinematic-from-document + save/restore + replay, byte-identical) rides `cargo test --workspace` on 3 OS; workspace test suite grew from 468 to 489 passed on the linux CI job (#127, including doctests) |
| Benchmarks | **Done** — bench_step covers 5 scenarios including the two new pillars; methodology, environment disclosure and advisory unchanged (ADR-0029) |
| Validation | **Done** — 19 local gates ↔ CI mirror; the `gens-release-1` manifest inventories the crate set, the decision ledger, the risk register, the roadmap and the wire formats, and fails loudly on drift (ADR-0034) |
| Serialized cinematic format (P9 handoff) | **Done** — `gens-cine-1` with exact-bits floats and constructor-routed loading (ADR-0033) |
| Repo hygiene (found this phase) | **Done** — 697 artifacts untracked, ignore rule added |
| Window shell (winit) | Still **deferred** — unchanged since Phase 8; no CI-provable story exists on a headless 3-OS matrix, and the honest answer at project end is the documented deferral (ADR-0028), not a fake window |
| Packaging texture/audio assets | Still **future work** — GENSPKG1 carries scripts + data documents; binary asset streaming stays on the documented roadmap-behind-the-roadmap |
| Network tab live sessions | Still **honest-empty in replays** — a live session would break byte-identity on packet timing; the limitation is stated where the empty state renders |

**Tests:** gens-cinematics 41 lib (was 32) + 1 doctest; gens-runtime +2 (final integration); gens-build 18 lib + manifest example; workspace carried green by CI on every push.

**Binary sizes:** gpu_hello debug 180.1 MB (gate 350 MiB — measured in the local 19-gate PASS transcript); release gate enforced by CI runners (local run loud-skipped at <3 GB free per R-028).

## 5. Decisions & risks delta

- **ADR-0033** — cinematic documents carry exact f32 bits and load through the constructors; no decimal corruption channel, no second validation path.
- **ADR-0034** — the release manifest is a self-validating gate whose thresholds hard-code the phase-10 inventory on purpose; drift fails, it is not described politely.
- **RISKS**: **no new risks opened.** Phase 10's changes are additive (a format, two bench rows, two tests, one gate); every pre-existing risk's mitigation held this phase (R-002 re-install, R-017 env discipline, R-028 loud skips, R-032 clean-pass playbook, R-033 exact-bit seeks applied in the integration test). R-013's cross-platform caveat remains the one documented trust boundary — CI's 3-OS green is the evidence.

## 6. Evidence index

- `docs/evidence/phase10/ci-runs.txt` — the ledger above (#123–#127)
- `docs/evidence/phase10/verify_19gates.txt` — the local 19-gate PASS transcript (with the R-032 incident captured mid-file)
- `docs/evidence/phase10/release_manifest.json` — the gens-release-1 document (crate set, ledger counts, roadmap, wire formats)
- `docs/evidence/phase10/final_integration.txt` — the cross-pillar suite green in CI (log excerpt: `cross_pillar_composition_replays_and_restores_byte_identically` + the workspace test totals)
- CI artifacts per run: `smoke_release_manifest.json` joins the existing snapshot set

## 7. Project completion note

This was the last roadmap row. Phases 0–10 are complete, and the engine's final state is exactly what the evidence ledgers say it is: 20 workspace crates, 34 ADRs, 35 tracked risks, 11 wire formats, a 19-gate verification discipline mirrored by CI on three operating systems — and a short, honest list of things it deliberately does not claim (the window shell, binary asset packaging, live networking inside replays). The deferrals are documented where a reader would look for them, which is the difference between an engine that is small and an engine that is honest about being small.
