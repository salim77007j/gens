#!/usr/bin/env bash
# Gens - local verification gate (Phase 0 §0.3 and §0.8).
# Run before EVERY push:  ./scripts/verify.sh
# CI (`.github/workflows/ci.yml`) mirrors these gates; this is the fast local pre-flight.
set -euo pipefail
cd "$(dirname "$0")/.."

say() { printf '==> %s\n' "$*"; }
die() { printf 'VERIFY: FAIL - %s\n' "$*" >&2; exit 1; }

say "[0/11] governance docs present"
for f in docs/WORKLOG.md docs/DECISIONS.md docs/RISKS.md docs/ROLLBACK_PLAN.md docs/EXPERIMENTS.md; do
  [ -f "$f" ] || die "missing $f"
done

say "[1/11] secret scan"
if git grep -nE 'gh[pousr]_[A-Za-z0-9]{20,}' -- . 2>/dev/null; then
  die "possible GitHub credential in tracked files (see docs/ROLLBACK_PLAN.md section 5)"
fi

if [ ! -f Cargo.toml ]; then
  say "no Cargo workspace yet (Phase 0) - Rust gates armed, not active"
  echo "VERIFY: PASS (phase-0 scope)"
  exit 0
fi

say "[2/11] cargo fmt";    cargo fmt --all --check
say "[3/11] cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
say "[4/11] cargo build";  cargo build --workspace --all-targets
say "[5/11] cargo test";   cargo test --workspace
say "[6/11] physics smoke (input -> physics -> ECS replay, CPU-only, deterministic)"
cargo run -q -p gens-runtime --example physics_hello -- --steps 600 --out /tmp/gens_verify_physics.json
echo "physics smoke: $(wc -c < /tmp/gens_verify_physics.json) bytes of trajectory evidence"

say "[7/11] animation+AI+UI smoke (BT -> animation -> ECS -> egui layout replay)"
cargo run -q -p gens-runtime --example animate_hello -- --steps 900 --out /tmp/gens_verify_animate.json
echo "animate smoke: $(wc -c < /tmp/gens_verify_animate.json) bytes of trajectory evidence"

say "[8/11] assets+scripting+save smoke (glTF -> Rhai script -> save/load replay, CPU-only)"
cargo run -q -p gens-runtime --example asset_script_hello -- --steps 1200 --out /tmp/gens_verify_asset_script.json
echo "asset+script smoke: $(wc -c < /tmp/gens_verify_asset_script.json) bytes of save/load replay evidence"

say "[9/11] networking smoke (data document -> QUIC session -> world replication, loopback only)"
cargo run -q -p gens-runtime --example net_hello -- --steps 600 --replicate-at 300 --out /tmp/gens_verify_net.json
echo "net smoke: $(wc -c < /tmp/gens_verify_net.json) bytes of replication evidence"

say "[10/11] GPU smoke (offscreen wgpu render; skips honestly without an adapter)"
# Local machines may lack a Vulkan driver: --allow-no-gpu makes the
# absence explicit instead of failing. CI (Linux+macOS) renders for real;
# see ADR-0011 for the per-OS guarantees.
cargo run -q -p gens-runtime --example gpu_hello -- --frames 60 --out /tmp/gens_verify_gpu.png --allow-no-gpu
if [ -f /tmp/gens_verify_gpu.png ]; then
  echo "gpu smoke: $(wc -c < /tmp/gens_verify_gpu.png) bytes rendered locally"
else
  echo "gpu smoke: GPU-SKIP (no adapter here; CI renders on Linux/macOS)"
fi
say "[11/12] editor example build (the example itself needs an adapter; CI renders it)"
cargo build -q -p gens-editor --examples
echo "editor example: built (GPU render proven in CI on Linux/macOS)"

say "[12/12] binary size gate"
# Calibrated in Phase 2 (ADR-0012): gpu_hello debug binary measured
# 171.3 MiB on Linux x86_64 (wgpu-linked); gate = 2x headroom.
LIMIT_BYTES=$((350 * 1024 * 1024))
cargo build -q -p gens-runtime --example gpu_hello
SIZE=$(wc -c < target/debug/examples/gpu_hello)
echo "gpu_hello debug binary: $SIZE bytes (limit $LIMIT_BYTES)"
if [ "$SIZE" -gt "$LIMIT_BYTES" ]; then
  die "binary size gate failed: $SIZE > $LIMIT_BYTES (see docs/DECISIONS.md ADR-0012)"
fi

echo "VERIFY: PASS"
