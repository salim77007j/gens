# Gens

**A modern, high-performance, cross-platform game engine — built in Rust.**

Gens aims to deliver the capability surface of Unity, Unreal, and Godot with a lighter, safer, more modern core: a data-oriented ECS, a `wgpu`-based renderer targeting Vulkan / DirectX 12 / Metal / WebGPU, deterministic physics, and a professional visual editor — while staying small enough to actually understand and hack on.

[![CI](https://github.com/salim77007j/gens/actions/workflows/ci.yml/badge.svg)](https://github.com/salim77007j/gens/actions/workflows/ci.yml)

> **Status: Phase 0 — Risk Management & Safe Development.**
> The repository currently contains process infrastructure, governance documents, and CI gates. Engine code lands in Phase 1. See the [roadmap](#roadmap), [`docs/PHASE_0_REPORT.md`](docs/PHASE_0_REPORT.md), and the live status of this phase in [`docs/WORKLOG.md`](docs/WORKLOG.md).

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

The workspace, dependency graph, and profiles are defined in Phase 1 (see `docs/DECISIONS.md` ADR-0001).

## Roadmap

| Phase | Scope | Status | Report |
|---|---|---|---|
| 0 | Risk management & safe development | **Complete** | [`docs/PHASE_0_REPORT.md`](docs/PHASE_0_REPORT.md) |
| 1 | Workspace + core runtime + ECS + math | **Complete** | — |
| 2 | Renderer (2D/3D, wgpu) | Pending | — |
| 3 | Physics + audio + input | Pending | — |
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
cargo run -p gens-runtime --example hello_world -- --frames 120 --out hello_world.png
```

`scripts/verify.sh` runs the full local gate set (fmt, clippy `-D warnings`, build, tests, size gate, governance checks) — it must pass before every push.

## License

MIT — see [`LICENSE-MIT`](LICENSE-MIT). Dual MIT | Apache-2.0 licensing is under evaluation for Phase 1 (ADR-0006).
