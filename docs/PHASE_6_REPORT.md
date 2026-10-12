# Phase 6 Report — Networking + Data Assets (content-defined scenes, state across the wire)

**Status: COMPLETE and CI-verified.** Branch `phase6-networking-data-assets`, 7 sub-item commits, all green on Windows / Linux / macOS (runs #70–#76). `main` = merge commit, tag `pre-phase7-safe`.

## 1. What was delivered

### 1.1 gens-assets — data documents (P6-1; ADR-0023)

`DataDocument` is the engine's scene / prefab format: versioned JSON (`format_version` 1), entities in authored order with name (unique), `active`, optional `tag`, TRS (unit-quaternion validated within 1e-4, scale finite and non-zero), and optional script bindings with initial variables. `DataVar` is a mirror vocabulary (`Number(f64)`/`Flag`/`Text`) so gens-assets stays free of the scripting engine — the runtime bridge converts with kind checks, the same decoupling the save format uses (ADR-0021). Validation is loud (`DataError`: wrong version, empty/duplicate names, non-finite TRS, non-unit rotation, zero scale, empty script bindings); `import_data_document` validates BEFORE registering, so invalid content never enters the registry. Canonical JSON is byte-stable (struct field order + `BTreeMap` vars). `AssetKind::Data` added; the `.json` extension re-mapped from the never-used `UiLayout` to `Data` (UiLayout keeps `.uilayout`). 14 new tests.

### 1.2 gens-runtime — the document → world bridge (P6-2)

`gens-runtime::data::apply_data_document` re-validates (programmatic documents get the same contract), then spawns entities in authored order — `Name`, `Transform` (TRS verbatim), `Active`, `Tag`, `ScriptComponent` with `DataVar → ScriptValue` conversion and empty requests. Byte-equality tests prove the data path and the save path agree exactly: `data → world → save → fresh world → capture` equals `data → world → capture`. Honest test-expectation fix: bevy 0.16+ reserves low entity indices for resource entities, so the asserted contract is *relative* consecutiveness (spawn order == authored order), not absolute index 0. 6 bridge tests.

### 1.3 gens-networking — QUIC loopback transport (P6-3; ADR-0022)

`LoopbackCert` (rcgen self-signed, DER in memory; the client trusts exactly the operator-provided certificate — no system roots, no TOFU) and `QuicEndpoint`/`QuicLink`: each endpoint owns a **dedicated, permanently-driven I/O thread**; frames are `[u32 len][u64 FNV(payload)][payload]`, hash-verified on receipt with a 16 MiB pre-allocation guard; link establishment ends with an empty announce frame the server consumes during `accept`. Provider parity: quinn pinned with `rustls-ring`, rcgen with `ring` — no aws-lc-rs build burden on Windows CI. Three empirical findings shaped the architecture, each reproduced before being fixed and now documented in the WORKLOG:

1. **quinn transmits only while its runtime is being driven.** A facade that `block_on`s per call freezes outbound frames the moment its main future completes — a handshake can complete against a live peer while the peer's reply sits untransmitted. The permanently-driven I/O thread removes that failure class.
2. **QUIC announces a stream only when its first byte is written.** `open_bi` alone leaves the server's `accept_bi` waiting forever; the announce frame makes establishment deterministic.
3. **Runtime drop cancels queued teardown frames.** Without a graceful shutdown the `CONNECTION_CLOSE` raced the runtime drop (flaky 2/5); `Shutdown{grace}` + thread join fixed it.

### 1.4 gens-networking — sessions (P6-4)

Strict message vocabulary (`Snapshot`/`Input`/`Bye` — unknown kinds rejected; a `Custom(u8)` escape hatch would let two disagreeing builds "work" while corrupting each other's semantics), envelope codec (`tick u64 | kind u8 | payload_len u32 | payload_hash u64 | payload`) as pure, byte-exact-tested functions, and a `HELLO`/`WELCOME`/`REJECT` handshake with protocol-version pinning. The envelope's payload hash is deliberately independent of the transport's per-frame hash (belt and suspenders): a future transport swap cannot silently weaken integrity. Ticks are caller-assigned simulation state; session ids are host-assigned and injected — this layer never invents identity.

### 1.5 gens-runtime — world snapshots + `net_hello` (P6-5)

`gens-runtime::net::send_world_snapshot`/`recv_world_snapshot`: `capture_world` → canonical save JSON → `Snapshot` envelope. Integrity rides three independent layers (transport frame hash, session envelope hash, the save document's own hash when written to disk). The `net_hello` integration proof: a scene defined entirely as a data document (registry hash logged into the evidence) spawns a scripted patrol agent through the P6-2 bridge; the host simulates and replicates the world at step K over real loopback QUIC; the client applies the snapshot into a fresh world and continues to step N. **Host and client final captures are byte-identical, and the script's tick counter reads 600/600** — the Phase 5 save/load continuation property, now across the wire. Scope statement: state replication, not content streaming — assets stay host-side.

### 1.6 CI + local gates (P6-6)

`net_hello` runs on every OS on every push (loopback QUIC, in-memory certs, no external network); `smoke_net.json` joins the artifact set; `scripts/verify.sh` renumbered to 11 gates.

## 2. Success criteria scorecard (honest)

| Criterion | Result |
|---|---|
| Data-driven content on the Phase 5 registry | **Met** — `scenes/net-demo.json` imports through `import_data_document`; its FNV hash is logged into the example evidence |
| Scenes instantiate into worlds deterministically | **Met** — data path and save path agree byte-for-byte (unit-pinned) |
| Real QUIC transport, no external network in CI | **Met** — loopback 127.0.0.1, ephemeral ports, in-memory self-signed certs; green on Windows/Linux/macOS |
| State crosses the wire with the Phase 5 hash discipline | **Met** — three independent hash layers; corrupted frames/payloads rejected loudly (unit-pinned) |
| Wire continuation replays bit-identically | **Met** — `net_hello`: host and client final captures identical, script ticks 600/600, on all three OS |
| Async stays out of the engine | **Met** — the public surface is synchronous; tokio is confined to gens-networking internals |
| Multi-client sessions, content streaming, loss recovery | **Not claimed** — one client per session; state snapshots, not deltas; see §6 scope decisions |

## 3. Numbers

- Workspace: **300 tests passed, 0 failed, 8 ignored** (GPU) — 41 new this phase (gens-assets +14, gens-runtime +8, gens-networking +18... net: +41 total).
- `net_hello` evidence: 600 steps, replication at 300, snapshot payload **544 B**, evidence JSON **1571 B** — byte-identical output on Windows, Linux and macOS (evidence, not a claimed guarantee — R-013 discipline).
- Scene document: 705 B, FNV `6141612941022299661`.
- `gpu_hello` debug binary: 252,130,152 B (240.5 MiB) — grew from 211.5 MiB with quinn/tokio/rustls linked into the workspace; still far under the 350 MiB gate (ADR-0012).
- Dependencies pinned (crates.io max-stable, 2026-10-12): tokio 1.53.2, quinn 0.11.12, rcgen 0.14.10 (MSRVs 1.85/1.88/1.71, all ≤ 1.99).

## 4. Issues found and fixed during the phase (honest log)

1. **The per-call-`block_on` facade froze outbound frames.** Three tests deadlocked (server "accept timeout") while the client saw a completed handshake — quinn transmits only while its runtime is driven. Fixed by the dedicated I/O thread; the finding is documented where the next maintainer will meet it (transport module docs + WORKLOG).
2. **`accept_bi` waited forever for a stream the client had opened but never written.** QUIC lazily announces streams on first byte; fixed by the announce-frame protocol (client sends an empty frame, server consumes it during accept).
3. **Flaky `CONNECTION_CLOSE` (2/5 runs).** Runtime drop cancelled queued teardown frames; fixed by graceful `Shutdown{grace}` + join and bounded link drains. The flake hunt ran the full crate suite 8+ consecutive parallel times until green, and the tests were made deterministic (acceptance-signal handshakes instead of timing races) — the flakiness had also contaminated earlier "passes".
4. **`tokio::time::timeout` panics when constructed outside `block_on`** ("no reactor running") — timers register at construction. Fixed by creating the timer inside the driven context; the constraint is commented at `with_timeout`.
5. **bevy 0.16+ entity indices start where resource entities left them** — the P6-2 test initially asserted absolute index 0 and failed; the contract (relative consecutiveness) is now what's tested.
6. **Sandbox toolchain wiped by the environment reset** (R-015/R-017 reality): rustup 1.99.0 reinstalled mid-phase, same pin as CI; zero auth failures (per-call token discipline held).
7. **Repository hygiene: the default branch was still `phase0-risk-management`** (GitHub had kept the first-pushed branch since Phase 0; fresh clones got a stub workspace). Fixed to `main` via API, disclosed in WORKLOG P6-0.
8. **Disk exhaustion (twice)**: target/ + the new networking stack hit the 9.9 GB sandbox limit; linker SIGBUS was the symptom. `CARGO_INCREMENTAL=0` held; cleaned and re-ran the affected gates honestly.

## 5. CI runs

| Run | Commit | Result |
|---|---|---|
| #69 | 7db9eb0 (main, pre-phase6 anchor) | success |
| #70 | 111a594 (P6-0 pins) | success |
| #71 | 3fda745 (P6-1 data documents) | success |
| #72 | dab1999 (P6-2 bridge) | success |
| #73 | 8ff4682 (P6-3 transport) | success |
| #74 | da49f72 (P6-4 sessions) | success |
| #75 | a055b50 (P6-5 snapshots + net_hello) | success |
| #76 | 25704ea (P6-6 CI smoke) | success |

Cross-OS net smoke (run #76): 544 B snapshot, 1571 B evidence, "host/client captures IDENTICAL, client ticks 600" on all three runners. Ledger: `docs/evidence/phase6/ci-runs.txt`.

## 6. Decisions & risks delta

- **ADR-0022** — networking: quinn/QUIC, loopback-first, sync facade over a permanently-driven I/O thread; async never escapes gens-networking.
- **ADR-0023** — data assets: versioned JSON documents on the registry; mirror vocabulary; loud validation; not a reflection layer (R-023 scope unchanged).
- **ADR-0024** — session protocol: strict kind vocabulary, version-pinned handshake, envelope hash independent of the transport hash.
- **Risks**: R-013 narrowed further (cross-OS byte-identical net evidence recorded, still not claimed as a guarantee); new **R-025** (loopback-only transport — Internet operation, NAT, congestion and hardening unproven), **R-026** (quinn/tokio upstream churn), **R-027** (replication scope: snapshots, one client, no deltas/interest management).

## 7. What Phase 7 can build on

A world whose *scene* is content and whose *state* crosses the wire verifiably: the editor (Phase 7) can be driven by data documents for its scene model, and the session layer is the seam a network inspector/debugger would attach to. The renderer's offscreen capture (Phase 2) plus the UI stack (Phase 4) remain the visual substrate the editor windowing will hang off.
