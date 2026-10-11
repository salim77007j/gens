> **Status: Phase 3 — Physics + Audio + Input.**
> Phases 0-2 complete and CI-verified (workspace, core runtime, ECS, math, offscreen-first wgpu renderer with GPU screenshots on 3 OS). Phase 3 adds rapier3d rigid-body physics, kira headless audio, and a pure action-mapping input model — composed and proven by a deterministic scripted-replay demo (`physics_hello`) that runs bit-identically on every push. See [`docs/PHASE_3_REPORT.md`](docs/PHASE_3_REPORT.md) and the live status in [`docs/WORKLOG.md`](docs/WORKLOG.md).


## Design Goals

1. **Performance first** — frame-time budgets, data-oriented design, GPU-driven rendering where the API allows it.
2. **Determinism where it matters** — fixed-timestep simulation, deterministic physics runs, replay-friendly architecture.
3. **Developer experience** — fast compiles, headless testability, every feature verifiable in CI without a GPU or a display.
4. **Cross-platform** — Windows, Linux, macOS as first-class targets; Web (WASM) as an export target.
5. **Honest engineering** — no feature is "done" without CI evidence: screenshots, metrics, logs committed to the repository.

## Architecture (planned)

| Crate | Responsibility |
|---|---|
| `gens-core` | Game loop, time, events, memory strategy, logging, errors |
| `gens-ecs` | Entity component system (bevy_ecs behind a stable facade) |
| `gens-math` | Vec/Mat/Quat math on glam + engine-specific utilities |
| `gens-renderer` | wgpu renderer: PBR, shadows, post-FX, 2D, text, particles |
| `gens-physics` | Rigid bodies, colliders, character controllers (rapier) |
| `gens-audio` | Decode, mix, spatialize; headless processing for CI |
| `gens-input` | Device abstraction + rebinding action maps |
| `gens-animation` | Skeletal animation, blending, animation state machine |
| `gens-ui` | Runtime UI (egui integration) + text layout (cosmic-text) |
| `gens-ai` | Navigation, pathfinding, behavior trees |
| `gens-scripting` | Scripting layer for game logic |
| `gens-assets` | Import, process, version, and stream assets |
| `gens-networking` | Sessions, replication, transport (quinn/QUIC) |
| `gens-editor` | The visual editor (separate executable) |
| `gens-runtime` | Exported-game runtime shell |

The workspace, dependency graph, and profiles are defined in Phase 1 (see `docs/DECISIONS.md` ADR-0001). The renderer is **offscreen-first** (ADR-0010): every render goes to an engine-owned target with CPU readback, so the full pipeline runs — and is CI-verified on all three operating systems — without any window system or display server (ADR-0011 documents the per-OS GPU guarantees).

## Roadmap

| Phase | Scope | Status | Report |
|---|---|---|---|
| 0 | Risk management & safe development | **Complete** | [`docs/PHASE_0_REPORT.md`](docs/PHASE_0_REPORT.md) |
| 1 | Workspace + core runtime + ECS + math | **Complete** | inline in `docs/WORKLOG.md` (Phase 1 entry) |
| 2 | Renderer (offscreen-first wgpu, GPU CI) | **Complete** | [`docs/PHASE_2_REPORT.md`](docs/PHASE_2_REPORT.md) |
| 3 | Physics + audio + input | **Complete** | [`docs/PHASE_3_REPORT.md`](docs/PHASE_3_REPORT.md) |
| 4 | Animation + UI + AI | Pending | — |
| 5 | Scripting + assets + save | Pending | — |
| 6 | Networking + data assets | Pending | — |
| 7 | Visual editor | Pending | — |
| 8 | Profiling + debugging + build pipeline | Pending | — |
| 9 | Cinematics + world building + plugins | Pending | — |
| 10 | Final testing + benchmarks + validation | Pending | — |

Each phase runs on its own branch (`phase<N>-<slug>`), is verified by CI, and lands on `main` only when its success criteria are met. `pre-phase<N>-safe` tags mark rollback anchors before each phase begins.

## Development Process

This project is developed CI-first with a written risk-management process (Phase 0):

- **Governance docs** — [`docs/DECISIONS.md`](docs/DECISIONS.md) (architecture decision records), [`docs/RISKS.md`](docs/RISKS.md) (risk register), [`docs/ROLLBACK_PLAN.md`](docs/ROLLBACK_PLAN.md), [`docs/EXPERIMENTS.md`](docs/EXPERIMENTS.md), [`docs/WORKLOG.md`](docs/WORKLOG.md).
- **Evidence in-repo** — logs, metrics, and screenshots for every meaningful change live under `docs/evidence/`.
- **Quality gates** — `.github/workflows/ci.yml` builds and tests on Windows, Linux, and macOS, enforces `clippy -D warnings` and `rustfmt`, runs a headless smoke test, and scans every commit for accidentally committed credentials.
- **Local pre-flight** — `scripts/verify.sh` mirrors the CI gates; it must pass before every push.

## Repository Layout

```
gens/
├── .github/workflows/   # CI quality gates
├── docs/                # Governance, phase reports, evidence, design reference
├── scripts/             # Build-safety and verification tooling
├── crates/              # (Phase 1) workspace members gens-core … gens-runtime
├── LICENSE-MIT
└── README.md
```

## Building

Requires Rust stable (1.99+). `rustup` installs everything:

```bash
git clone https://github.com/salim77007j/gens.git
cd gens
cargo build --workspace
cargo test --workspace

# CPU path (no GPU needed): deterministic scene snapshot
cargo run -p gens-runtime --example hello_world -- --frames 120 --out hello_world.png

# GPU path (needs any Vulkan/DX12/Metal adapter; software like lavapipe works):
cargo run -p gens-runtime --example gpu_hello -- --frames 120 --out gpu_hello.png
#   append --allow-no-gpu to skip with an explicit marker instead of failing

# Deterministic physics replay (input -> physics -> ECS, CPU-only):
cargo run -p gens-runtime --example physics_hello -- --steps 600 --out physics_hello.json

# run the GPU integration tests too (requires an adapter):
cargo test -p gens-renderer -- --ignored
```

`scripts/verify.sh` runs the full local gate set (fmt, clippy `-D warnings`, build, tests, size gate, governance checks) — it must pass before every push.

## License

MIT — see [`LICENSE-MIT`](LICENSE-MIT). Dual MIT | Apache-2.0 licensing decision recorded in ADR-0006.
